use crate::world::BlockType;
use rand::Rng;
use rodio::{OutputStream, OutputStreamHandle, Sink};

// Music track types
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MusicTrack {
    CalmDay,
    CalmNight,
    Underwater,
}

// Background music manager
pub struct MusicManager {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    music_sink: Option<Sink>,
    current_track: Option<MusicTrack>,
    music_volume: f32,
    track_timer: f32,
    enabled: bool,
}

impl MusicManager {
    pub fn new() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => {
                eprintln!("Music: MusicManager initialized successfully");
                Some(Self {
                    _stream: stream,
                    handle,
                    music_sink: None,
                    current_track: None,
                    music_volume: 0.7,  // Background music volume
                    track_timer: 0.0,
                    enabled: true,
                })
            },
            Err(e) => {
                eprintln!("Music: Failed to initialize MusicManager: {:?}", e);
                None
            }
        }
    }

    pub fn update(&mut self, dt: f32, time_of_day: f32, is_underwater: bool) {
        if !self.enabled {
            return;
        }

        self.track_timer -= dt;

        // Check if we need to change tracks
        let desired_track = Self::select_track(time_of_day, is_underwater);

        // Start or change music
        if self.current_track != Some(desired_track) || self.music_sink.is_none() {
            self.play_track(desired_track);
        } else if self.track_timer <= 0.0 {
            // Restart track for looping (track is about to end)
            if let Some(sink) = &self.music_sink {
                if sink.empty() {
                    self.play_track(desired_track);
                }
            }
        }
    }

    fn select_track(time_of_day: f32, is_underwater: bool) -> MusicTrack {
        if is_underwater {
            MusicTrack::Underwater
        } else if time_of_day > 0.25 && time_of_day < 0.75 {
            MusicTrack::CalmDay
        } else {
            MusicTrack::CalmNight
        }
    }

    fn play_track(&mut self, track: MusicTrack) {
        // Stop current music
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }

        // Generate and play new track
        let samples = Self::generate_ambient_track(track);
        let duration_secs = samples.len() as f32 / 44100.0;

        if let Ok(sink) = Sink::try_new(&self.handle) {
            let source = rodio::buffer::SamplesBuffer::new(1, 44100, samples);
            sink.set_volume(self.music_volume);
            sink.append(source);
            self.music_sink = Some(sink);
            self.current_track = Some(track);
            self.track_timer = duration_secs - 1.0;  // Restart slightly before end
            eprintln!("Music: Playing {:?} track (duration: {:.1}s)", track, duration_secs);
        } else {
            eprintln!("Music: Failed to create sink");
        }
    }

    fn generate_ambient_track(track: MusicTrack) -> Vec<i16> {
        let sample_rate = 44100;
        let duration_secs = 30.0;  // 30 second loops
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        let mut samples = vec![0i16; num_samples];

        match track {
            MusicTrack::CalmDay => {
                // Peaceful C major chord with slow modulation
                // C4 (261.63), E4 (329.63), G4 (392.00)
                let freqs = [130.81, 164.81, 196.00, 261.63];  // C3, E3, G3, C4
                Self::add_layered_tones(&mut samples, &freqs, 0.5, 0.02, sample_rate);
            }
            MusicTrack::CalmNight => {
                // Minor key, lower frequencies
                // A minor: A2, C3, E3
                let freqs = [110.00, 130.81, 164.81, 220.00];  // A2, C3, E3, A3
                Self::add_layered_tones(&mut samples, &freqs, 0.4, 0.015, sample_rate);
            }
            MusicTrack::Underwater => {
                // Muffled, low frequencies with heavy filtering effect
                let freqs = [65.41, 82.41, 98.00];  // C2, E2, G2 (very low)
                Self::add_layered_tones(&mut samples, &freqs, 0.5, 0.025, sample_rate);
                // Apply simple low-pass effect by averaging
                Self::apply_lowpass(&mut samples, 0.3);
            }
        }

        // Apply fade in/out for seamless looping
        let fade_samples = (sample_rate as f32 * 2.0) as usize;  // 2 second fades
        for i in 0..fade_samples.min(num_samples / 2) {
            let factor = i as f32 / fade_samples as f32;
            samples[i] = (samples[i] as f32 * factor) as i16;
            let end_idx = num_samples - 1 - i;
            samples[end_idx] = (samples[end_idx] as f32 * factor) as i16;
        }

        samples
    }

    fn add_layered_tones(samples: &mut [i16], freqs: &[f32], volume: f32, modulation: f32, sample_rate: u32) {
        let num_samples = samples.len();
        let mut rng = rand::thread_rng();

        for (i, sample) in samples.iter_mut().enumerate() {
            let t = i as f32 / sample_rate as f32;

            // Slow volume modulation
            let mod_factor = 1.0 + modulation * (t * 0.1).sin();

            // Sum all frequencies with slight detuning
            let mut value = 0.0f32;
            for (j, &freq) in freqs.iter().enumerate() {
                // Slight random detuning for organic feel
                let detune = 1.0 + (j as f32 * 0.001);
                let phase_offset = j as f32 * 0.5;

                // Very slow attack/decay envelope
                let env_cycle = (t * 0.05 + phase_offset).sin().abs();
                let env = 0.3 + 0.7 * env_cycle;

                value += (2.0 * std::f32::consts::PI * freq * detune * t).sin() * env;
            }

            // Normalize and apply volume
            value = value / freqs.len() as f32 * volume * mod_factor;

            // Add subtle noise for texture
            value += rng.gen_range(-0.01..0.01);

            // Mix with existing
            let current = *sample as f32 / i16::MAX as f32;
            let mixed = (current + value).clamp(-1.0, 1.0);
            *sample = (mixed * i16::MAX as f32) as i16;
        }
    }

    fn apply_lowpass(samples: &mut [i16], factor: f32) {
        // Simple moving average low-pass filter
        let mut prev = samples[0] as f32;
        for sample in samples.iter_mut() {
            let current = *sample as f32;
            let filtered = prev * (1.0 - factor) + current * factor;
            *sample = filtered as i16;
            prev = filtered;
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.music_volume = volume.clamp(0.0, 1.0);
        if let Some(sink) = &self.music_sink {
            sink.set_volume(self.music_volume);
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            if let Some(sink) = self.music_sink.take() {
                sink.stop();
            }
            self.current_track = None;
        }
    }
}

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
