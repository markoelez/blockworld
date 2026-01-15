// Volumetric Light Scattering (God Rays) Shader

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct GodRaysUniform {
    light_screen_pos: vec2<f32>,  // Sun position in screen space
    density: f32,
    weight: f32,
    decay: f32,
    exposure: f32,
    num_samples: f32,
    light_intensity: f32,
}

@group(0) @binding(0)
var<uniform> u_godrays: GodRaysUniform;

@group(0) @binding(1)
var t_scene: texture_2d<f32>;

@group(0) @binding(2)
var t_depth: texture_2d<f32>;

@group(0) @binding(3)
var s_sampler: sampler;

// Full-screen triangle vertex shader
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_index & 1u) * 4 - 1);
    let y = f32(i32(vertex_index >> 1u) * 4 - 1);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.tex_coords = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// Occlusion pre-pass - creates light source mask
@fragment
fn fs_occlusion(in: VertexOutput) -> @location(0) vec4<f32> {
    let depth = textureSample(t_depth, s_sampler, in.tex_coords).r;

    // Check if this pixel is sky (not occluded)
    let is_sky = select(0.0, 1.0, depth >= 0.9999);

    // Distance from sun center for radial falloff
    let sun_dist = distance(in.tex_coords, u_godrays.light_screen_pos);
    let sun_glow = exp(-sun_dist * 3.0) * u_godrays.light_intensity;

    // Only show light where sky is visible
    let light = is_sky * sun_glow;

    return vec4<f32>(light, light, light, 1.0);
}

// Main god rays pass - radial blur from light source
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let num_samples = i32(u_godrays.num_samples);

    // Vector from pixel to light source
    let delta_tex_coord = (in.tex_coords - u_godrays.light_screen_pos) * (1.0 / f32(num_samples)) * u_godrays.density;

    var tex_coord = in.tex_coords;
    var illumination_decay = 1.0;
    var color = vec3<f32>(0.0);

    // Sample along ray toward light source
    for (var i = 0; i < num_samples; i = i + 1) {
        tex_coord = tex_coord - delta_tex_coord;

        // Clamp to valid texture coordinates
        let clamped_coord = clamp(tex_coord, vec2<f32>(0.0), vec2<f32>(1.0));

        // Sample the occlusion texture
        let sample_color = textureSample(t_scene, s_sampler, clamped_coord).rgb;

        // Apply decay and accumulate
        color = color + sample_color * illumination_decay * u_godrays.weight;
        illumination_decay = illumination_decay * u_godrays.decay;
    }

    // Apply exposure
    color = color * u_godrays.exposure;

    return vec4<f32>(color, 1.0);
}

// Composite pass - blend god rays with scene
@fragment
fn fs_composite(in: VertexOutput) -> @location(0) vec4<f32> {
    let scene_color = textureSample(t_scene, s_sampler, in.tex_coords).rgb;
    let depth = textureSample(t_depth, s_sampler, in.tex_coords).r;

    // God rays from the normal texture slot (reused)
    let god_rays = textureSample(t_depth, s_sampler, in.tex_coords).rgb;

    // Additive blending
    let final_color = scene_color + god_rays;

    return vec4<f32>(final_color, 1.0);
}
