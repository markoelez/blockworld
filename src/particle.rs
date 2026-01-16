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
            // Change weather randomly
            self.weather_type = match rng.gen_range(0..10) {
                0..=5 => WeatherType::Clear,
                6..=8 => WeatherType::Rain,
                _ => WeatherType::Snow,
            };
            self.intensity = if self.weather_type == WeatherType::Clear {
                0.0
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
