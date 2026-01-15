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
    @location(1) view_position: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) block_type: f32,
    @location(4) damage: f32,
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
@group(1) @binding(8)
var t_sand: texture_2d<f32>;
@group(1) @binding(9)
var t_snow: texture_2d<f32>;
@group(1) @binding(6)
var s_diffuse: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // Since the arm is in view space, we just need a projection matrix
    // Using FOV of 70 degrees (matching the camera)
    let fovy = 1.2217; // ~70 degrees in radians
    let aspect = 1.77778; // 16:9 aspect ratio approximation
    let near = 0.1;
    let far = 1000.0;
    
    let f = 1.0 / tan(fovy * 0.5);
    let nf = 1.0 / (near - far);
    
    let proj_matrix = mat4x4<f32>(
        f / aspect, 0.0, 0.0, 0.0,
        0.0, f, 0.0, 0.0,
        0.0, 0.0, (far + near) * nf, -1.0,
        0.0, 0.0, 2.0 * far * near * nf, 0.0
    );
    
    out.clip_position = proj_matrix * vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.view_position = in.position;
    out.normal = in.normal;
    out.block_type = in.block_type;
    out.damage = in.damage;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Simple face-based lighting without directional light
    var face_brightness = 1.0;
    if abs(in.normal.y) > 0.9 {
        face_brightness = 1.0; // Top/bottom faces - brightest
    } else if abs(in.normal.z) > 0.9 {
        face_brightness = 0.9; // Front/back faces
    } else {
        face_brightness = 0.75; // Left/right faces - darkest
    }
    
    let lighting = 0.9 * face_brightness;
    
    // Sample all textures to avoid uniform control flow issues
    let grass_color = textureSample(t_grass, s_diffuse, in.tex_coords);
    let grass_top_color = textureSample(t_grass_top, s_diffuse, in.tex_coords);
    let dirt_color = textureSample(t_dirt, s_diffuse, in.tex_coords);
    let stone_color = textureSample(t_stone, s_diffuse, in.tex_coords);
    let wood_color = textureSample(t_wood, s_diffuse, in.tex_coords);
    let leaves_color = textureSample(t_leaves, s_diffuse, in.tex_coords);
    let water_color = textureSample(t_water, s_diffuse, in.tex_coords);
    let sand_color = textureSample(t_sand, s_diffuse, in.tex_coords);
    let snow_color = textureSample(t_snow, s_diffuse, in.tex_coords);
    
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
    
    // Sand texture
    if (bt == 7.0) {
        texture_color = sand_color;
    }
    // Snow texture
    else if (bt == 8.0) {
        texture_color = snow_color;
    }
    
    // Arm color (sandy/skin tone)
    if (bt == 6.0) {
        texture_color = vec4<f32>(0.804, 0.498, 0.196, 1.0);
    }
    // Ice (light blue)
    else if (bt == 9.0) {
        texture_color = vec4<f32>(0.678, 0.847, 0.902, 1.0);
    }
    // Cobblestone (gray)
    else if (bt == 10.0) {
        texture_color = vec4<f32>(0.5, 0.5, 0.5, 1.0);
    }
    // Coal (dark gray/black)
    else if (bt == 11.0) {
        texture_color = vec4<f32>(0.2, 0.2, 0.2, 1.0);
    }
    // Iron (metallic gray)
    else if (bt == 12.0) {
        texture_color = vec4<f32>(0.7, 0.7, 0.7, 1.0);
    }
    // Gold (golden yellow)
    else if (bt == 13.0) {
        texture_color = vec4<f32>(1.0, 0.843, 0.0, 1.0);
    }
    // Diamond (light blue/cyan)
    else if (bt == 14.0) {
        texture_color = vec4<f32>(0.678, 0.847, 0.902, 1.0);
    }
    
    // Crack effect
    let crack_intensity = step(1.0 - in.damage, noise(in.tex_coords * 20.0)) * step(0.01, in.damage);
    let crack_color = vec4<f32>(0.0, 0.0, 1.0, 1.0);
    texture_color = mix(texture_color, crack_color, crack_intensity * 0.7);
    
    var base_color = texture_color.rgb;
    var lit_color = base_color * lighting;
    
    // No fog for held items
    let alpha = select(1.0, 0.7, bt == 5.0);
    return vec4<f32>(lit_color, alpha);
}

fn noise(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}