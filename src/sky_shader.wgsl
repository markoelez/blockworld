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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let ray_dir = in.world_direction;
    let sun_dir = u_uniform.sun_direction;
    let sun_dot = dot(ray_dir, sun_dir);

    // Day/night factor (0 = night, 1 = day)
    let day_factor = smoothstep(-0.1, 0.3, sun_dir.y);

    // Mie scattering for sun glow
    let g = 0.76;
    let mie_phase = (1.0 - g * g) / (4.0 * 3.14159 * pow(1.0 + g * g - 2.0 * g * sun_dot, 1.5));

    // Base sky colors
    let zenith_day = vec3<f32>(0.15, 0.35, 0.65);
    let horizon_day = vec3<f32>(0.6, 0.75, 0.95);
    let zenith_sunset = vec3<f32>(0.1, 0.15, 0.3);
    let horizon_sunset = vec3<f32>(0.9, 0.4, 0.15);
    let zenith_night = vec3<f32>(0.01, 0.01, 0.03);
    let horizon_night = vec3<f32>(0.05, 0.05, 0.1);

    // Sunset factor
    let sunset_factor = smoothstep(0.0, 0.3, sun_dir.y) * smoothstep(0.5, 0.2, sun_dir.y);

    // Interpolate colors
    var zenith_color = mix(zenith_night, zenith_day, day_factor);
    zenith_color = mix(zenith_color, zenith_sunset, sunset_factor);

    var horizon_color = mix(horizon_night, horizon_day, day_factor);
    horizon_color = mix(horizon_color, horizon_sunset, sunset_factor);

    // Height-based gradient
    let height = max(ray_dir.y, 0.0);
    let gradient = pow(1.0 - height, 3.0);
    var sky_color = mix(zenith_color, horizon_color, gradient);

    // Sun disc
    let sun_disc = smoothstep(0.9995, 0.9998, sun_dot) * day_factor;
    let sun_color = vec3<f32>(1.5, 1.3, 1.0) * 3.0;

    // Sun glow
    let sun_glow = pow(max(sun_dot, 0.0), 64.0) * mie_phase * 0.3 * day_factor;
    let glow_color = vec3<f32>(1.2, 0.9, 0.6);

    // Moon
    let moon_dir = -sun_dir;
    let moon_dot = dot(ray_dir, moon_dir);
    let moon_disc = smoothstep(0.9995, 0.9998, moon_dot) * (1.0 - day_factor);
    let moon_color = vec3<f32>(0.8, 0.85, 1.0) * 0.5;

    // Combine elements
    sky_color = sky_color + glow_color * sun_glow;
    sky_color = sky_color + sun_color * sun_disc;
    sky_color = sky_color + moon_color * moon_disc;

    // Stars at night
    let night_factor = 1.0 - day_factor;
    if (night_factor > 0.1 && ray_dir.y > 0.0) {
        let star = stars(ray_dir);
        sky_color = sky_color + star * night_factor * 0.8;
    }

    return vec4<f32>(sky_color, 1.0);
}
