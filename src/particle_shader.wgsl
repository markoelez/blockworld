// Particle billboard shader

struct ParticleVertexInput {
    @location(0) position: vec3<f32>,     // World position of particle center
    @location(1) offset: vec2<f32>,       // Quad corner offset (-1 to 1)
    @location(2) color: vec4<f32>,        // RGBA color with alpha
    @location(3) size: f32,               // Particle size
}

struct ParticleVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
}

struct ParticleUniform {
    view_proj: mat4x4<f32>,
    camera_right: vec3<f32>,
    _pad1: f32,
    camera_up: vec3<f32>,
    _pad2: f32,
}

@group(0) @binding(0)
var<uniform> u_particle: ParticleUniform;

@vertex
fn vs_main(in: ParticleVertexInput) -> ParticleVertexOutput {
    var out: ParticleVertexOutput;

    // Billboard: offset particle position by camera-aligned vectors
    let world_pos = in.position
        + u_particle.camera_right * in.offset.x * in.size
        + u_particle.camera_up * in.offset.y * in.size;

    out.clip_position = u_particle.view_proj * vec4<f32>(world_pos, 1.0);
    out.color = in.color;
    out.uv = in.offset * 0.5 + 0.5; // Convert -1..1 to 0..1

    return out;
}

@fragment
fn fs_main(in: ParticleVertexOutput) -> @location(0) vec4<f32> {
    // Soft circular particle shape
    let dist = length(in.uv - vec2<f32>(0.5, 0.5)) * 2.0;
    let alpha = 1.0 - smoothstep(0.5, 1.0, dist);

    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
