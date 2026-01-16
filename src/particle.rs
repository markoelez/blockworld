use cgmath::{Point3, Vector3};
use rand::prelude::*;
use crate::world::BlockType;

const MAX_PARTICLES: usize = 2000;
const GRAVITY: f32 = -15.0;
const SNOW_GRAVITY: f32 = -2.0;

#[derive(Clone, Copy, PartialEq)]
pub enum WeatherType {
    Clear,
    Rain,
    Snow,
    Thunderstorm,
}

// Lightning bolt segment for rendering
#[derive(Clone)]
pub struct LightningSegment {
    pub start: Point3<f32>,
    pub end: Point3<f32>,
    pub brightness: f32,
}

// A single lightning bolt with branching
#[derive(Clone)]
pub struct LightningBolt {
    pub segments: Vec<LightningSegment>,
    pub lifetime: f32,
    pub age: f32,
    pub strike_pos: Point3<f32>,
}

impl LightningBolt {
    pub fn new(start: Point3<f32>, end: Point3<f32>, rng: &mut ThreadRng) -> Self {
        let mut segments = Vec::new();
        Self::generate_bolt(&mut segments, start, end, 1.0, 4, rng);

        Self {
            segments,
            lifetime: rng.gen_range(0.15..0.3),
            age: 0.0,
            strike_pos: end,
        }
    }

    fn generate_bolt(
        segments: &mut Vec<LightningSegment>,
        start: Point3<f32>,
        end: Point3<f32>,
        brightness: f32,
        depth: u32,
        rng: &mut ThreadRng,
    ) {
        if depth == 0 {
            segments.push(LightningSegment { start, end, brightness });
            return;
        }

        // Midpoint displacement
        let mid = Point3::new(
            (start.x + end.x) / 2.0 + rng.gen_range(-5.0..5.0),
            (start.y + end.y) / 2.0 + rng.gen_range(-2.0..2.0),
            (start.z + end.z) / 2.0 + rng.gen_range(-5.0..5.0),
        );

        Self::generate_bolt(segments, start, mid, brightness, depth - 1, rng);
        Self::generate_bolt(segments, mid, end, brightness, depth - 1, rng);

        // Random branching (30% chance)
        if rng.gen_bool(0.3) && depth > 1 {
            let branch_end = Point3::new(
                mid.x + rng.gen_range(-15.0..15.0),
                mid.y - rng.gen_range(10.0..30.0),
                mid.z + rng.gen_range(-15.0..15.0),
            );
            Self::generate_bolt(segments, mid, branch_end, brightness * 0.6, depth - 2, rng);
        }
    }

    pub fn is_alive(&self) -> bool {
        self.age < self.lifetime
    }

    pub fn brightness(&self) -> f32 {
        // Flash effect - bright at start, then fade
        let progress = self.age / self.lifetime;
        if progress < 0.1 {
            1.0
        } else {
            1.0 - ((progress - 0.1) / 0.9).powf(2.0)
        }
    }
}

// Lightning system to manage multiple lightning bolts
pub struct LightningSystem {
    pub bolts: Vec<LightningBolt>,
    pub sky_flash: f32,  // 0.0 to 1.0, for screen flash effect
    pub pending_thunder: Vec<(f32, f32)>,  // (delay_remaining, volume)
    strike_cooldown: f32,
}

impl LightningSystem {
    pub fn new() -> Self {
        Self {
            bolts: Vec::new(),
            sky_flash: 0.0,
            pending_thunder: Vec::new(),
            strike_cooldown: 0.0,
        }
    }

