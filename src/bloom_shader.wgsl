// Bloom extraction and blur shader

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@group(0) @binding(0)
var t_input: texture_2d<f32>;
@group(0) @binding(1)
var s_linear: sampler;

struct BloomUniform {
    threshold: f32,
    soft_threshold: f32,
    blur_direction: vec2<f32>,  // (1,0) for horizontal, (0,1) for vertical
    texel_size: vec2<f32>,
    _padding: vec2<f32>,
}

@group(0) @binding(2)
var<uniform> u_bloom: BloomUniform;

// Full-screen triangle vertices
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.tex_coords = vec2<f32>(x, y);
    return out;
}

// Extract bright parts of the image
fn extract_bright(color: vec3<f32>, threshold: f32, soft_threshold: f32) -> vec3<f32> {
    let brightness = max(max(color.r, color.g), color.b);
    let soft = brightness - threshold + soft_threshold;
    let soft_clamped = clamp(soft, 0.0, 2.0 * soft_threshold);
    let contribution = max(soft_clamped * soft_clamped / (4.0 * soft_threshold + 0.00001), brightness - threshold);
    let contribution_clamped = max(contribution, 0.0) / max(brightness, 0.00001);
    return color * contribution_clamped;
}

// Gaussian blur weights for 9-tap filter
const BLUR_WEIGHTS: array<f32, 5> = array<f32, 5>(
    0.227027,
    0.1945946,
    0.1216216,
    0.054054,
    0.016216
);

@fragment
fn fs_extract(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_input, s_linear, in.tex_coords).rgb;
    let bright = extract_bright(color, u_bloom.threshold, u_bloom.soft_threshold);
    return vec4<f32>(bright, 1.0);
}

@fragment
fn fs_blur(in: VertexOutput) -> @location(0) vec4<f32> {
    var result = textureSample(t_input, s_linear, in.tex_coords).rgb * BLUR_WEIGHTS[0];

    let offset_dir = u_bloom.blur_direction * u_bloom.texel_size;

    // Manually unrolled loop for WGSL compatibility
    let offset1 = offset_dir * 1.0;
    result += textureSample(t_input, s_linear, in.tex_coords + offset1).rgb * BLUR_WEIGHTS[1];
    result += textureSample(t_input, s_linear, in.tex_coords - offset1).rgb * BLUR_WEIGHTS[1];

    let offset2 = offset_dir * 2.0;
    result += textureSample(t_input, s_linear, in.tex_coords + offset2).rgb * BLUR_WEIGHTS[2];
    result += textureSample(t_input, s_linear, in.tex_coords - offset2).rgb * BLUR_WEIGHTS[2];

    let offset3 = offset_dir * 3.0;
    result += textureSample(t_input, s_linear, in.tex_coords + offset3).rgb * BLUR_WEIGHTS[3];
    result += textureSample(t_input, s_linear, in.tex_coords - offset3).rgb * BLUR_WEIGHTS[3];

    let offset4 = offset_dir * 4.0;
    result += textureSample(t_input, s_linear, in.tex_coords + offset4).rgb * BLUR_WEIGHTS[4];
    result += textureSample(t_input, s_linear, in.tex_coords - offset4).rgb * BLUR_WEIGHTS[4];

    return vec4<f32>(result, 1.0);
}
