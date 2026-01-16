// Enhanced terrain shader with shadows, improved lighting, and HDR output

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

// Block textures
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
@group(1) @binding(10)
var t_torch: texture_2d<f32>;
@group(1) @binding(6)
var s_diffuse: sampler;

// Shadow map
@group(2) @binding(0)
var t_shadow: texture_depth_2d;
@group(2) @binding(1)
var s_shadow: sampler_comparison;

// Point lighting
struct PointLight {
    position: vec3<f32>,
    radius: f32,
    color: vec3<f32>,
    intensity: f32,
}

struct LightingUniform {
    point_lights: array<PointLight, 32>,
    num_lights: u32,
}

@group(3) @binding(0)
var<uniform> u_lighting: LightingUniform;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Start with original position
    var animated_pos = in.position;

    // Foliage animation for grass tops (2.0) and leaves (6.0)
    let bt = in.block_type;
    if (bt == 2.0 || bt == 6.0) {
        // Wind sway parameters
        let sway_speed = 2.5;
        let sway_amount = 0.06;

        // Use world position to create varied phase across the world
        let phase = in.position.x * 0.4 + in.position.z * 0.3 + in.position.y * 0.2;

        // Calculate sway based on time
        let time = u_uniform.time_of_day * 600.0;  // Convert back from day fraction to seconds
        let sway = sin(time * sway_speed + phase) * sway_amount;
        let sway2 = cos(time * sway_speed * 0.7 + phase * 1.3) * sway_amount * 0.5;

        // Only sway blocks that are in the upper portion (for grass, top face only)
        // For leaves, sway all of them
        var height_factor = 1.0;
        if (bt == 2.0) {
            // For grass, only sway if this is an upward-facing normal (top face)
            height_factor = max(0.0, in.normal.y);
        }

        animated_pos.x += sway * height_factor;
        animated_pos.z += sway2 * height_factor;
    }

    out.clip_position = u_uniform.view_proj * vec4<f32>(animated_pos, 1.0);
    out.tex_coords = in.tex_coords;
    out.world_position = animated_pos;
    out.normal = in.normal;
    out.block_type = in.block_type;
    out.damage = in.damage;

    // Calculate shadow map coordinates using animated position
    let light_space_pos = u_uniform.light_view_proj * vec4<f32>(animated_pos, 1.0);
    let ndc = light_space_pos.xyz / light_space_pos.w;
    out.shadow_coord = vec3<f32>(
        ndc.x * 0.5 + 0.5,
        ndc.y * -0.5 + 0.5,
        ndc.z
    );

    return out;
}

// PCF shadow sampling with Poisson disk for soft shadows
fn calculate_shadow(shadow_coord: vec3<f32>, normal: vec3<f32>, light_dir: vec3<f32>) -> f32 {
    // Calculate bias based on surface angle
    let NdotL = max(dot(normal, light_dir), 0.0);
    let bias = max(0.003 * (1.0 - NdotL), 0.001);

    let texel_size = 1.0 / 2048.0;
    let spread = 2.5; // Shadow softness

    // Check if in shadow map bounds
    let in_bounds = shadow_coord.x >= 0.0 && shadow_coord.x <= 1.0 &&
                    shadow_coord.y >= 0.0 && shadow_coord.y <= 1.0 &&
                    shadow_coord.z >= 0.0 && shadow_coord.z <= 1.0;

    let sample_coord = clamp(shadow_coord.xy, vec2<f32>(0.001), vec2<f32>(0.999));
    let sample_depth = clamp(shadow_coord.z - bias, 0.0, 1.0);
    let ts = texel_size * spread;

    var shadow = 0.0;

    // 16-tap Poisson disk PCF for smooth soft shadows (manually inlined)
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.94201624, -0.39906216) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.94558609, -0.76890725) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.094184101, -0.92938870) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.34495938, 0.29387760) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.91588581, 0.45771432) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.81544232, -0.87912464) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.38277543, 0.27676845) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.97484398, 0.75648379) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.44323325, -0.97511554) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.53742981, -0.47373420) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.26496911, -0.41893023) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.79197514, 0.19090188) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.24188840, 0.99706507) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(-0.81409955, 0.91437590) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.19984126, 0.78641367) * ts, sample_depth);
    shadow += textureSampleCompare(t_shadow, s_shadow, sample_coord + vec2<f32>(0.14383161, -0.14100790) * ts, sample_depth);

    shadow = shadow / 16.0;

    // Soften shadow edges
    shadow = smoothstep(0.0, 1.0, shadow);

    // If out of bounds, return fully lit
    return select(1.0, shadow, in_bounds);
}