    pub fn update(&mut self, dt: f32, camera_pos: Point3<f32>, weather: &WeatherState, rng: &mut ThreadRng) -> Option<f32> {
        // Update existing bolts
        self.bolts.retain_mut(|bolt| {
            bolt.age += dt;
            bolt.is_alive()
        });

        // Fade sky flash
        self.sky_flash = (self.sky_flash - dt * 5.0).max(0.0);

        // Update pending thunder and check for sounds to play
        let mut thunder_to_play = None;
        self.pending_thunder.retain_mut(|(delay, volume)| {
            *delay -= dt;
            if *delay <= 0.0 {
                thunder_to_play = Some(*volume);
                false
            } else {
                true
            }
        });

        // Update cooldown
        self.strike_cooldown = (self.strike_cooldown - dt).max(0.0);

        // Only spawn new lightning during thunderstorms
        if weather.weather_type == WeatherType::Thunderstorm && self.strike_cooldown <= 0.0 {
            // Random chance for lightning strike
            if rng.gen_bool(0.02 * weather.intensity as f64) {
                self.spawn_lightning(camera_pos, rng);
            }
        }

        thunder_to_play
    }

    pub fn spawn_lightning(&mut self, camera_pos: Point3<f32>, rng: &mut ThreadRng) {
        // Strike within 100 blocks of player
        let offset_x = rng.gen_range(-100.0..100.0);
        let offset_z = rng.gen_range(-100.0..100.0);

        let start = Point3::new(
            camera_pos.x + offset_x,
            camera_pos.y + 80.0,  // Start high in sky
            camera_pos.z + offset_z,
        );

        let end = Point3::new(
            start.x + rng.gen_range(-10.0..10.0),
            camera_pos.y - 20.0,  // Strike ground
            start.z + rng.gen_range(-10.0..10.0),
        );

        let bolt = LightningBolt::new(start, end, rng);

        // Calculate distance for thunder delay (speed of sound ~343 m/s)
        let distance = ((offset_x * offset_x + offset_z * offset_z).sqrt()).abs();
        let thunder_delay = distance / 343.0;  // Delay in seconds
        let thunder_volume = (1.0 - distance / 150.0).max(0.2);  // Louder when closer

        self.pending_thunder.push((thunder_delay, thunder_volume));
        self.sky_flash = 1.0;
        self.strike_cooldown = rng.gen_range(2.0..8.0);  // Cooldown between strikes

        self.bolts.push(bolt);
    }
}

pub struct WeatherState {
    pub weather_type: WeatherType,
    pub intensity: f32,  // 0.0 - 1.0
    pub transition_timer: f32,
}

impl WeatherState {
    pub fn new() -> Self {
        Self {
            weather_type: WeatherType::Clear,
            intensity: 0.0,
            transition_timer: 30.0,  // Start with weather change soon
        }
    }

