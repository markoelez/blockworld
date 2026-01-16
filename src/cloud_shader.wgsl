// Enhanced cloud shader with day/night cycle support

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) distance: f32,
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

    // Drift animation - clouds move with time
    // time_of_day cycles 0-1 every 10 minutes, so multiply for visible movement
    var pos = in.position;
    let drift_speed_x = 80.0;   // Units per full day cycle (main wind direction)
    let drift_speed_z = 25.0;   // Slight diagonal drift

    // Use time_of_day but scale it up for continuous movement
    // This creates a slow, steady drift across the sky
    pos.x += u_uniform.time_of_day * drift_speed_x;
    pos.z += u_uniform.time_of_day * drift_speed_z;

    // Wrap clouds around when they drift too far (seamless looping)
    let wrap_range = 200.0;
    pos.x = pos.x - floor((pos.x + wrap_range) / (wrap_range * 2.0)) * (wrap_range * 2.0);
    pos.z = pos.z - floor((pos.z + wrap_range) / (wrap_range * 2.0)) * (wrap_range * 2.0);

    out.clip_position = u_uniform.view_proj * vec4<f32>(pos, 1.0);
    out.world_position = pos;
    out.normal = in.normal;
    out.distance = length(pos - u_uniform.camera_pos);
    return out;
}

// 3D noise function for volumetric density
fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q = q + dot(q, q.yxz + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn noise3d(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);  // Smooth interpolation

    return mix(
        mix(
            mix(hash3(i + vec3<f32>(0.0, 0.0, 0.0)), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
            mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x),
            u.y
        ),
        mix(
            mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
            mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x),
            u.y
        ),
        u.z
    );
}

// Fractal Brownian Motion for cloud density
fn fbm(p: vec3<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var frequency = 1.0;

    for (var i = 0; i < 4; i = i + 1) {
        value = value + amplitude * noise3d(p * frequency);
        amplitude = amplitude * 0.5;
        frequency = frequency * 2.0;
    }

    return value;
}

// Henyey-Greenstein phase function for light scattering
fn henyeyGreenstein(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * 3.14159 * pow(1.0 + g2 - 2.0 * g * cos_theta, 1.5));
}

// Beer-Lambert attenuation approximation
fn beerLambert(density: f32, distance: f32) -> f32 {
    return exp(-density * distance);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Day/night factor
    let day_factor = smoothstep(-0.1, 0.3, u_uniform.sun_direction.y);

    // Weather factor from fog density (storms darken clouds)
    let weather_intensity = saturate((u_uniform.fog_density - 0.003) / 0.012);

    // Base cloud colors with weather influence
    var base_cloud_day = vec3<f32>(1.0, 1.0, 1.0);
    var shadow_cloud_day = vec3<f32>(0.7, 0.7, 0.8);

    // Storm clouds are darker and more gray
    let storm_base = vec3<f32>(0.45, 0.48, 0.52);
    let storm_shadow = vec3<f32>(0.25, 0.27, 0.32);

    base_cloud_day = mix(base_cloud_day, storm_base, weather_intensity);
    shadow_cloud_day = mix(shadow_cloud_day, storm_shadow, weather_intensity);

    let base_cloud_sunset = vec3<f32>(1.0, 0.8, 0.6);
    let shadow_cloud_sunset = vec3<f32>(0.8, 0.5, 0.4);
    let base_cloud_night = vec3<f32>(0.15, 0.15, 0.2);
    let shadow_cloud_night = vec3<f32>(0.1, 0.1, 0.15);

    // Sunset factor
    let sunset_factor = smoothstep(0.0, 0.3, u_uniform.sun_direction.y) * smoothstep(0.5, 0.2, u_uniform.sun_direction.y);

    // Interpolate cloud colors
    var base_cloud = mix(base_cloud_night, base_cloud_day, day_factor);
    base_cloud = mix(base_cloud, base_cloud_sunset, sunset_factor * (1.0 - weather_intensity));

    var shadow_cloud = mix(shadow_cloud_night, shadow_cloud_day, day_factor);
    shadow_cloud = mix(shadow_cloud, shadow_cloud_sunset, sunset_factor * (1.0 - weather_intensity));

    // Calculate volumetric density using FBM noise
    // Noise animation matches cloud drift for coherent movement
    // Scale matches vertex shader drift (80x, 25z) scaled by noise factor (0.08)
    let noise_drift = vec3<f32>(
        u_uniform.time_of_day * 80.0 * 0.08,  // Matches X drift
        0.0,
        u_uniform.time_of_day * 25.0 * 0.08   // Matches Z drift
    );
    let noise_pos = in.world_position * 0.08 + noise_drift;
    let density = fbm(noise_pos);

    // View and light directions
    let view_dir = normalize(u_uniform.camera_pos - in.world_position);
    let light_dir = normalize(u_uniform.sun_direction);
    let cos_theta = dot(view_dir, light_dir);

    // Light scattering using Henyey-Greenstein
    // Forward scattering for sun-facing edges, back scattering for shadows
    let scatter_forward = henyeyGreenstein(cos_theta, 0.5) * 0.8;
    let scatter_back = henyeyGreenstein(cos_theta, -0.3) * 0.3;
    let scattering = scatter_forward + scatter_back;

    // Self-shadowing approximation using Beer-Lambert
    let shadow_density = density * (1.0 + weather_intensity * 2.0);
    let self_shadow = beerLambert(shadow_density, 2.0);

    // Combine lighting
    let sun_lighting = max(dot(in.normal, light_dir), 0.0);
    let ambient = 0.4 + 0.2 * day_factor - weather_intensity * 0.2;
    let direct_light = sun_lighting * self_shadow * (0.5 + scattering * 0.5) * day_factor;
    let lighting = ambient + direct_light;

    // Cloud color with volumetric lighting
    var cloud_color = mix(shadow_cloud, base_cloud, saturate(lighting));

    // Silver lining effect (bright edges when backlit by sun)
    let rim_light = pow(1.0 - abs(dot(view_dir, in.normal)), 2.0);
    let sun_behind = max(-cos_theta, 0.0);
    let silver_lining = rim_light * sun_behind * day_factor * (1.0 - weather_intensity);
    cloud_color = cloud_color + vec3<f32>(1.0, 0.95, 0.9) * silver_lining * 0.5;

    // Soft edge effect using density-based fade
    let edge_noise = noise3d(in.world_position * 0.2) * 0.5 + 0.5;
    let edge_fade = pow(abs(dot(view_dir, in.normal)), 0.4) * edge_noise;

    // Distance-based alpha fade
    let max_distance = 500.0;
    let alpha_fade = 1.0 - smoothstep(300.0, max_distance, in.distance);

    // Density variation for fluffy appearance
    let density_alpha = smoothstep(0.3, 0.6, density);

    // Combine all alpha factors
    let base_alpha = 0.7 + 0.15 * day_factor + weather_intensity * 0.15;  // Thicker clouds during storms
    let cloud_alpha = base_alpha * alpha_fade * edge_fade * density_alpha;

    return vec4<f32>(cloud_color, cloud_alpha);
}
