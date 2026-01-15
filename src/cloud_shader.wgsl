// Enhanced cloud shader with day/night cycle support

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) distance: f32,
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
}

@group(0) @binding(0)
var<uniform> u_uniform: Uniform;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = u_uniform.view_proj * vec4<f32>(in.position, 1.0);
    out.world_position = in.position;
    out.normal = in.normal;
    out.distance = length(in.position - u_uniform.camera_pos);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Day/night factor
    let day_factor = smoothstep(-0.1, 0.3, u_uniform.sun_direction.y);

    // Base cloud colors (change with time of day)
    let base_cloud_day = vec3<f32>(1.0, 1.0, 1.0);
    let shadow_cloud_day = vec3<f32>(0.7, 0.7, 0.8);
    let base_cloud_sunset = vec3<f32>(1.0, 0.8, 0.6);
    let shadow_cloud_sunset = vec3<f32>(0.8, 0.5, 0.4);
    let base_cloud_night = vec3<f32>(0.15, 0.15, 0.2);
    let shadow_cloud_night = vec3<f32>(0.1, 0.1, 0.15);

    // Sunset factor
    let sunset_factor = smoothstep(0.0, 0.3, u_uniform.sun_direction.y) * smoothstep(0.5, 0.2, u_uniform.sun_direction.y);

    // Interpolate cloud colors
    var base_cloud = mix(base_cloud_night, base_cloud_day, day_factor);
    base_cloud = mix(base_cloud, base_cloud_sunset, sunset_factor);

    var shadow_cloud = mix(shadow_cloud_night, shadow_cloud_day, day_factor);
    shadow_cloud = mix(shadow_cloud, shadow_cloud_sunset, sunset_factor);

    // Lighting based on normal and sun direction
    let light_dir = -u_uniform.sun_direction;
    let light_intensity = max(dot(in.normal, light_dir), 0.0);
    let ambient = 0.5 + 0.2 * day_factor;
    let lighting = ambient + light_intensity * 0.5;

    // Cloud color with lighting
    var cloud_color = mix(shadow_cloud, base_cloud, lighting);

    // Distance-based alpha fade
    let max_distance = 500.0;
    let alpha_fade = 1.0 - smoothstep(300.0, max_distance, in.distance);
    let cloud_alpha = (0.7 + 0.1 * day_factor) * alpha_fade;

    return vec4<f32>(cloud_color, cloud_alpha);
}
