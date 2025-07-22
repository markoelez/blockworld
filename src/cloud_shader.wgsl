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
    pad: f32,
}

@group(0) @binding(0)
var<uniform> u_uniform: Uniform;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = u_uniform.view_proj * vec4<f32>(in.position, 1.0);
    out.world_position = in.position;
    out.normal = in.normal;
    out.distance = length(in.position - u_uniform.camera_pos); // Distance from origin for fog effect
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Minecraft-style cloud coloring
    let base_cloud_color = vec3<f32>(1.0, 1.0, 1.0); // White clouds
    let cloud_shadow_color = vec3<f32>(0.7, 0.7, 0.8); // Slightly blue-tinted shadows
    
    // Simple lighting based on normal (top faces are brighter)
    let light_dir = normalize(vec3<f32>(0.3, -0.8, 0.5)); // Sun direction
    let light_intensity = max(dot(in.normal, -light_dir), 0.0);
    let ambient = 0.6; // Bright ambient for clouds
    let lighting = ambient + light_intensity * 0.4;
    
    // Cloud color with lighting
    var cloud_color = mix(cloud_shadow_color, base_cloud_color, lighting);
    
    // Distance-based alpha fade (clouds fade with distance)
    let max_distance = 500.0;
    let alpha_fade = 1.0 - smoothstep(300.0, max_distance, in.distance);
    let cloud_alpha = 0.8 * alpha_fade; // Semi-transparent clouds
    
    return vec4<f32>(cloud_color, cloud_alpha);
}