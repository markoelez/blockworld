// Post-processing shader for HDR tone mapping and effects

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@group(0) @binding(0)
var t_hdr: texture_2d<f32>;
@group(0) @binding(1)
var t_bloom: texture_2d<f32>;
@group(0) @binding(2)
var s_linear: sampler;

struct PostProcessUniform {
    exposure: f32,
    bloom_intensity: f32,
    saturation: f32,
    contrast: f32,
}

@group(0) @binding(3)
var<uniform> u_post: PostProcessUniform;

// Full-screen triangle vertices
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    // Generate a full-screen triangle
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.tex_coords = vec2<f32>(x, y);
    return out;
}

// ACES filmic tone mapping
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return saturate((x * (a * x + b)) / (x * (c * x + d) + e));
}

// Uncharted 2 tone mapping (alternative filmic look)
fn uncharted2_tonemap_partial(x: vec3<f32>) -> vec3<f32> {
    let A = 0.15;
    let B = 0.50;
    let C = 0.10;
    let D = 0.20;
    let E = 0.02;
    let F = 0.30;
    return ((x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F)) - E / F;
}

fn uncharted2_tonemap(color: vec3<f32>) -> vec3<f32> {
    let W = 11.2;
    let exposure_bias = 2.0;
    let curr = uncharted2_tonemap_partial(color * exposure_bias);
    let white_scale = vec3<f32>(1.0) / uncharted2_tonemap_partial(vec3<f32>(W));
    return curr * white_scale;
}

// Apply contrast
fn apply_contrast(color: vec3<f32>, contrast: f32) -> vec3<f32> {
    return (color - 0.5) * contrast + 0.5;
}

// Apply saturation
fn apply_saturation(color: vec3<f32>, saturation: f32) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(vec3<f32>(luminance), color, saturation);
}

// Vignette effect
fn apply_vignette(color: vec3<f32>, uv: vec2<f32>, strength: f32) -> vec3<f32> {
    let dist = distance(uv, vec2<f32>(0.5));
    let vignette = 1.0 - smoothstep(0.4, 0.8, dist) * strength;
    return color * vignette;
}

// Simple Reinhard tone mapping (more color-preserving)
fn reinhard_tonemap(color: vec3<f32>) -> vec3<f32> {
    return color / (color + vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample HDR scene
    let hdr_color = textureSample(t_hdr, s_linear, in.tex_coords).rgb;

    // Sample bloom
    let bloom_color = textureSample(t_bloom, s_linear, in.tex_coords).rgb;

    // Combine HDR with bloom (subtle bloom)
    var color = hdr_color + bloom_color * u_post.bloom_intensity * 0.5;

    // Apply exposure
    color = color * u_post.exposure;

    // Simple Reinhard tone mapping (preserves colors better than ACES)
    color = reinhard_tonemap(color);

    // Light saturation boost
    color = apply_saturation(color, u_post.saturation);

    // Very subtle vignette
    color = apply_vignette(color, in.tex_coords, 0.15);

    // Note: No gamma correction here - the sRGB swapchain handles it automatically
    return vec4<f32>(color, 1.0);
}
