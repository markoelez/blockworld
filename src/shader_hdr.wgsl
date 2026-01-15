// Enhanced terrain shader with shadows, PBR-style lighting, and HDR output

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
    @location(3) block_type: f32,
    @location(4) damage: f32,
    @location(5) shadow_coord: vec3<f32>,
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

// Textures
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

// Shadow map
@group(2) @binding(0)
var t_shadow: texture_depth_2d;
@group(2) @binding(1)
var s_shadow: sampler_comparison;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = u_uniform.view_proj * vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.world_position = in.position;
    out.normal = in.normal;
    out.block_type = in.block_type;
    out.damage = in.damage;

    // Calculate shadow map coordinates
    let light_space_pos = u_uniform.light_view_proj * vec4<f32>(in.position, 1.0);
    out.shadow_coord = vec3<f32>(
        light_space_pos.x * 0.5 + 0.5,
        light_space_pos.y * -0.5 + 0.5,  // Flip Y for texture coords
        light_space_pos.z
    );

    return out;
}

// PCF shadow sampling for soft shadows
fn calculate_shadow(shadow_coord: vec3<f32>, bias: f32) -> f32 {
    let texel_size = 1.0 / 2048.0;  // Shadow map resolution
    var shadow = 0.0;

    // 5x5 PCF kernel for very soft shadows
    for (var x = -2; x <= 2; x++) {
        for (var y = -2; y <= 2; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            shadow += textureSampleCompare(
                t_shadow,
                s_shadow,
                shadow_coord.xy + offset,
                shadow_coord.z - bias
            );
        }
    }

    return shadow / 25.0;
}

// Ambient occlusion from face direction (enhanced)
fn calculate_ao(normal: vec3<f32>) -> f32 {
    // Top faces get most light, sides less, bottom least
    let up_factor = (normal.y + 1.0) * 0.5;
    return 0.6 + 0.4 * up_factor;
}

