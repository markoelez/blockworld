// Shadow map depth-only shader
// Renders the scene from the light's perspective to create shadow map

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) block_type: f32,
    @location(4) damage: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
}

struct ShadowUniform {
    light_view_proj: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> u_shadow: ShadowUniform;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = u_shadow.light_view_proj * vec4<f32>(in.position, 1.0);
    return out;
}

// No fragment shader needed - we only care about depth
@fragment
fn fs_main(in: VertexOutput) {
    // Depth is written automatically
}
