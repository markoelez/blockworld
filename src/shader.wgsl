struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) block_type: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) block_type: f32,
}

struct Uniform {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    pad: f32,
}

@group(0) @binding(0)
var<uniform> u_uniform: Uniform;

@group(1) @binding(0)
var t_grass: texture_2d<f32>;
@group(1) @binding(1)
var t_grass_top: texture_2d<f32>;
@group(1) @binding(2)
var t_dirt: texture_2d<f32>;
@group(1) @binding(3)
var t_stone: texture_2d<f32>;
@group(1) @binding(4)
var t_wood: texture_2d<f32>;
@group(1) @binding(5)
var t_leaves: texture_2d<f32>;
@group(1) @binding(7)
var t_water: texture_2d<f32>;
@group(1) @binding(6)
var s_diffuse: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = u_uniform.view_proj * vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.world_position = in.position;
    out.normal = in.normal;
    out.block_type = in.block_type;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Simple face-based lighting without directional light
    var face_brightness = 1.0;
    if abs(in.normal.y) > 0.9 {
        face_brightness = 1.0; // Top/bottom faces - brightest
    } else if abs(in.normal.z) > 0.9 {
        face_brightness = 0.85; // Front/back faces
    } else {
        face_brightness = 0.7; // Left/right faces - darkest
    }
    
    let lighting = 0.8 * face_brightness; // Simple uniform lighting
    
    // Sample all textures to avoid uniform control flow issues
    let grass_color = textureSample(t_grass, s_diffuse, in.tex_coords);
    let grass_top_color = textureSample(t_grass_top, s_diffuse, in.tex_coords);
    let dirt_color = textureSample(t_dirt, s_diffuse, in.tex_coords);
    let stone_color = textureSample(t_stone, s_diffuse, in.tex_coords);
    let wood_color = textureSample(t_wood, s_diffuse, in.tex_coords);
    let leaves_color = textureSample(t_leaves, s_diffuse, in.tex_coords);
    let water_color = textureSample(t_water, s_diffuse, in.tex_coords);
    
    // Select appropriate texture using smooth interpolation
    let bt = floor(in.block_type + 0.5);
    var texture_color: vec4<f32>;
    
    // For grass blocks (bt == 0), use grass_top for top faces, regular grass for sides
    let is_grass = step(bt, 0.5) * step(-0.5, -bt);
    let is_top_face = step(0.9, abs(in.normal.y));
    let grass_final = mix(grass_color, grass_top_color, is_top_face);
    
    // Use smooth step functions to select the right texture
    texture_color = mix(grass_final, dirt_color, step(0.5, bt));
    texture_color = mix(texture_color, stone_color, step(1.5, bt));
    texture_color = mix(texture_color, wood_color, step(2.5, bt));
    texture_color = mix(texture_color, leaves_color, step(3.5, bt));
    texture_color = mix(texture_color, water_color, step(4.5, bt));
    
    if (bt == 6.0) {
        texture_color = vec4<f32>(0.804, 0.498, 0.196, 1.0);
    }
    
    var base_color = texture_color.rgb;
    
    var direction = normalize(in.world_position - u_uniform.camera_pos);
    let height = direction.y;
    let gradient_factor = max(0.0, height);
    let zenith_color = vec3<f32>(0.3098, 0.4392, 0.6392);
    let horizon_color = vec3<f32>(0.8627, 0.9333, 1.0);
    let t = gradient_factor;
    let sky_color = mix(horizon_color, zenith_color, t);
    let distance = length(in.world_position - u_uniform.camera_pos);
    let fog_density = 0.0015;
    let fog_factor = exp(-pow(distance * fog_density, 1.5)); // Softer falloff
    let fog_color = mix(horizon_color, zenith_color * 0.8, pow(gradient_factor, 2.0)); // Bluer fog for distance
    var lit_color = base_color * lighting;
    let final_color = mix(fog_color, lit_color, fog_factor);
    let alpha = select(1.0, 0.7, bt == 5.0);
    return vec4<f32>(final_color, alpha);
}