// Fresnel effect for specular
fn fresnel_schlick(cos_theta: f32, f0: f32) -> f32 {
    return f0 + (1.0 - f0) * pow(1.0 - cos_theta, 5.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample all textures
    let grass_color = textureSample(t_grass, s_diffuse, in.tex_coords);
    let grass_top_color = textureSample(t_grass_top, s_diffuse, in.tex_coords);
    let dirt_color = textureSample(t_dirt, s_diffuse, in.tex_coords);
    let stone_color = textureSample(t_stone, s_diffuse, in.tex_coords);
    let wood_color = textureSample(t_wood, s_diffuse, in.tex_coords);
    let leaves_color = textureSample(t_leaves, s_diffuse, in.tex_coords);
    let water_color = textureSample(t_water, s_diffuse, in.tex_coords);
    let sand_color = textureSample(t_sand, s_diffuse, in.tex_coords);
    let snow_color = textureSample(t_snow, s_diffuse, in.tex_coords);

    // Select texture based on block type
    let bt = floor(in.block_type + 0.5);
    var texture_color: vec4<f32>;
    var roughness = 0.8;  // Default roughness
    var metallic = 0.0;   // Default metallic

    let is_top_face = step(0.9, abs(in.normal.y));
    let grass_final = mix(grass_color, grass_top_color, is_top_face);

    texture_color = mix(grass_final, dirt_color, step(0.5, bt));
    texture_color = mix(texture_color, stone_color, step(1.5, bt));
    texture_color = mix(texture_color, wood_color, step(2.5, bt));
    texture_color = mix(texture_color, leaves_color, step(3.5, bt));
    texture_color = mix(texture_color, water_color, step(4.5, bt));

    // Special materials
    if (bt == 7.0) {
        texture_color = sand_color;
        roughness = 0.9;
    } else if (bt == 8.0) {
        texture_color = snow_color;
        roughness = 0.7;
    } else if (bt == 6.0) {
        texture_color = vec4<f32>(0.804, 0.498, 0.196, 1.0);
        roughness = 0.6;
    } else if (bt == 9.0) {
        texture_color = vec4<f32>(0.678, 0.847, 0.902, 1.0);
        roughness = 0.1;  // Ice is smooth
        metallic = 0.1;
    } else if (bt == 10.0) {
        texture_color = vec4<f32>(0.5, 0.5, 0.5, 1.0);
        roughness = 0.85;
    } else if (bt == 11.0) {
        texture_color = vec4<f32>(0.2, 0.2, 0.2, 1.0);
        roughness = 0.7;
    } else if (bt == 12.0) {
        texture_color = vec4<f32>(0.75, 0.75, 0.78, 1.0);
        roughness = 0.4;
        metallic = 0.8;  // Iron is metallic
    } else if (bt == 13.0) {
        texture_color = vec4<f32>(1.0, 0.843, 0.0, 1.0);
        roughness = 0.3;
        metallic = 1.0;  // Gold is very metallic
    } else if (bt == 14.0) {
        texture_color = vec4<f32>(0.4, 0.85, 0.92, 1.0);
        roughness = 0.2;
        metallic = 0.3;  // Diamond has some reflectivity
    }

    // Crack effect
    let crack_intensity = step(1.0 - in.damage, noise(in.tex_coords * 20.0)) * step(0.01, in.damage);
    let crack_color = vec4<f32>(0.1, 0.1, 0.1, 1.0);
    texture_color = mix(texture_color, crack_color, crack_intensity * 0.8);

    var base_color = texture_color.rgb;

    // Lighting calculations
    let N = normalize(in.normal);
    let L = normalize(u_uniform.sun_direction);
    let V = normalize(u_uniform.camera_pos - in.world_position);
    let H = normalize(L + V);

    // Diffuse lighting (Lambert)
    let NdotL = max(dot(N, L), 0.0);

    // Specular lighting (Blinn-Phong with roughness)
    let NdotH = max(dot(N, H), 0.0);
    let shininess = (1.0 - roughness) * 128.0 + 1.0;
    let specular = pow(NdotH, shininess) * (1.0 - roughness) * 0.5;

    // Fresnel for specular
    let VdotH = max(dot(V, H), 0.0);
    let fresnel = fresnel_schlick(VdotH, mix(0.04, 0.8, metallic));

    // Shadow calculation
    let shadow_bias = max(0.005 * (1.0 - NdotL), 0.001);
    var shadow = 1.0;

    // Only apply shadows if in valid shadow map range
    if (in.shadow_coord.x >= 0.0 && in.shadow_coord.x <= 1.0 &&
        in.shadow_coord.y >= 0.0 && in.shadow_coord.y <= 1.0 &&
        in.shadow_coord.z >= 0.0 && in.shadow_coord.z <= 1.0) {
        shadow = calculate_shadow(in.shadow_coord, shadow_bias);
    }

    // Ambient occlusion
    let ao = calculate_ao(N);

    // Day/night factor from sun direction
    let day_factor = smoothstep(-0.1, 0.3, u_uniform.sun_direction.y);

    // Ambient light (sky contribution)
    let sky_ambient = vec3<f32>(0.4, 0.5, 0.7) * u_uniform.ambient_intensity;
    let ground_ambient = vec3<f32>(0.15, 0.1, 0.05) * u_uniform.ambient_intensity * 0.3;
    let ambient = mix(ground_ambient, sky_ambient, (N.y + 1.0) * 0.5) * ao;

    // Combine lighting
    let sun_intensity = day_factor * 2.0;  // HDR sun intensity
    let diffuse = base_color * NdotL * u_uniform.sun_color * sun_intensity * shadow;
    let spec_color = mix(vec3<f32>(1.0), base_color, metallic);
    let spec = spec_color * specular * fresnel * u_uniform.sun_color * sun_intensity * shadow;

    var lit_color = ambient * base_color + diffuse + spec;

    // Atmospheric fog
    let distance = length(in.world_position - u_uniform.camera_pos);
    let fog_factor = exp(-pow(distance * u_uniform.fog_density, 1.5));

    // Fog color based on view direction and time of day
    let view_dir = normalize(in.world_position - u_uniform.camera_pos);
    let height_factor = max(view_dir.y, 0.0);
    let zenith_fog = vec3<f32>(0.3, 0.5, 0.8) * day_factor + vec3<f32>(0.02, 0.02, 0.05) * (1.0 - day_factor);
    let horizon_fog = vec3<f32>(0.7, 0.8, 0.95) * day_factor + vec3<f32>(0.05, 0.05, 0.1) * (1.0 - day_factor);
    let fog_color = mix(horizon_fog, zenith_fog, height_factor);

    lit_color = mix(fog_color, lit_color, fog_factor);

    // Water transparency
    let alpha = select(1.0, 0.7, bt == 5.0);
    let final_alpha = select(alpha, 0.0, in.block_type < 0.0);

    return vec4<f32>(lit_color, final_alpha);
}

fn noise(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}
