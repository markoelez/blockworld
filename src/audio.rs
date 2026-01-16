use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::collections::HashMap;
use std::io::Cursor;
use rand::Rng;

use crate::world::BlockType;

// Embedded sound data - simple generated tones for now
fn generate_sine_wave(frequency: f32, duration_ms: u32, volume: f32) -> Vec<i16> {
    let sample_rate = 44100;
    let num_samples = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
    let mut samples = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        // Apply envelope for less harsh sound
        let envelope = if i < num_samples / 10 {
            i as f32 / (num_samples / 10) as f32
        } else {
            1.0 - ((i - num_samples / 10) as f32 / (num_samples * 9 / 10) as f32)
        };
        let sample = (f32::sin(2.0 * std::f32::consts::PI * frequency * t) * volume * envelope * i16::MAX as f32) as i16;
        samples.push(sample);
    }
    samples
}

fn generate_noise_burst(duration_ms: u32, volume: f32) -> Vec<i16> {
    let sample_rate = 44100;
    let num_samples = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    let mut rng = rand::thread_rng();

    for i in 0..num_samples {
        let envelope = 1.0 - (i as f32 / num_samples as f32);
        let sample = (rng.gen_range(-1.0..1.0) * volume * envelope * i16::MAX as f32) as i16;
        samples.push(sample);
    }
    samples
}

pub struct AudioManager {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    enabled: bool,
}

impl AudioManager {
    pub fn new() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Some(Self {
                _stream: stream,
                handle,
                enabled: true,
            }),
            Err(_) => {
                eprintln!("Warning: Could not initialize audio system");
                None
            }
        }
    }

    pub fn play_block_break(&self, block_type: BlockType) {
        if !self.enabled {
            return;
        }

        let mut rng = rand::thread_rng();
        let pitch_variation = rng.gen_range(0.9..1.1);

        // Different sounds for different block types
        let (base_freq, duration, volume) = match block_type {
            BlockType::Stone | BlockType::Cobblestone => (200.0, 80, 0.3),
            BlockType::Dirt | BlockType::Grass => (150.0, 60, 0.25),
            BlockType::Sand | BlockType::Gravel => (100.0, 50, 0.2),
            BlockType::Wood => (250.0, 70, 0.25),
            BlockType::Leaves => (300.0, 40, 0.15),
            BlockType::Ice | BlockType::Snow => (400.0, 60, 0.2),
            _ => (180.0, 70, 0.25),
        };

        // Generate and play the sound
        let samples = generate_noise_burst(duration, volume);
        self.play_samples(samples, (44100.0 * pitch_variation) as u32);
    }

    pub fn play_block_place(&self, block_type: BlockType) {
        if !self.enabled {
            return;
        }

        let mut rng = rand::thread_rng();
        let pitch_variation = rng.gen_range(0.95..1.05);

        let (freq, duration, volume) = match block_type {
            BlockType::Stone | BlockType::Cobblestone => (150.0, 100, 0.3),
            BlockType::Dirt | BlockType::Grass => (120.0, 80, 0.25),
            BlockType::Wood => (200.0, 90, 0.25),
            _ => (140.0, 85, 0.25),
        };

        let samples = generate_sine_wave(freq * pitch_variation, duration, volume);
        self.play_samples(samples, 44100);
    }

    pub fn play_footstep(&self, block_type: BlockType) {
        if !self.enabled {
            return;
        }

        let mut rng = rand::thread_rng();
        let pitch_variation = rng.gen_range(0.85..1.15);

        let (duration, volume) = match block_type {
            BlockType::Stone | BlockType::Cobblestone => (30, 0.06),
            BlockType::Dirt | BlockType::Grass => (25, 0.05),
            BlockType::Sand | BlockType::Gravel => (35, 0.04),
            BlockType::Wood => (20, 0.06),
            BlockType::Snow => (40, 0.03),
            _ => (25, 0.05),
        };

        let samples = generate_noise_burst(duration, volume);
        self.play_samples(samples, (44100.0 * pitch_variation) as u32);
    }

    pub fn play_splash(&self) {
        if !self.enabled {
            return;
        }

        // Splash is a combination of noise and low frequency
        let noise_samples = generate_noise_burst(200, 0.3);
        let sine_samples = generate_sine_wave(80.0, 200, 0.2);

        // Combine them
        let samples: Vec<i16> = noise_samples.iter()
            .zip(sine_samples.iter())
            .map(|(n, s)| ((*n as i32 + *s as i32) / 2) as i16)
            .collect();

        self.play_samples(samples, 44100);
    }

    pub fn play_jump(&self) {
        if !self.enabled {
            return;
        }

        // Jump is a quick rising tone
        let sample_rate = 44100;
        let duration_ms = 100;
        let num_samples = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
        let mut samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let progress = i as f32 / num_samples as f32;
            let freq = 200.0 + progress * 300.0; // Rising frequency
            let envelope = 1.0 - progress;
            let sample = (f32::sin(2.0 * std::f32::consts::PI * freq * t) * 0.2 * envelope * i16::MAX as f32) as i16;
            samples.push(sample);
        }

        self.play_samples(samples, 44100);
    }

    pub fn play_land(&self) {
        if !self.enabled {
            return;
        }

        let samples = generate_noise_burst(60, 0.25);
        self.play_samples(samples, 44100);
    }

    fn play_samples(&self, samples: Vec<i16>, sample_rate: u32) {
        // Convert to bytes for rodio
        let bytes: Vec<u8> = samples.iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();

        if let Ok(sink) = Sink::try_new(&self.handle) {
            let source = rodio::buffer::SamplesBuffer::new(1, sample_rate, samples);
            sink.append(source);
            sink.detach(); // Let it play without blocking
        }
    }
}
