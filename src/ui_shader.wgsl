struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) use_texture: f32, // 0.0 = use color, 1.0+ = use texture (block type)
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) use_texture: f32,
}

@group(0) @binding(0)
var t_grass: texture_2d<f32>;
@group(0) @binding(1)
var t_grass_top: texture_2d<f32>;
@group(0) @binding(2)
var t_dirt: texture_2d<f32>;
@group(0) @binding(3)
var t_stone: texture_2d<f32>;
@group(0) @binding(4)
var t_wood: texture_2d<f32>;
@group(0) @binding(5)
var t_leaves: texture_2d<f32>;
@group(0) @binding(7)
var t_water: texture_2d<f32>;
@group(0) @binding(6)
var s_diffuse: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.use_texture = in.use_texture;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample all textures unconditionally to maintain uniform control flow
    let grass_color = textureSample(t_grass_top, s_diffuse, in.tex_coords);
    let dirt_color = textureSample(t_dirt, s_diffuse, in.tex_coords);
    let stone_color = textureSample(t_stone, s_diffuse, in.tex_coords);
    let wood_color = textureSample(t_wood, s_diffuse, in.tex_coords);
    let leaves_color = textureSample(t_leaves, s_diffuse, in.tex_coords);
    let water_color = textureSample(t_water, s_diffuse, in.tex_coords);
    
    let block_type = floor(in.use_texture);
    
    // Select texture using step functions (maintains uniform control flow)
    var texture_color = grass_color;
    texture_color = mix(texture_color, dirt_color, step(1.5, block_type));
    texture_color = mix(texture_color, stone_color, step(2.5, block_type));
    texture_color = mix(texture_color, wood_color, step(3.5, block_type));
    texture_color = mix(texture_color, leaves_color, step(4.5, block_type));
    texture_color = mix(texture_color, water_color, step(5.5, block_type));
    
    // Mix between color and texture based on use_texture flag
    let use_tex = step(0.5, in.use_texture);
    return mix(in.color, texture_color * in.color, use_tex);
}