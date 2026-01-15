// Enhanced Water Shader with reflections, refractions, and waves

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) block_type: f32,
    @location(4) damage: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) clip_space: vec4<f32>,
}

struct Uniform {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    time_of_day: f32,
    sun_direction: vec3<f32>,
    ambient_intensity: f32,
    light_view_proj: mat4x4<f32>,
    sun_color: vec3<f32>,
    fog_density: f32,
    time: f32,
    _padding: vec3<f32>,
}

@group(0) @binding(0)
var<uniform> u_uniform: Uniform;

@group(1) @binding(0)
var t_reflection: texture_2d<f32>;

@group(1) @binding(1)
var t_refraction: texture_2d<f32>;

@group(1) @binding(2)
var t_depth: texture_2d<f32>;

@group(1) @binding(3)
var t_normal_map: texture_2d<f32>;

@group(1) @binding(4)
var s_sampler: sampler;

// Wave generation using multiple sine waves
fn wave_height(pos: vec2<f32>, time: f32) -> f32 {
    var height = 0.0;

    // Multiple wave layers for realistic water movement
    height += sin(pos.x * 0.5 + time * 0.8) * 0.1;
    height += sin(pos.y * 0.7 + time * 1.1) * 0.08;
    height += sin((pos.x + pos.y) * 0.3 + time * 0.6) * 0.12;
    height += sin((pos.x - pos.y) * 0.4 + time * 0.9) * 0.06;

    // Small ripples
    height += sin(pos.x * 2.0 + time * 2.0) * 0.02;
    height += sin(pos.y * 2.5 + time * 2.3) * 0.02;

    return height;
}

// Calculate wave normal from height derivatives
fn wave_normal(pos: vec2<f32>, time: f32) -> vec3<f32> {
    let epsilon = 0.1;
    let h_center = wave_height(pos, time);
    let h_right = wave_height(pos + vec2<f32>(epsilon, 0.0), time);
    let h_forward = wave_height(pos + vec2<f32>(0.0, epsilon), time);

    let dx = (h_right - h_center) / epsilon;
    let dz = (h_forward - h_center) / epsilon;

    return normalize(vec3<f32>(-dx, 1.0, -dz));
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply wave displacement
    var pos = in.position;
    let wave_offset = wave_height(pos.xz, u_uniform.time_of_day * 100.0);
    pos.y += wave_offset * 0.3;

    out.clip_position = u_uniform.view_proj * vec4<f32>(pos, 1.0);
    out.tex_coords = in.tex_coords;
    out.world_position = pos;
    out.normal = wave_normal(pos.xz, u_uniform.time_of_day * 100.0);
    out.clip_space = out.clip_position;

    return out;
}

// Fresnel effect - more reflection at grazing angles
fn fresnel(view_dir: vec3<f32>, normal: vec3<f32>) -> f32 {
    let cos_theta = max(dot(view_dir, normal), 0.0);
    let f0 = 0.02; // Water IOR
    return f0 + (1.0 - f0) * pow(1.0 - cos_theta, 5.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let V = normalize(u_uniform.camera_pos - in.world_position);
    let N = normalize(in.normal);
    let L = normalize(u_uniform.sun_direction);

    // Screen-space coordinates for texture sampling
    var screen_coords = in.clip_space.xy / in.clip_space.w;
    screen_coords = screen_coords * 0.5 + 0.5;
    screen_coords.y = 1.0 - screen_coords.y;

    // Distortion based on wave normal
    let distortion = N.xz * 0.02;

    // Sample refraction (underwater view) with distortion
    let refract_coords = screen_coords + distortion;
    let refraction = textureSample(t_refraction, s_sampler, clamp(refract_coords, vec2<f32>(0.0), vec2<f32>(1.0))).rgb;

    // Sample reflection (sky/environment) with distortion
    let reflect_coords = vec2<f32>(screen_coords.x + distortion.x, 1.0 - screen_coords.y + distortion.y);
    let reflection = textureSample(t_reflection, s_sampler, clamp(reflect_coords, vec2<f32>(0.0), vec2<f32>(1.0))).rgb;

    // Fresnel effect
    let fresnel_factor = fresnel(V, N);

    // Water color (deep blue-green)
    let shallow_color = vec3<f32>(0.1, 0.4, 0.5);
    let deep_color = vec3<f32>(0.0, 0.1, 0.2);

    // Depth-based color (would need actual depth comparison)
    let water_depth = 0.5; // Placeholder
    let water_color = mix(shallow_color, deep_color, water_depth);

    // Combine reflection and refraction
    let water_surface = mix(refraction * water_color, reflection, fresnel_factor);

    // Specular highlight from sun
    let H = normalize(L + V);
    let NdotH = max(dot(N, H), 0.0);
    let specular = pow(NdotH, 256.0) * 2.0;

    // Sun reflection on water
    let day_factor = max(u_uniform.sun_direction.y, 0.0);
    let sun_specular = vec3<f32>(1.0, 0.95, 0.8) * specular * day_factor;

    // Foam at edges (simplified)
    let foam = 0.0; // Would need depth comparison for proper foam

    // Final color
    var final_color = water_surface + sun_specular;

    // Fog
    let distance = length(in.world_position - u_uniform.camera_pos);
    let fog_factor = exp(-pow(distance * u_uniform.fog_density, 1.5));
    let fog_color = vec3<f32>(0.5, 0.6, 0.8) * day_factor + vec3<f32>(0.1, 0.1, 0.15) * (1.0 - day_factor);
    final_color = mix(fog_color, final_color, fog_factor);

    return vec4<f32>(final_color, 0.85);
}

// Simple water pass for fallback (no reflection textures)
@fragment
fn fs_simple(in: VertexOutput) -> @location(0) vec4<f32> {
    let V = normalize(u_uniform.camera_pos - in.world_position);
    let N = normalize(in.normal);
    let L = normalize(u_uniform.sun_direction);

    // Water base color
    let deep_color = vec3<f32>(0.05, 0.2, 0.35);
    let shallow_color = vec3<f32>(0.1, 0.4, 0.5);
    let water_color = mix(deep_color, shallow_color, 0.5);

    // Fresnel for fake reflection
    let fresnel_factor = fresnel(V, N);
    let sky_color = vec3<f32>(0.4, 0.6, 0.9);
    let surface_color = mix(water_color, sky_color, fresnel_factor * 0.5);

    // Specular
    let H = normalize(L + V);
    let NdotH = max(dot(N, H), 0.0);
    let specular = pow(NdotH, 128.0) * 1.5;

    let day_factor = max(u_uniform.sun_direction.y + 0.1, 0.0);
    let sun_specular = vec3<f32>(1.0, 0.95, 0.85) * specular * day_factor;

    // Ambient
    let ambient = water_color * u_uniform.ambient_intensity;

    // Diffuse
    let NdotL = max(dot(N, L), 0.0);
    let diffuse = water_color * NdotL * day_factor * 0.3;

    var final_color = ambient + diffuse + sun_specular + surface_color * 0.5;

    // Fog
    let distance = length(in.world_position - u_uniform.camera_pos);
    let fog_factor = exp(-pow(distance * u_uniform.fog_density, 1.5));
    let fog_color = vec3<f32>(0.5, 0.6, 0.8) * day_factor + vec3<f32>(0.08, 0.09, 0.15) * (1.0 - day_factor);
    final_color = mix(fog_color, final_color, fog_factor);

    return vec4<f32>(final_color, 0.75);
}