    pub fn update(&mut self, dt: f32, rng: &mut ThreadRng) {
        self.transition_timer -= dt;
        if self.transition_timer <= 0.0 {
            // Change weather randomly (thunderstorms less common)
            self.weather_type = match rng.gen_range(0..12) {
                0..=5 => WeatherType::Clear,
                6..=8 => WeatherType::Rain,
                9..=10 => WeatherType::Snow,
                _ => WeatherType::Thunderstorm,  // ~8% chance
            };
            self.intensity = if self.weather_type == WeatherType::Clear {
                0.0
            } else if self.weather_type == WeatherType::Thunderstorm {
                rng.gen_range(0.6..1.0)  // Thunderstorms are always intense
            } else {
                rng.gen_range(0.3..1.0)
            };
            // Next weather change in 60-300 seconds
            self.transition_timer = rng.gen_range(60.0..300.0);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ParticleType {
    Normal,
    Rain,
    Snow,
}

#[derive(Clone)]
pub struct Particle {
    pub position: Point3<f32>,
    pub velocity: Vector3<f32>,
    pub color: [f32; 4],  // RGBA
    pub age: f32,
    pub lifetime: f32,
    pub size: f32,
    pub particle_type: ParticleType,
}

impl Particle {
    pub fn new(position: Point3<f32>, velocity: Vector3<f32>, color: [f32; 4], lifetime: f32, size: f32) -> Self {
        Self {
            position,
            velocity,
            color,
            age: 0.0,
            lifetime,
            size,
            particle_type: ParticleType::Normal,
        }
    }

    pub fn new_weather(position: Point3<f32>, velocity: Vector3<f32>, color: [f32; 4], lifetime: f32, size: f32, particle_type: ParticleType) -> Self {
        Self {
            position,
            velocity,
            color,
            age: 0.0,
            lifetime,
            size,
            particle_type,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.age < self.lifetime
    }

    pub fn alpha(&self) -> f32 {
        let life_ratio = 1.0 - (self.age / self.lifetime);
        self.color[3] * life_ratio
    }
}

pub struct ParticleSystem {
    particles: Vec<Particle>,
    rng: ThreadRng,
}

impl ParticleSystem {
    pub fn new() -> Self {
        Self {
            particles: Vec::with_capacity(MAX_PARTICLES),
            rng: thread_rng(),
        }
    }

    pub fn len(&self) -> usize {
        self.particles.len()
    }

    fn random_vector(&mut self, min: f32, max: f32) -> Vector3<f32> {
        Vector3::new(
            self.rng.gen_range(min..max),
            self.rng.gen_range(min..max),
            self.rng.gen_range(min..max),
        )
    }

    fn random_radial_velocity(&mut self, speed_min: f32, speed_max: f32, y_min: f32, y_max: f32) -> Vector3<f32> {
        let angle = self.rng.gen_range(0.0..std::f32::consts::TAU);
        let speed = self.rng.gen_range(speed_min..speed_max);
        Vector3::new(
            angle.cos() * speed,
            self.rng.gen_range(y_min..y_max),
            angle.sin() * speed,
        )
    }

    pub fn spawn_block_break(&mut self, pos: Point3<f32>, block_type: BlockType) {
        let color = get_block_particle_color(block_type);
        let count = 8 + self.rng.gen_range(0..5);

        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                return;
            }

            let offset = self.random_vector(-0.3, 0.3);
            let velocity = Vector3::new(
                self.rng.gen_range(-3.0..3.0),
                self.rng.gen_range(2.0..5.0),
                self.rng.gen_range(-3.0..3.0),
            );

            self.particles.push(Particle::new(
                pos + offset,
                velocity,
                color,
                self.rng.gen_range(0.4..0.8),
                self.rng.gen_range(0.08..0.15),
            ));
        }
    }

    pub fn spawn_water_splash(&mut self, pos: Point3<f32>) {
        let count = 12 + self.rng.gen_range(0..8);
        let water_color = [0.3, 0.5, 0.8, 0.7];

        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                return;
            }

            let velocity = self.random_radial_velocity(1.5, 4.0, 3.0, 6.0);

            self.particles.push(Particle::new(
                pos,
                velocity,
                water_color,
                self.rng.gen_range(0.5..1.0),
                self.rng.gen_range(0.05..0.1),
            ));
        }
    }

    #[allow(dead_code)]
    pub fn spawn_footstep_dust(&mut self, pos: Point3<f32>, block_type: BlockType) {
        if block_type == BlockType::Water || block_type == BlockType::Air {
            return;
        }

        let mut dust_color = get_block_particle_color(block_type);
        dust_color[3] = 0.5; // More transparent for dust
        let count = 3 + self.rng.gen_range(0..3);

        for _ in 0..count {
            if self.particles.len() >= MAX_PARTICLES {
                return;
            }

            let velocity = self.random_radial_velocity(0.3, 1.0, 0.5, 1.5);
            let spawn_pos = Point3::new(pos.x, pos.y - 0.5, pos.z);

            self.particles.push(Particle::new(
                spawn_pos,
                velocity,
                dust_color,
                self.rng.gen_range(0.3..0.6),
                self.rng.gen_range(0.03..0.06),
            ));
        }
    }

