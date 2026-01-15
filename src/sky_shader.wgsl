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
    var clip_pos = vec4<f32>(in.position.xy, 0.999, 1.0);
    out.clip_position = clip_pos;
    var homog_pos = u_uniform.inverse_view_proj * clip_pos;
    var world_pos = homog_pos.xyz / homog_pos.w;
    out.world_direction = normalize(world_pos - u_uniform.camera_pos);
    out.brightness = in.brightness;
    return out;
}

// Hash function for stars
fn hash33(p: vec3<f32>) -> vec3<f32> {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q = q + dot(q, q.yxz + 33.33);
    return fract((q.xxy + q.yxx) * q.zyx);
}

// Procedural stars
fn stars(dir: vec3<f32>) -> vec3<f32> {
    let scale = 500.0;
    let p = dir * scale;
    let grid = floor(p);
    let frac_p = fract(p);
    let rand1 = hash33(grid);
    let rand2 = hash33(grid + 0.5);
    let star_pos = rand1 - 0.5;
    let dist = length(frac_p - 0.5 - star_pos * 0.3);
    let star_intensity = smoothstep(0.05, 0.0, dist) * rand2.x;
    let star_color = mix(vec3<f32>(0.8, 0.9, 1.0), vec3<f32>(1.0, 0.95, 0.8), rand2.y);
    return star_color * star_intensity * step(0.92, rand2.z);
}

// Rayleigh scattering phase function
fn rayleigh_phase(cos_theta: f32) -> f32 {
    return (3.0 / (16.0 * 3.14159)) * (1.0 + cos_theta * cos_theta);
}

// Mie scattering phase function (Henyey-Greenstein)
fn mie_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * 3.14159 * pow(1.0 + g2 - 2.0 * g * cos_theta, 1.5));
}

// Approximate atmospheric scattering
fn atmosphere(ray_dir: vec3<f32>, sun_dir: vec3<f32>, day_factor: f32) -> vec3<f32> {
    let cos_theta = dot(ray_dir, sun_dir);

    // Rayleigh scattering coefficients (wavelength dependent - blue scatters more)
    let rayleigh_coeff = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6) * 1e6;

    // Mie scattering coefficient (wavelength independent)
    let mie_coeff = vec3<f32>(21e-6) * 1e6;

    // Calculate optical depth based on view angle
    let height = max(ray_dir.y, 0.001);
    let optical_depth = 1.0 / (height + 0.1);

    // Rayleigh and Mie contributions
    let rayleigh = rayleigh_coeff * rayleigh_phase(cos_theta) * optical_depth;
    let mie = mie_coeff * mie_phase(cos_theta, 0.76) * optical_depth * 0.5;

    // Sun intensity based on altitude (more red at horizon)
    let sun_altitude = max(sun_dir.y, 0.0);
    let sun_optical_depth = 1.0 / (sun_altitude + 0.05);
    let sun_extinction = exp(-rayleigh_coeff * sun_optical_depth * 0.1);
    let sun_intensity = vec3<f32>(1.2, 1.0, 0.9) * sun_extinction;

    // Combine scattering
    let inscatter = (rayleigh + mie) * sun_intensity * day_factor;

    return inscatter;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let ray_dir = in.world_direction;
    let sun_dir = u_uniform.sun_direction;
    let sun_dot = dot(ray_dir, sun_dir);

    // Day/night factor (0 = night, 1 = day)
    let day_factor = smoothstep(-0.1, 0.3, sun_dir.y);

    // Atmospheric scattering
    let scatter = atmosphere(ray_dir, sun_dir, day_factor);

    // Base sky colors with scattering
    let zenith_day = vec3<f32>(0.1, 0.3, 0.6) + scatter * 0.3;
    let horizon_day = vec3<f32>(0.5, 0.7, 0.9) + scatter;
    let zenith_sunset = vec3<f32>(0.1, 0.12, 0.25);
    let horizon_sunset = vec3<f32>(0.95, 0.35, 0.1);
    let zenith_night = vec3<f32>(0.008, 0.01, 0.025);
    let horizon_night = vec3<f32>(0.04, 0.045, 0.08);

    // Sunset factor (more pronounced)
    let sunset_factor = smoothstep(-0.05, 0.25, sun_dir.y) * smoothstep(0.45, 0.15, sun_dir.y);

    // Interpolate colors
    var zenith_color = mix(zenith_night, zenith_day, day_factor);
    zenith_color = mix(zenith_color, zenith_sunset, sunset_factor);

    var horizon_color = mix(horizon_night, horizon_day, day_factor);
    horizon_color = mix(horizon_color, horizon_sunset, sunset_factor);

    // Height-based gradient with better curve
    let height = max(ray_dir.y, 0.0);
    let gradient = pow(1.0 - height, 2.5);
    var sky_color = mix(zenith_color, horizon_color, gradient);

    // Sun disc (brighter, with corona)
    let sun_disc = smoothstep(0.9994, 0.9998, sun_dot) * day_factor;
    let sun_corona = smoothstep(0.990, 0.9994, sun_dot) * 0.3 * day_factor;
    let sun_color = vec3<f32>(1.6, 1.35, 1.0);

    // Sun glow with Mie scattering
    let mie_g = 0.76;
    let sun_glow = pow(max(sun_dot, 0.0), 32.0) * mie_phase(sun_dot, mie_g) * 0.4 * day_factor;
    let glow_color = mix(vec3<f32>(1.2, 0.95, 0.7), vec3<f32>(1.0, 0.5, 0.2), sunset_factor);

    // Moon with glow
    let moon_dir = -sun_dir;
    let moon_dot = dot(ray_dir, moon_dir);
    let moon_disc = smoothstep(0.9994, 0.9998, moon_dot) * (1.0 - day_factor);
    let moon_glow = smoothstep(0.99, 0.9994, moon_dot) * 0.15 * (1.0 - day_factor);
    let moon_color = vec3<f32>(0.85, 0.9, 1.0);

    // Combine elements
    sky_color = sky_color + glow_color * sun_glow;
    sky_color = sky_color + sun_color * (sun_disc * 3.0 + sun_corona);
    sky_color = sky_color + moon_color * (moon_disc * 0.6 + moon_glow);

    // Stars at night (more visible)
    let night_factor = 1.0 - day_factor;
    if (night_factor > 0.05 && ray_dir.y > -0.1) {
        let star = stars(ray_dir);
        sky_color = sky_color + star * night_factor * 1.2;
    }

    return vec4<f32>(sky_color, 1.0);
}
