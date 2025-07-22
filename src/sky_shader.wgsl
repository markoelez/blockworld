struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) brightness: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) brightness: f32,
    @location(1) world_position: vec3<f32>,
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
    var clip_pos = vec4<f32>(in.position.xy, 0.999, 1.0);
    out.clip_position = clip_pos;
    var homog_pos = u_uniform.inverse_view_proj * clip_pos;
    var world_pos = homog_pos.xyz / homog_pos.w;
    out.world_position = normalize(world_pos - u_uniform.camera_pos);
    out.brightness = in.brightness;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let height = in.world_position.y;
    let gradient_factor = max(0.0, height);
    let zenith_color = vec3<f32>(0.15, 0.25, 0.45); // Even darker blue #264073
    let horizon_color = vec3<f32>(0.3, 0.6, 0.8); // #4D99CC darker sky blue
    let t = pow(gradient_factor, 0.5);
    let sky_color = mix(horizon_color, zenith_color, t);
    return vec4<f32>(sky_color, 1.0);
}