    #[allow(dead_code)]
    pub fn spawn_bubble(&mut self, pos: Point3<f32>) {
        if self.particles.len() >= MAX_PARTICLES {
            return;
        }

        let velocity = Vector3::new(
            self.rng.gen_range(-0.3..0.3),
            self.rng.gen_range(1.5..3.0),
            self.rng.gen_range(-0.3..0.3),
        );

        self.particles.push(Particle::new(
            pos,
            velocity,
            [0.8, 0.9, 1.0, 0.6], // bubble color
            self.rng.gen_range(1.0..2.5),
            self.rng.gen_range(0.02..0.05),
        ));
    }

    pub fn spawn_weather(&mut self, camera_pos: Point3<f32>, weather: &WeatherState, dt: f32) {
        if weather.intensity < 0.01 || weather.weather_type == WeatherType::Clear {
            return;
        }

        let spawn_rate = match weather.weather_type {
            WeatherType::Rain => 150.0 * weather.intensity,
            WeatherType::Snow => 80.0 * weather.intensity,
            WeatherType::Thunderstorm => 200.0 * weather.intensity,  // Heavy rain
            WeatherType::Clear => 0.0,
        };

        let spawn_count = (spawn_rate * dt) as usize;
        let spawn_radius = 25.0;
        let spawn_height = 35.0;

        for _ in 0..spawn_count {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }

            let offset_x = self.rng.gen_range(-spawn_radius..spawn_radius);
            let offset_z = self.rng.gen_range(-spawn_radius..spawn_radius);

            let pos = Point3::new(
                camera_pos.x + offset_x,
                camera_pos.y + spawn_height,
                camera_pos.z + offset_z,
            );

            match weather.weather_type {
                WeatherType::Rain => {
                    let velocity = Vector3::new(
                        self.rng.gen_range(-0.5..0.5),
                        -18.0 - self.rng.gen_range(0.0..4.0),
                        self.rng.gen_range(-0.5..0.5),
                    );
                    self.particles.push(Particle::new_weather(
                        pos,
                        velocity,
                        [0.6, 0.7, 0.9, 0.35],  // Blue-ish, transparent
                        2.5,
                        0.015,
                        ParticleType::Rain,
                    ));
                }
                WeatherType::Thunderstorm => {
                    // Heavy rain with wind
                    let velocity = Vector3::new(
                        self.rng.gen_range(-2.0..2.0),  // More wind
                        -20.0 - self.rng.gen_range(0.0..6.0),  // Faster
                        self.rng.gen_range(-2.0..2.0),
                    );
                    self.particles.push(Particle::new_weather(
                        pos,
                        velocity,
                        [0.5, 0.55, 0.7, 0.4],  // Darker rain
                        2.0,
                        0.018,  // Slightly larger drops
                        ParticleType::Rain,
                    ));
                }
                WeatherType::Snow => {
                    let velocity = Vector3::new(
                        self.rng.gen_range(-1.0..1.0),
                        -2.5 - self.rng.gen_range(0.0..1.0),
                        self.rng.gen_range(-1.0..1.0),
                    );
                    self.particles.push(Particle::new_weather(
                        pos,
                        velocity,
                        [0.95, 0.95, 1.0, 0.85],  // White
                        6.0,
                        0.04,
                        ParticleType::Snow,
                    ));
                }
                _ => {}
            }
        }
    }

    /// Spawn flame particles for torches
    pub fn spawn_torch_flames(&mut self, torch_positions: &[Point3<f32>]) {
        for pos in torch_positions {
            // Only spawn 1 particle per torch per call (throttle externally)
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }

            // Position slightly above torch tip
            let spawn_pos = Point3::new(
                pos.x + self.rng.gen_range(-0.05..0.05),
                pos.y + 0.15,
                pos.z + self.rng.gen_range(-0.05..0.05),
            );

            // Upward velocity with slight random drift
            let velocity = Vector3::new(
                self.rng.gen_range(-0.3..0.3),
                self.rng.gen_range(0.5..1.5),  // Rises upward
                self.rng.gen_range(-0.3..0.3),
            );

            // Orange/yellow flame color
            let color = [
                self.rng.gen_range(0.9..1.0),   // Red
                self.rng.gen_range(0.5..0.8),   // Green (makes orange/yellow)
                self.rng.gen_range(0.1..0.3),   // Blue
                0.9,  // Alpha
            ];

            self.particles.push(Particle::new(
                spawn_pos,
                velocity,
                color,
                self.rng.gen_range(0.3..0.8),  // Short lifetime
                self.rng.gen_range(0.02..0.05),  // Small size
            ));
        }
    }

    pub fn update(&mut self, dt: f32) {
        for particle in &mut self.particles {
            // Apply appropriate gravity based on particle type
            let gravity = match particle.particle_type {
                ParticleType::Snow => SNOW_GRAVITY,
                ParticleType::Rain => GRAVITY * 0.5,  // Rain has constant velocity mostly
                ParticleType::Normal => GRAVITY,
            };
            particle.velocity.y += gravity * dt;

            // Add slight drift for snow
            if particle.particle_type == ParticleType::Snow {
                particle.velocity.x += self.rng.gen_range(-0.5..0.5) * dt;
                particle.velocity.z += self.rng.gen_range(-0.5..0.5) * dt;
            }

            particle.position.x += particle.velocity.x * dt;
            particle.position.y += particle.velocity.y * dt;
            particle.position.z += particle.velocity.z * dt;
            particle.age += dt;

            // Ground collision - weather particles just die, others bounce
            if particle.position.y < 0.0 {
                match particle.particle_type {
                    ParticleType::Rain | ParticleType::Snow => {
                        particle.age = particle.lifetime;  // Kill weather particles on ground
                    }
                    ParticleType::Normal => {
                        particle.position.y = 0.0;
                        particle.velocity.y = -particle.velocity.y * 0.3;
                        particle.velocity.x *= 0.8;
                        particle.velocity.z *= 0.8;
                    }
                }
            }
        }

        // Remove dead particles
        self.particles.retain(|p| p.is_alive());
    }

    pub fn get_particles(&self) -> &[Particle] {
        &self.particles
    }

    #[allow(dead_code)]
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }
}

