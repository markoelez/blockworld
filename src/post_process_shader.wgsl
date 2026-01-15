// Post-processing shader for HDR tone mapping, god rays, and cinematic effects

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
    sun_screen_pos: vec2<f32>,  // Sun position in screen space
    god_ray_intensity: f32,
    god_ray_decay: f32,
    screen_size: vec2<f32>,    // Screen dimensions for SSAO
    ssao_intensity: f32,
    ssao_radius: f32,
}

@group(0) @binding(3)
var<uniform> u_post: PostProcessUniform;

@group(0) @binding(4)
var t_depth: texture_depth_2d;
@group(0) @binding(5)
var s_nearest: sampler;

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

// ACES filmic tone mapping (cinematic look)
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return saturate((x * (a * x + b)) / (x * (c * x + d) + e));
}

// AgX-inspired tone mapping (preserves colors in highlights)
fn agx_tonemap(color: vec3<f32>) -> vec3<f32> {
    // Attempt to mimic AgX look
    let agx_mat = mat3x3<f32>(
        vec3<f32>(0.842479, 0.0784336, 0.0792227),
        vec3<f32>(0.0423303, 0.878468, 0.0791988),
        vec3<f32>(0.0423745, 0.0784336, 0.879142)
    );
    var val = agx_mat * color;
    val = max(val, vec3<f32>(0.0));

    // Attempt polynomial approximation of AgX curve
    let v = clamp(val, vec3<f32>(0.0), vec3<f32>(1.0));
    return v * v * (3.0 - 2.0 * v);
}

// Simple Reinhard tone mapping
fn reinhard_tonemap(color: vec3<f32>) -> vec3<f32> {
    return color / (color + vec3<f32>(1.0));
}

// Extended Reinhard for better highlight handling
fn reinhard_extended(color: vec3<f32>, max_white: f32) -> vec3<f32> {
    let numerator = color * (1.0 + (color / vec3<f32>(max_white * max_white)));
    return numerator / (1.0 + color);
}

// Apply contrast with midpoint preservation
fn apply_contrast(color: vec3<f32>, contrast: f32) -> vec3<f32> {
    let midpoint = 0.18; // Middle gray
    return (color - midpoint) * contrast + midpoint;
}

// Apply saturation
fn apply_saturation(color: vec3<f32>, saturation: f32) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(vec3<f32>(luminance), color, saturation);
}

// Vignette effect
fn apply_vignette(color: vec3<f32>, uv: vec2<f32>, strength: f32) -> vec3<f32> {
    let dist = distance(uv, vec2<f32>(0.5));
    let vignette = 1.0 - smoothstep(0.5, 1.0, dist) * strength;
    return color * vignette;
}

// Simplified sun glow effect (god rays disabled - requires depth texture)
fn calculate_sun_glow(uv: vec2<f32>, sun_pos: vec2<f32>) -> vec3<f32> {
    // Simple radial glow from sun position
    let sun_dist = distance(uv, sun_pos);
    let glow = exp(-sun_dist * 3.0) * 0.3;
    return vec3<f32>(glow) * u_post.god_ray_intensity;
}

// Color grading - warm highlights, cool shadows
fn color_grade(color: vec3<f32>) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));

    // Warm tint for highlights
    let warm = vec3<f32>(1.02, 1.0, 0.95);
    // Cool tint for shadows
    let cool = vec3<f32>(0.95, 0.97, 1.02);

    let tint = mix(cool, warm, smoothstep(0.0, 1.0, luminance));
    return color * tint;
}

// Subtle chromatic aberration
fn chromatic_aberration(uv: vec2<f32>, strength: f32) -> vec3<f32> {
    let dir = (uv - vec2<f32>(0.5)) * strength;

    let r = textureSample(t_hdr, s_linear, uv + dir).r;
    let g = textureSample(t_hdr, s_linear, uv).g;
    let b = textureSample(t_hdr, s_linear, uv - dir).b;

    return vec3<f32>(r, g, b);
}

// Simple hash function for noise
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// Load depth at pixel coordinates (integer)
fn load_depth(pixel: vec2<i32>) -> f32 {
    let dims = textureDimensions(t_depth);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(t_depth, clamped, 0);
}