// Fresnel effect
fn fresnel_schlick(cos_theta: f32, f0: f32) -> f32 {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Calculate point light contribution
fn calculate_point_lights(world_pos: vec3<f32>, normal: vec3<f32>, base_color: vec3<f32>) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    let N = normalize(normal);

    for (var i = 0u; i < u_lighting.num_lights; i = i + 1u) {
        let light = u_lighting.point_lights[i];
        let light_vec = light.position - world_pos;
        let dist = length(light_vec);

        if dist < light.radius {
            let L = normalize(light_vec);
            let NdotL = max(dot(N, L), 0.0);

            // Smooth quadratic attenuation
            let attenuation = 1.0 - smoothstep(0.0, light.radius, dist);
            let att_sq = attenuation * attenuation;

            result += base_color * light.color * NdotL * light.intensity * att_sq;
        }
    }

    return result;
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
    let torch_color = textureSample(t_torch, s_diffuse, in.tex_coords);

    // Select texture based on block type
    let bt = floor(in.block_type + 0.5);
    var texture_color: vec4<f32>;
    var roughness = 0.8;
    var metallic = 0.0;

    let is_top_face = step(0.9, in.normal.y);
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
        roughness = 0.1;
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
        metallic = 0.8;
    } else if (bt == 13.0) {
        texture_color = vec4<f32>(1.0, 0.843, 0.0, 1.0);
        roughness = 0.3;
        metallic = 1.0;
    } else if (bt == 14.0) {
        texture_color = vec4<f32>(0.4, 0.85, 0.92, 1.0);
        roughness = 0.2;
        metallic = 0.3;
    } else if (bt == 15.0) {
        // Gravel (gray with variation)
        texture_color = vec4<f32>(0.55, 0.53, 0.5, 1.0);
        roughness = 0.9;
    } else if (bt == 16.0) {
        // Clay (brownish-gray)
        texture_color = vec4<f32>(0.6, 0.55, 0.5, 1.0);
        roughness = 0.7;
    } else if (bt == 17.0) {
        // Villager skin (tan)
        texture_color = vec4<f32>(0.76, 0.60, 0.42, 1.0);
        roughness = 0.6;
    } else if (bt == 18.0) {
        // Villager robe (brown)
        texture_color = vec4<f32>(0.45, 0.30, 0.15, 1.0);
        roughness = 0.7;
    } else if (bt == 19.0) {
        // Villager robe (green)
        texture_color = vec4<f32>(0.2, 0.5, 0.25, 1.0);
        roughness = 0.7;
    } else if (bt == 20.0) {
        // Villager robe (red)
        texture_color = vec4<f32>(0.6, 0.15, 0.15, 1.0);
        roughness = 0.7;
    } else if (bt == 21.0) {
        // Villager robe (blue)
        texture_color = vec4<f32>(0.15, 0.25, 0.55, 1.0);
        roughness = 0.7;
    } else if (bt == 22.0) {
        // Villager robe (purple)
        texture_color = vec4<f32>(0.4, 0.2, 0.5, 1.0);
        roughness = 0.7;
    } else if (bt == 23.0) {
        // Villager robe (white/gray)
        texture_color = vec4<f32>(0.8, 0.78, 0.75, 1.0);
        roughness = 0.7;
    } else if (bt == 24.0) {
        // Torch stick - warm brown/orange color
        texture_color = vec4<f32>(0.6, 0.4, 0.2, 1.0);  // Brown wood color
        roughness = 0.9;
    } else if (bt == 25.0) {
        // Torch flame - bright yellow/orange HDR glow
        texture_color = vec4<f32>(1.0, 0.8, 0.3, 1.0) * 4.0;  // Very bright for bloom
        roughness = 1.0;
    } else if (bt == 26.0) {
        // Chest - darker wood color with some variation
        let wood_grain = noise(in.tex_coords * 8.0) * 0.1;
        texture_color = vec4<f32>(0.45 + wood_grain, 0.3 + wood_grain * 0.5, 0.15, 1.0);
        roughness = 0.8;
    } else if (bt == 27.0) {
        // Pig - pink
        texture_color = vec4<f32>(0.95, 0.70, 0.70, 1.0);
        roughness = 0.7;
    } else if (bt == 28.0) {
        // Cow - brown/tan
        texture_color = vec4<f32>(0.45, 0.30, 0.20, 1.0);
        roughness = 0.7;
    } else if (bt == 29.0) {
        // Sheep - white wool
        texture_color = vec4<f32>(0.92, 0.90, 0.88, 1.0);
        roughness = 0.8;
    }

    // Crack effect - dark cracks that spread as damage increases
    if in.damage > 0.01 {
        // Multiple noise layers for more natural crack pattern
        let crack1 = noise(in.tex_coords * 12.0);
        let crack2 = noise(in.tex_coords * 24.0 + vec2<f32>(0.5, 0.5));
        let crack3 = noise(in.tex_coords * 6.0 + vec2<f32>(0.3, 0.7));
        let combined_noise = (crack1 + crack2 * 0.5 + crack3 * 0.3) / 1.8;

        // Threshold decreases as damage increases, showing more cracks
        let crack_threshold = 1.0 - in.damage;
        let crack_intensity = step(crack_threshold, combined_noise);

        // Dark crack color
        let crack_color = vec4<f32>(0.08, 0.06, 0.04, 1.0);
        texture_color = mix(texture_color, crack_color, crack_intensity * 0.9);
    }

    var base_color = texture_color.rgb;

    // Lighting vectors
    let N = normalize(in.normal);
    let L = normalize(u_uniform.sun_direction);
    let V = normalize(u_uniform.camera_pos - in.world_position);
    let H = normalize(L + V);

    // Day/night factor
    let day_factor = smoothstep(-0.1, 0.3, u_uniform.sun_direction.y);
    let night_factor = 1.0 - day_factor;

    // Diffuse lighting (Lambert with hemisphere)
    let NdotL = max(dot(N, L), 0.0);

    // Moon lighting at night (opposite of sun)
    let moon_dir = -L;
    let NdotMoon = max(dot(N, moon_dir), 0.0);

    // Specular lighting (Blinn-Phong)
    let NdotH = max(dot(N, H), 0.0);
    let shininess = (1.0 - roughness) * 128.0 + 1.0;
    let specular = pow(NdotH, shininess) * (1.0 - roughness) * 0.5;

    // Fresnel
    let VdotH = max(dot(V, H), 0.0);
    let fresnel = fresnel_schlick(VdotH, mix(0.04, 0.8, metallic));

    // Shadow
    var shadow = calculate_shadow(in.shadow_coord, N, L);
    // Only apply shadows during daytime
    shadow = mix(1.0, shadow, day_factor);

    // Ambient occlusion from face direction
    let ao = 0.6 + 0.4 * ((N.y + 1.0) * 0.5);

    // Hemisphere ambient (sky + ground) - brighter at night for visibility
    let night_sky_color = vec3<f32>(0.15, 0.18, 0.25);  // Bluish night ambient
    let day_sky_color = vec3<f32>(0.5, 0.6, 0.8);
    let sky_color_ambient = mix(night_sky_color, day_sky_color, day_factor);
    let sky_ambient = sky_color_ambient * u_uniform.ambient_intensity;
    let ground_ambient = vec3<f32>(0.15, 0.12, 0.1) * u_uniform.ambient_intensity * 0.5;
    let ambient = mix(ground_ambient, sky_ambient, (N.y + 1.0) * 0.5) * ao;

    // Sun lighting
    let sun_intensity = day_factor * 1.8;
    let diffuse_sun = base_color * NdotL * u_uniform.sun_color * sun_intensity * shadow;

    // Moon lighting at night (soft blue-white light)
    let moon_color = vec3<f32>(0.6, 0.65, 0.8);
    let moon_intensity = night_factor * 0.4;  // Moon is dimmer than sun
    let diffuse_moon = base_color * NdotMoon * moon_color * moon_intensity;

    let diffuse = diffuse_sun + diffuse_moon;

    let spec_color = mix(vec3<f32>(1.0), base_color, metallic);
    let spec = spec_color * specular * fresnel * u_uniform.sun_color * sun_intensity * shadow;

    // Point light contribution (torches, etc.)
    let point_lights = calculate_point_lights(in.world_position, N, base_color);

    var lit_color = ambient * base_color + diffuse + spec + point_lights;

    // Enhanced atmospheric fog with aerial perspective
    let distance = length(in.world_position - u_uniform.camera_pos);
    let view_dir = normalize(in.world_position - u_uniform.camera_pos);
    let height_factor = max(view_dir.y, 0.0);

    // Distance-based fog (exponential squared for softer falloff)
    let fog_factor = exp(-pow(distance * u_uniform.fog_density, 1.8));

    // Height-based fog (thicker at low altitudes)
    let world_height = in.world_position.y;
    let height_fog = exp(-max(world_height - 40.0, 0.0) * 0.02);

    // Combined fog
    let combined_fog = fog_factor * (0.7 + 0.3 * height_fog);

    // Rayleigh-inspired fog color (blue tint increases with distance)
    let rayleigh_scatter = vec3<f32>(0.15, 0.25, 0.45) * (1.0 - fog_factor) * day_factor;

    // Base fog colors with time of day
    let zenith_day = vec3<f32>(0.25, 0.45, 0.75);
    let horizon_day = vec3<f32>(0.65, 0.78, 0.92);
    let zenith_night = vec3<f32>(0.03, 0.04, 0.08);
    let horizon_night = vec3<f32>(0.06, 0.07, 0.12);

    // Sunset tint
    let sunset_factor = smoothstep(-0.05, 0.25, u_uniform.sun_direction.y) * smoothstep(0.45, 0.15, u_uniform.sun_direction.y);
    let sunset_horizon = vec3<f32>(0.85, 0.5, 0.3);

    var zenith_fog = mix(zenith_night, zenith_day, day_factor);
    var horizon_fog = mix(horizon_night, horizon_day, day_factor);
    horizon_fog = mix(horizon_fog, sunset_horizon, sunset_factor * 0.6);

    // Sun direction influence on fog color
    let sun_influence = max(dot(view_dir, L), 0.0);
    let sun_tint = vec3<f32>(1.1, 1.0, 0.9) * sun_influence * day_factor * 0.2;

    let fog_color = mix(horizon_fog, zenith_fog, height_factor) + rayleigh_scatter + sun_tint;

    // Apply aerial perspective
    lit_color = mix(fog_color, lit_color, combined_fog);

    // Water rendering with depth-based coloring
    var final_alpha = 1.0;
    let is_water = bt == 5.0;

    if is_water {
        // Water depth passed via damage field
        let normalized_depth = clamp(in.damage / 8.0, 0.0, 1.0);

        // Water color gradient: vibrant blue (surface) -> dark navy (deep)
        let shallow_color = vec3<f32>(0.0, 0.2, 0.5);   // Vibrant dark blue
        let deep_color = vec3<f32>(0.0, 0.02, 0.1);     // Very dark navy
        var water_color = mix(shallow_color, deep_color, normalized_depth);

        // Minimal sky reflection (only at very grazing angles)
        let view_dot_normal = max(dot(V, N), 0.0);
        let water_fresnel = pow(1.0 - view_dot_normal, 5.0);
        let sky_reflect = mix(horizon_fog, zenith_fog, 0.4) * 0.5;  // Dimmed sky
        water_color = mix(water_color, sky_reflect, water_fresnel * 0.15);

        // Subtle sun specular highlight
        let sun_reflect = pow(max(dot(reflect(-L, N), V), 0.0), 256.0);
        water_color += vec3<f32>(1.0, 0.95, 0.8) * sun_reflect * day_factor * 0.5;

        lit_color = mix(fog_color, water_color, combined_fog);

        // Higher alpha so water color is more visible
        final_alpha = 0.7 + normalized_depth * 0.25;
    } else {
        // Check for preview block (damage == -1.0 indicates preview mode)
        if in.damage < -0.5 {
            final_alpha = 0.4;  // Semi-transparent preview
        } else {
            final_alpha = select(1.0, 0.0, in.block_type < 0.0);
        }
    }

    return vec4<f32>(lit_color, final_alpha);
}

fn noise(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}
