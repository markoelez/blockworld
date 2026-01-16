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
                    music_volume: 0.15,  // Background music volume (subtle ambient)
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
        let duration_secs = 60.0;  // 60 second loops for more variety
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        let mut samples = vec![0i16; num_samples];

        match track {
            MusicTrack::CalmDay => {
                // Soft, ethereal pad - C major 7th spread across octaves
                // Very quiet, slowly evolving tones
                Self::add_ambient_pad(&mut samples, &[
                    (130.81, 0.0),   // C3
                    (196.00, 5.0),   // G3 (offset by 5 seconds)
                    (246.94, 12.0),  // B3 (offset by 12 seconds)
                    (329.63, 20.0),  // E4 (offset by 20 seconds)
                ], 0.12, sample_rate);
            }
            MusicTrack::CalmNight => {
                // Darker, more mysterious - A minor with added 9th
                Self::add_ambient_pad(&mut samples, &[
                    (110.00, 0.0),   // A2
                    (164.81, 8.0),   // E3
                    (220.00, 15.0),  // A3
                    (246.94, 25.0),  // B3 (add9)
                ], 0.10, sample_rate);
            }
            MusicTrack::Underwater => {
                // Very low, muffled drone
                Self::add_ambient_pad(&mut samples, &[
                    (55.00, 0.0),    // A1 (very low)
                    (82.41, 10.0),   // E2
                    (110.00, 20.0),  // A2
                ], 0.15, sample_rate);
                // Heavy low-pass for muffled underwater sound
                Self::apply_lowpass(&mut samples, 0.15);
                Self::apply_lowpass(&mut samples, 0.15);
            }
        }

        // Apply long fade in/out for seamless looping
        let fade_samples = (sample_rate as f32 * 5.0) as usize;  // 5 second fades
        for i in 0..fade_samples.min(num_samples / 2) {
            let factor = (i as f32 / fade_samples as f32).powf(0.5);  // Smooth curve
            samples[i] = (samples[i] as f32 * factor) as i16;
            let end_idx = num_samples - 1 - i;
            samples[end_idx] = (samples[end_idx] as f32 * factor) as i16;
        }

        samples
    }

    // Generate soft ambient pad tones with slow attack/release
    fn add_ambient_pad(samples: &mut [i16], notes: &[(f32, f32)], volume: f32, sample_rate: u32) {
        let num_samples = samples.len();
        let duration_secs = num_samples as f32 / sample_rate as f32;

        for (i, sample) in samples.iter_mut().enumerate() {
            let t = i as f32 / sample_rate as f32;
            let mut value = 0.0f32;

            for &(freq, offset_secs) in notes.iter() {
                // Each note fades in slowly, sustains, then fades out
                let note_duration = 20.0;  // Each note lasts ~20 seconds
                let attack = 6.0;   // 6 second fade in
                let release = 6.0;  // 6 second fade out

                // Calculate envelope for this note
                let note_time = t - offset_secs;
                let env = if note_time < 0.0 {
                    0.0
                } else if note_time < attack {
                    // Smooth attack curve
                    (note_time / attack).powf(2.0)
                } else if note_time < note_duration - release {
                    // Sustain
                    1.0
                } else if note_time < note_duration {
                    // Smooth release curve
                    let release_time = note_time - (note_duration - release);
                    1.0 - (release_time / release).powf(2.0)
                } else {
                    0.0
                };

                // Soft sine wave with slight vibrato
                let vibrato = 1.0 + 0.002 * (t * 4.0).sin();
                let tone = (2.0 * std::f32::consts::PI * freq * vibrato * t).sin();

                // Add a subtle harmonic (octave above, very quiet)
                let harmonic = (2.0 * std::f32::consts::PI * freq * 2.0 * t).sin() * 0.1;

                value += (tone + harmonic) * env * volume;
            }

            // Soft limiting
            value = value.tanh() * 0.8;

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

fn generate_thunder(volume: f32) -> Vec<i16> {
    let sample_rate = 44100;
    let duration_secs = 2.5;  // Thunder rumbles for 2.5 seconds
    let num_samples = (sample_rate as f32 * duration_secs) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    let mut rng = rand::thread_rng();

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let progress = i as f32 / num_samples as f32;

        // Multi-layered thunder sound
        // 1. Low rumble (20-60 Hz)
        let low_freq = 30.0 + 20.0 * (t * 0.5).sin();
        let low_rumble = (2.0 * std::f32::consts::PI * low_freq * t).sin() * 0.4;

        // 2. Mid rumble with modulation (60-150 Hz)
        let mid_freq = 80.0 + 40.0 * (t * 2.0).sin();
        let mid_rumble = (2.0 * std::f32::consts::PI * mid_freq * t).sin() * 0.3;

        // 3. Noise crackle
        let noise = rng.gen_range(-1.0..1.0) * 0.25;

        // 4. Initial crack (loud burst at start)
        let crack_envelope = if progress < 0.1 {
            (1.0 - progress / 0.1).powf(0.5)
        } else {
            0.0
        };
        let crack = noise * crack_envelope * 2.0;

        // 5. Rolling envelope (multiple peaks for rumbling effect)
        let roll1 = (progress * 8.0).sin().abs() * 0.3;
        let roll2 = ((progress + 0.25) * 6.0).sin().abs() * 0.2;
        let roll_envelope = roll1 + roll2;

        // Overall envelope: loud start, rolling middle, fade out
        let main_envelope = if progress < 0.05 {
            progress / 0.05  // Quick attack
        } else if progress < 0.3 {
            1.0  // Sustain
        } else {
            (1.0 - (progress - 0.3) / 0.7).powf(1.5)  // Long decay
        };

        // Combine all components
        let combined = (low_rumble + mid_rumble + noise * roll_envelope + crack) * main_envelope * volume;
        let sample = (combined.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        samples.push(sample);
    }

    // Apply simple low-pass filter for more realistic thunder
    let mut prev = samples[0] as f32;
    for sample in samples.iter_mut() {
        let current = *sample as f32;
        let filtered = prev * 0.7 + current * 0.3;
        *sample = filtered as i16;
        prev = filtered;
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
            BlockType::Stone | BlockType::Cobblestone => (80, 0.08),
            BlockType::Dirt | BlockType::Grass => (60, 0.06),
            BlockType::Sand | BlockType::Gravel => (50, 0.05),
            BlockType::Wood => (70, 0.06),
            BlockType::Leaves => (40, 0.04),
            BlockType::Ice | BlockType::Snow => (60, 0.05),
            _ => (70, 0.06),
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
            BlockType::Stone | BlockType::Cobblestone => (150.0, 100, 0.08),
            BlockType::Dirt | BlockType::Grass => (120.0, 80, 0.06),
            BlockType::Wood => (200.0, 90, 0.06),
            _ => (140.0, 85, 0.06),
        };

        let samples = generate_sine_wave(freq * pitch, duration, volume);
        self.play_samples(samples, 44100);
    }

    pub fn play_footstep(&self, block_type: BlockType) {
        if !self.enabled {
            return;
        }

        let (duration, volume) = match block_type {
            BlockType::Stone | BlockType::Cobblestone => (30, 0.015),
            BlockType::Dirt | BlockType::Grass => (25, 0.012),
            BlockType::Sand | BlockType::Gravel => (35, 0.010),
            BlockType::Wood => (20, 0.015),
            BlockType::Snow => (40, 0.008),
            _ => (25, 0.012),
        };

        let samples = generate_noise_burst(duration, volume);
        let sample_rate = (44100.0 * self.random_pitch(0.85, 1.15)) as u32;
        self.play_samples(samples, sample_rate);
    }

    pub fn play_splash(&self) {
        if !self.enabled {
            return;
        }

        let noise_samples = generate_noise_burst(200, 0.08);
        let sine_samples = generate_sine_wave(80.0, 200, 0.05);

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

        let samples = generate_rising_tone(100, 200.0, 500.0, 0.04);
        self.play_samples(samples, 44100);
    }

    pub fn play_land(&self) {
        if !self.enabled {
            return;
        }

        let samples = generate_noise_burst(60, 0.05);
        self.play_samples(samples, 44100);
    }

    pub fn play_thunder(&self, volume: f32) {
        if !self.enabled {
            return;
        }

        let samples = generate_thunder(volume);
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