// Simplified SSAO - samples depth at neighboring pixels
fn calculate_ssao(uv: vec2<f32>) -> f32 {
    let pixel = vec2<i32>(uv * u_post.screen_size);
    let center_depth = load_depth(pixel);

    // Skip sky (far depth)
    if center_depth >= 0.9999 {
        return 1.0;
    }

    let radius_pixels = u_post.ssao_radius * 8.0;  // Radius in pixels

    var occlusion = 0.0;

    // Fixed 8-sample pattern (manually unrolled for uniform control flow)
    let noise = hash(uv * u_post.screen_size);
    let angle_offset = noise * 6.28318;

    // Sample 0
    var angle = 0.0 * (6.28318 / 8.0) + angle_offset;
    var dist = 1.0 / 8.0 * radius_pixels;
    var offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    var sample_depth = load_depth(pixel + offset);
    var depth_diff = center_depth - sample_depth;
    var range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    var is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    // Sample 1
    angle = 1.0 * (6.28318 / 8.0) + angle_offset;
    dist = 2.0 / 8.0 * radius_pixels;
    offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    sample_depth = load_depth(pixel + offset);
    depth_diff = center_depth - sample_depth;
    range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    // Sample 2
    angle = 2.0 * (6.28318 / 8.0) + angle_offset;
    dist = 3.0 / 8.0 * radius_pixels;
    offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    sample_depth = load_depth(pixel + offset);
    depth_diff = center_depth - sample_depth;
    range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    // Sample 3
    angle = 3.0 * (6.28318 / 8.0) + angle_offset;
    dist = 4.0 / 8.0 * radius_pixels;
    offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    sample_depth = load_depth(pixel + offset);
    depth_diff = center_depth - sample_depth;
    range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    // Sample 4
    angle = 4.0 * (6.28318 / 8.0) + angle_offset;
    dist = 5.0 / 8.0 * radius_pixels;
    offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    sample_depth = load_depth(pixel + offset);
    depth_diff = center_depth - sample_depth;
    range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    // Sample 5
    angle = 5.0 * (6.28318 / 8.0) + angle_offset;
    dist = 6.0 / 8.0 * radius_pixels;
    offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    sample_depth = load_depth(pixel + offset);
    depth_diff = center_depth - sample_depth;
    range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    // Sample 6
    angle = 6.0 * (6.28318 / 8.0) + angle_offset;
    dist = 7.0 / 8.0 * radius_pixels;
    offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    sample_depth = load_depth(pixel + offset);
    depth_diff = center_depth - sample_depth;
    range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    // Sample 7
    angle = 7.0 * (6.28318 / 8.0) + angle_offset;
    dist = 8.0 / 8.0 * radius_pixels;
    offset = vec2<i32>(vec2<f32>(cos(angle), sin(angle)) * dist);
    sample_depth = load_depth(pixel + offset);
    depth_diff = center_depth - sample_depth;
    range_check = smoothstep(0.0, 1.0, 0.01 / max(abs(depth_diff), 0.0001));
    is_occluded = step(0.0001, depth_diff) * step(depth_diff, 0.02);
    occlusion = occlusion + is_occluded * range_check;

    occlusion = 1.0 - (occlusion / 8.0) * u_post.ssao_intensity;

    // Smooth the result
    return clamp(occlusion, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample HDR scene
    var hdr_color = textureSample(t_hdr, s_linear, in.tex_coords).rgb;

    // Calculate SSAO
    let ssao = calculate_ssao(in.tex_coords);

    // Apply SSAO to scene (darken occluded areas)
    hdr_color = hdr_color * ssao;

    // Sample bloom
    let bloom_color = textureSample(t_bloom, s_linear, in.tex_coords).rgb;

    // Calculate sun glow if sun is on screen
    var sun_glow = vec3<f32>(0.0);
    if u_post.god_ray_intensity > 0.01 {
        let sun_in_view = u_post.sun_screen_pos.x >= -0.5 && u_post.sun_screen_pos.x <= 1.5 &&
                          u_post.sun_screen_pos.y >= -0.5 && u_post.sun_screen_pos.y <= 1.5;
        if sun_in_view {
            sun_glow = calculate_sun_glow(in.tex_coords, u_post.sun_screen_pos);
            // Tint with warm sun color
            sun_glow = sun_glow * vec3<f32>(1.0, 0.9, 0.7);
        }
    }

    // Combine HDR with bloom and sun glow
    var color = hdr_color + bloom_color * u_post.bloom_intensity * 0.6 + sun_glow;

    // Apply exposure
    color = color * u_post.exposure;

    // Tone mapping - using extended Reinhard for natural look
    color = reinhard_extended(color, 4.0);

    // Color grading
    color = color_grade(color);

    // Saturation adjustment
    color = apply_saturation(color, u_post.saturation);

    // Subtle contrast boost
    color = apply_contrast(color, u_post.contrast);

    // Subtle vignette
    color = apply_vignette(color, in.tex_coords, 0.2);

    // Clamp to valid range
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));

    return vec4<f32>(color, 1.0);
}
