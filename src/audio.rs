use crate::world::BlockType;
use rand::Rng;
use rodio::{OutputStream, OutputStreamHandle, Sink};

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

fn generate_rising_tone(duration_ms: u32, freq_start: f32, freq_end: f32, volume: f32) -> Vec<i16> {
    let sample_rate = 44100;
    let num_samples = (sample_rate as f32 * duration_ms as f32 / 1000.0) as usize;
    let mut samples = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let progress = i as f32 / num_samples as f32;
        let freq = freq_start + progress * (freq_end - freq_start);
        let envelope = 1.0 - progress;
        let sample = (f32::sin(2.0 * std::f32::consts::PI * freq * t) * volume * envelope * i16::MAX as f32) as i16;
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

    fn random_pitch(&self, min: f32, max: f32) -> f32 {
        rand::thread_rng().gen_range(min..max)
    }

    pub fn play_block_break(&self, block_type: BlockType) {
        if !self.enabled {
            return;
        }

        let (duration, volume) = match block_type {
            BlockType::Stone | BlockType::Cobblestone => (80, 0.3),
            BlockType::Dirt | BlockType::Grass => (60, 0.25),
            BlockType::Sand | BlockType::Gravel => (50, 0.2),
            BlockType::Wood => (70, 0.25),
            BlockType::Leaves => (40, 0.15),
            BlockType::Ice | BlockType::Snow => (60, 0.2),
            _ => (70, 0.25),
        };

        let samples = generate_noise_burst(duration, volume);
        let sample_rate = (44100.0 * self.random_pitch(0.9, 1.1)) as u32;
        self.play_samples(samples, sample_rate);
    }

    pub fn play_block_place(&self, block_type: BlockType) {
        if !self.enabled {
            return;
        }

        let pitch = self.random_pitch(0.95, 1.05);
        let (freq, duration, volume) = match block_type {
            BlockType::Stone | BlockType::Cobblestone => (150.0, 100, 0.3),
            BlockType::Dirt | BlockType::Grass => (120.0, 80, 0.25),
            BlockType::Wood => (200.0, 90, 0.25),
            _ => (140.0, 85, 0.25),
        };

        let samples = generate_sine_wave(freq * pitch, duration, volume);
        self.play_samples(samples, 44100);
    }

    pub fn play_footstep(&self, block_type: BlockType) {
        if !self.enabled {
            return;
        }

        let (duration, volume) = match block_type {
            BlockType::Stone | BlockType::Cobblestone => (30, 0.06),
            BlockType::Dirt | BlockType::Grass => (25, 0.05),
            BlockType::Sand | BlockType::Gravel => (35, 0.04),
            BlockType::Wood => (20, 0.06),
            BlockType::Snow => (40, 0.03),
            _ => (25, 0.05),
        };

        let samples = generate_noise_burst(duration, volume);
        let sample_rate = (44100.0 * self.random_pitch(0.85, 1.15)) as u32;
        self.play_samples(samples, sample_rate);
    }

    pub fn play_splash(&self) {
        if !self.enabled {
            return;
        }

        let noise_samples = generate_noise_burst(200, 0.3);
        let sine_samples = generate_sine_wave(80.0, 200, 0.2);

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

        let samples = generate_rising_tone(100, 200.0, 500.0, 0.2);
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
        if let Ok(sink) = Sink::try_new(&self.handle) {
            let source = rodio::buffer::SamplesBuffer::new(1, sample_rate, samples);
            sink.append(source);
            sink.detach();
        }
    }
}
