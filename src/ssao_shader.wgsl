// Screen-Space Ambient Occlusion (SSAO) Shader

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct SSAOUniform {
    projection: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    screen_size: vec2<f32>,
    radius: f32,
    bias: f32,
    intensity: f32,
    _padding: vec3<f32>,
}

@group(0) @binding(0)
var<uniform> u_ssao: SSAOUniform;

@group(0) @binding(1)
var t_depth: texture_2d<f32>;

@group(0) @binding(2)
var t_normal: texture_2d<f32>;

@group(0) @binding(3)
var t_noise: texture_2d<f32>;

@group(0) @binding(4)
var s_sampler: sampler;

// Full-screen triangle vertex shader
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    // Full-screen triangle
    let x = f32(i32(vertex_index & 1u) * 4 - 1);
    let y = f32(i32(vertex_index >> 1u) * 4 - 1);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.tex_coords = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// Reconstruct view-space position from depth
fn get_view_position(tex_coords: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(tex_coords.x * 2.0 - 1.0, (1.0 - tex_coords.y) * 2.0 - 1.0, depth, 1.0);
    let view_pos = u_ssao.inverse_projection * ndc;
    return view_pos.xyz / view_pos.w;
}

// SSAO kernel - hemisphere samples
const KERNEL_SIZE: i32 = 16;

fn get_kernel_sample(index: i32) -> vec3<f32> {
    // Pre-computed hemisphere samples, biased towards center
    let samples = array<vec3<f32>, 16>(
        vec3<f32>(0.0383, 0.0206, 0.0027),
        vec3<f32>(-0.0424, 0.0167, 0.0075),
        vec3<f32>(0.0123, -0.0347, 0.0069),
        vec3<f32>(-0.0119, 0.0489, 0.0130),
        vec3<f32>(0.0756, -0.0286, 0.0117),
        vec3<f32>(-0.0638, -0.0563, 0.0292),
        vec3<f32>(0.0214, 0.0879, 0.0356),
        vec3<f32>(-0.0893, 0.0469, 0.0467),
        vec3<f32>(0.1067, 0.0283, 0.0544),
        vec3<f32>(-0.0470, -0.1125, 0.0619),
        vec3<f32>(0.0325, 0.1320, 0.0783),
        vec3<f32>(-0.1422, 0.0196, 0.0845),
        vec3<f32>(0.1536, -0.0689, 0.0923),
        vec3<f32>(-0.0834, 0.1572, 0.1126),
        vec3<f32>(0.0492, -0.1803, 0.1278),
        vec3<f32>(-0.1894, -0.0913, 0.1467)
    );
    return samples[index];
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let depth = textureSample(t_depth, s_sampler, in.tex_coords).r;

    // Skip sky (far depth)
    if depth >= 0.9999 {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }

    let view_pos = get_view_position(in.tex_coords, depth);
    let normal = textureSample(t_normal, s_sampler, in.tex_coords).xyz * 2.0 - 1.0;

    // Get random rotation vector from noise texture
    let noise_scale = u_ssao.screen_size / 4.0;
    let random_vec = textureSample(t_noise, s_sampler, in.tex_coords * noise_scale).xyz * 2.0 - 1.0;

    // Create TBN matrix for hemisphere orientation
    let tangent = normalize(random_vec - normal * dot(random_vec, normal));
    let bitangent = cross(normal, tangent);
    let tbn = mat3x3<f32>(tangent, bitangent, normal);

    var occlusion = 0.0;

    for (var i = 0; i < KERNEL_SIZE; i = i + 1) {
        // Get sample position in view space
        let sample_offset = tbn * get_kernel_sample(i);
        let sample_pos = view_pos + sample_offset * u_ssao.radius;

        // Project sample to screen space
        let proj = u_ssao.projection * vec4<f32>(sample_pos, 1.0);
        var sample_coords = proj.xy / proj.w;
        sample_coords = sample_coords * 0.5 + 0.5;
        sample_coords.y = 1.0 - sample_coords.y;

        // Get depth at sample position
        let sample_depth = textureSample(t_depth, s_sampler, sample_coords).r;
        let sample_view_pos = get_view_position(sample_coords, sample_depth);

        // Range check and occlusion test
        let range_check = smoothstep(0.0, 1.0, u_ssao.radius / abs(view_pos.z - sample_view_pos.z));
        let is_occluded = select(0.0, 1.0, sample_view_pos.z >= sample_pos.z + u_ssao.bias);
        occlusion = occlusion + is_occluded * range_check;
    }

    occlusion = 1.0 - (occlusion / f32(KERNEL_SIZE)) * u_ssao.intensity;
    occlusion = pow(occlusion, 2.0); // Increase contrast

    return vec4<f32>(occlusion, occlusion, occlusion, 1.0);
}

// Blur pass for SSAO (bilateral blur to preserve edges)
@fragment
fn fs_blur(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel_size = 1.0 / u_ssao.screen_size;
    let center_depth = textureSample(t_depth, s_sampler, in.tex_coords).r;

    var result = 0.0;
    var total_weight = 0.0;

    // 4x4 blur kernel
    for (var x = -2; x <= 2; x = x + 1) {
        for (var y = -2; y <= 2; y = y + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            let sample_coords = in.tex_coords + offset;
            let sample_ao = textureSample(t_normal, s_sampler, sample_coords).r; // Reusing normal texture slot for AO
            let sample_depth = textureSample(t_depth, s_sampler, sample_coords).r;

            // Bilateral weight based on depth difference
            let depth_diff = abs(center_depth - sample_depth);
            let weight = exp(-depth_diff * 100.0);

            result = result + sample_ao * weight;
            total_weight = total_weight + weight;
        }
    }

    return vec4<f32>(result / total_weight, 0.0, 0.0, 1.0);
}
