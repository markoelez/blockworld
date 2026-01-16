struct VertexInput {
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
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
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Pulsing glow effect - time_of_day cycles 0-1 over 10 minutes
    // Multiply by larger value for visible pulse rate
    let pulse = sin(u_uniform.time_of_day * 50.0) * 0.2 + 0.8;

    // White/cyan glow color - HDR value (>1.0) for bloom pickup
    let glow_color = vec3<f32>(0.9, 0.95, 1.0) * 2.5 * pulse;

    return vec4<f32>(glow_color, 1.0);
}