fn get_block_particle_color(block_type: BlockType) -> [f32; 4] {
    match block_type {
        BlockType::Grass => [0.35, 0.55, 0.25, 1.0],
        BlockType::Dirt => [0.55, 0.35, 0.2, 1.0],
        BlockType::Stone => [0.5, 0.5, 0.5, 1.0],
        BlockType::Cobblestone => [0.45, 0.45, 0.45, 1.0],
        BlockType::Sand => [0.85, 0.8, 0.55, 1.0],
        BlockType::Wood => [0.55, 0.4, 0.25, 1.0],
        BlockType::Leaves => [0.2, 0.55, 0.2, 1.0],
        BlockType::Snow => [0.95, 0.95, 0.98, 1.0],
        BlockType::Ice => [0.7, 0.85, 0.95, 1.0],
        BlockType::Gravel => [0.55, 0.5, 0.5, 1.0],
        BlockType::Clay => [0.6, 0.55, 0.5, 1.0],
        BlockType::Coal => [0.2, 0.2, 0.2, 1.0],
        BlockType::Iron => [0.7, 0.65, 0.6, 1.0],
        BlockType::Gold => [0.95, 0.8, 0.2, 1.0],
        BlockType::Diamond => [0.4, 0.9, 0.95, 1.0],
        BlockType::Water => [0.3, 0.5, 0.8, 0.7],
        BlockType::Torch => [0.9, 0.6, 0.2, 1.0],  // Orange particles
        _ => [0.5, 0.5, 0.5, 1.0],
    }
}
