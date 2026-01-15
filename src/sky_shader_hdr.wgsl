// Enhanced sky shader with atmospheric scattering and day/night cycle

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) brightness: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) brightness: f32,
    @location(1) world_direction: vec3<f32>,
}

struct SkyUniform {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    time_of_day: f32,  // 0.0 = midnight, 0.5 = noon, 1.0 = midnight
    sun_direction: vec3<f32>,
    _pad: f32,
}

@group(0) @binding(0)
var<uniform> u_sky: SkyUniform;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    var clip_pos = vec4<f32>(in.position.xy, 0.999, 1.0);
    out.clip_position = clip_pos;
    var homog_pos = u_sky.inverse_view_proj * clip_pos;
    var world_pos = homog_pos.xyz / homog_pos.w;
    out.world_direction = normalize(world_pos - u_sky.camera_pos);
    out.brightness = in.brightness;
    return out;
}

// Rayleigh scattering coefficient
const RAYLEIGH_COEFF: vec3<f32> = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6);
const MIE_COEFF: f32 = 21e-6;

// Atmospheric scattering approximation
fn atmosphere(ray_dir: vec3<f32>, sun_dir: vec3<f32>, time: f32) -> vec3<f32> {
    let sun_dot = dot(ray_dir, sun_dir);

    // Day/night factor (0 = night, 1 = day)
    let day_factor = smoothstep(-0.1, 0.3, sun_dir.y);

    // Rayleigh scattering (blue sky)
    let rayleigh_phase = 0.75 * (1.0 + sun_dot * sun_dot);

    // Mie scattering (sun glow)
    let g = 0.76;
    let mie_phase = (1.0 - g * g) / (4.0 * 3.14159 * pow(1.0 + g * g - 2.0 * g * sun_dot, 1.5));

    // Base sky color (changes with time of day)
    let zenith_day = vec3<f32>(0.15, 0.35, 0.65);    // Deep blue
    let horizon_day = vec3<f32>(0.6, 0.75, 0.95);    // Light blue
    let zenith_sunset = vec3<f32>(0.1, 0.15, 0.3);   // Deep purple-blue
    let horizon_sunset = vec3<f32>(0.9, 0.4, 0.15);  // Orange
    let zenith_night = vec3<f32>(0.01, 0.01, 0.03);  // Dark blue-black
    let horizon_night = vec3<f32>(0.05, 0.05, 0.1);  // Slightly lighter

    // Sunset factor (peaks when sun is at horizon)
    let sunset_factor = smoothstep(0.0, 0.3, sun_dir.y) * smoothstep(0.5, 0.2, sun_dir.y);

    // Interpolate colors based on time
    var zenith_color = mix(zenith_night, zenith_day, day_factor);
    zenith_color = mix(zenith_color, zenith_sunset, sunset_factor);

    var horizon_color = mix(horizon_night, horizon_day, day_factor);
    horizon_color = mix(horizon_color, horizon_sunset, sunset_factor);

    // Height-based gradient
    let height = max(ray_dir.y, 0.0);
    let gradient = pow(1.0 - height, 3.0);
    var sky_color = mix(zenith_color, horizon_color, gradient);

    // Add sun disc
    let sun_disc = smoothstep(0.9995, 0.9998, sun_dot) * day_factor;
    let sun_color = vec3<f32>(1.5, 1.3, 1.0) * 5.0;  // HDR sun

    // Add sun glow
    let sun_glow = pow(max(sun_dot, 0.0), 64.0) * mie_phase * 0.5 * day_factor;
    let glow_color = vec3<f32>(1.2, 0.9, 0.6);

    // Add moon
    let moon_dir = -sun_dir;
    let moon_dot = dot(ray_dir, moon_dir);
    let moon_disc = smoothstep(0.9995, 0.9998, moon_dot) * (1.0 - day_factor);
    let moon_color = vec3<f32>(0.8, 0.85, 1.0) * 0.5;

    // Combine all elements
    sky_color = sky_color * (1.0 + rayleigh_phase * 0.1);
    sky_color = sky_color + glow_color * sun_glow;
    sky_color = sky_color + sun_color * sun_disc;
    sky_color = sky_color + moon_color * moon_disc;

    // Add stars at night
    let night_factor = 1.0 - day_factor;
    if (night_factor > 0.1 && ray_dir.y > 0.0) {
        let star = stars(ray_dir);
        sky_color = sky_color + star * night_factor * 0.8;
    }

    return sky_color;
}

// Procedural stars
fn stars(dir: vec3<f32>) -> vec3<f32> {
    let scale = 500.0;
    let p = dir * scale;

    // Create star grid
    let grid = floor(p);
    let frac = fract(p);

    // Random star positions within cells
    let rand1 = hash33(grid);
    let rand2 = hash33(grid + 0.5);

    // Star intensity and twinkle
    let star_pos = rand1 - 0.5;
    let dist = length(frac - 0.5 - star_pos * 0.3);
    let star_intensity = smoothstep(0.05, 0.0, dist) * rand2.x;

    // Color variation
    let star_color = mix(vec3<f32>(0.8, 0.9, 1.0), vec3<f32>(1.0, 0.95, 0.8), rand2.y);

    return star_color * star_intensity * step(0.92, rand2.z);
}

// Hash function for stars
fn hash33(p: vec3<f32>) -> vec3<f32> {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q = q + dot(q, q.yxz + 33.33);
    return fract((q.xxy + q.yxx) * q.zyx);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let sky_color = atmosphere(in.world_direction, u_sky.sun_direction, u_sky.time_of_day);
    return vec4<f32>(sky_color, 1.0);
}
