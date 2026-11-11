//! Output-only demo audio. Dropping `Preview` stops its stream.

use std::{
    f32::consts::TAU,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use oxide_tape::features::saturation::domain::{Parameters, TapeProcessor};

/// Independent relaxed atomics; a snapshot is not a transactional update.
/// Float controls and the output peak meter store `f32::to_bits()` values.
pub struct Controls {
    pub drive_db: AtomicU32,
    pub output_db: AtomicU32,
    pub mix: AtomicU32,
    pub tone: AtomicU32,
    pub wow: AtomicU32,
    pub speed_30_ips: AtomicBool,
    pub playing: AtomicBool,
    pub bypass: AtomicBool,
    pub meter: AtomicU32,
}

impl Default for Controls {
    fn default() -> Self {
        let parameters = Parameters::default();
        Self {
            drive_db: AtomicU32::new(parameters.drive_db.to_bits()),
            output_db: AtomicU32::new(parameters.output_db.to_bits()),
            mix: AtomicU32::new(parameters.mix.to_bits()),
            tone: AtomicU32::new(parameters.tone.to_bits()),
            wow: AtomicU32::new(parameters.wow.to_bits()),
            speed_30_ips: AtomicBool::new(parameters.speed_30_ips),
            playing: AtomicBool::new(false),
            bypass: AtomicBool::new(false),
            meter: AtomicU32::new(0.0_f32.to_bits()),
        }
    }
}

impl Controls {
    /// Read each parameter once. The domain validates and smooths these values.
    pub fn parameters(&self) -> Parameters {
        Parameters {
            drive_db: f32::from_bits(self.drive_db.load(Ordering::Relaxed)),
            output_db: f32::from_bits(self.output_db.load(Ordering::Relaxed)),
            mix: f32::from_bits(self.mix.load(Ordering::Relaxed)),
            tone: f32::from_bits(self.tone.load(Ordering::Relaxed)),
            wow: f32::from_bits(self.wow.load(Ordering::Relaxed)),
            speed_30_ips: self.speed_30_ips.load(Ordering::Relaxed),
        }
    }
}

pub struct Preview {
    pub controls: Arc<Controls>,
    _stream: cpal::Stream,
}

impl Preview {
    /// Start the default output device, initially silent. Never opens an input.
    pub fn new() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device available")?;
        let supported = device
            .default_output_config()
            .map_err(|error| format!("Cannot query audio output: {error}"))?;
        let config = supported.config();
        if config.channels == 0 || config.sample_rate.0 == 0 {
            return Err("Audio output has an invalid channel count or sample rate".into());
        }
        let controls = Arc::new(Controls::default());
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, controls.clone()),
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, controls.clone()),
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, controls.clone()),
            format => return Err(format!("Unsupported audio output format: {format:?}")),
        }?;
        stream
            .play()
            .map_err(|error| format!("Cannot start audio output: {error}"))?;
        Ok(Self {
            controls,
            _stream: stream,
        })
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    controls: Arc<Controls>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut audio = DemoAudio::new(config.sample_rate.0 as f32);
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| audio.render(output, channels, &controls),
            |error| eprintln!("Tape preview audio error: {error}"),
            None,
        )
        .map_err(|error| format!("Cannot build audio output: {error}"))
}

struct DemoAudio {
    processor: TapeProcessor,
    phase: [f32; 3],
    increments: [f32; 3],
    gain: f32,
    gain_step: f32,
    peak: f32,
    peak_decay: f32,
}

impl DemoAudio {
    fn new(sample_rate: f32) -> Self {
        Self {
            processor: TapeProcessor::new(sample_rate),
            phase: [0.0; 3],
            increments: [130.8128, 164.8138, 195.9977].map(|hz| TAU * hz / sample_rate),
            gain: 0.0,
            gain_step: 1.0 / (0.01 * sample_rate),
            peak: 0.0,
            peak_decay: (-1.0 / (0.15 * sample_rate)).exp(),
        }
    }

    fn render<T>(&mut self, output: &mut [T], channels: usize, controls: &Controls)
    where
        T: cpal::Sample + cpal::FromSample<f32>,
    {
        self.processor.set_parameters(controls.parameters());
        let playing = controls.playing.load(Ordering::Relaxed);
        let bypass = controls.bypass.load(Ordering::Relaxed);
        for frame in output.chunks_mut(channels) {
            // A quiet C-major chord, with different voicings on each side.
            let tones = self.phase.map(f32::sin);
            let dry = [
                0.025 * (tones[0] + 0.8 * tones[1] + 0.6 * tones[2]),
                0.025 * (0.6 * tones[0] + tones[1] + 0.8 * tones[2]),
            ];
            for (phase, increment) in self.phase.iter_mut().zip(self.increments) {
                *phase = (*phase + increment).rem_euclid(TAU);
            }
            // Keep DSP history warm while bypassed; bypass itself is true dry.
            let wet = self.processor.process_frame(dry);
            let stereo = if bypass { dry } else { wet };
            let target = if playing { 1.0 } else { 0.0 };
            self.gain += (target - self.gain).clamp(-self.gain_step, self.gain_step);
            // Bound even maximum drive/output settings to a conservative peak.
            let stereo = stereo.map(|sample| sample.clamp(-0.15, 0.15) * self.gain);
            let mut peak = 0.0_f32;
            let mono = (stereo[0] + stereo[1]) * 0.5;
            for (channel, sample) in frame.iter_mut().enumerate() {
                // Mono folds down; surround uses front L/R, with other channels silent.
                let value = if channels == 1 {
                    mono
                } else {
                    stereo.get(channel).copied().unwrap_or(0.0)
                };
                *sample = T::from_sample(value);
                peak = peak.max(value.abs());
            }
            self.peak = peak.max(self.peak * self.peak_decay);
        }
        controls.meter.store(self.peak.to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_silent_and_match_domain() {
        let controls = Controls::default();
        assert_eq!(controls.parameters(), Parameters::default());
        let mut audio = DemoAudio::new(48_000.0);
        let mut output = [1.0_f32; 1024];
        audio.render(&mut output, 2, &controls);
        assert!(output.iter().all(|&sample| sample == 0.0));
        assert_eq!(f32::from_bits(controls.meter.load(Ordering::Relaxed)), 0.0);
    }

    #[test]
    fn output_is_bounded_and_surround_channels_stay_silent() {
        let controls = Controls::default();
        controls.playing.store(true, Ordering::Relaxed);
        controls
            .drive_db
            .store(24.0_f32.to_bits(), Ordering::Relaxed);
        controls
            .output_db
            .store(6.0_f32.to_bits(), Ordering::Relaxed);
        let mut audio = DemoAudio::new(96_000.0);
        let mut output = [0.0_f32; 6000];
        audio.render(&mut output, 6, &controls);
        assert!(output
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 0.15));
        assert!(output.iter().any(|sample| sample.abs() > 0.01));
        assert!(output.chunks_exact(6).all(|frame| frame[2..] == [0.0; 4]));
        let peak = f32::from_bits(controls.meter.load(Ordering::Relaxed));
        assert!(peak > 0.0 && peak <= 0.15);
    }

    #[test]
    fn bypass_ignores_processing_controls_and_mono_folds_down() {
        let controls = Controls::default();
        controls.playing.store(true, Ordering::Relaxed);
        controls.bypass.store(true, Ordering::Relaxed);
        let mut stereo_audio = DemoAudio::new(44_100.0);
        let mut mono_audio = DemoAudio::new(44_100.0);
        let mut stereo = [0.0_f32; 2048];
        let mut mono = [0.0_f32; 1024];
        stereo_audio.render(&mut stereo, 2, &controls);
        controls
            .output_db
            .store((-24.0_f32).to_bits(), Ordering::Relaxed);
        mono_audio.render(&mut mono, 1, &controls);
        for (frame, sample) in stereo.chunks_exact(2).zip(mono) {
            assert!(((frame[0] + frame[1]) * 0.5 - sample).abs() < 1.0e-7);
        }
    }

    #[test]
    fn integer_silence_uses_format_midpoint() {
        let controls = Controls::default();
        let mut audio = DemoAudio::new(48_000.0);
        let mut signed = [1_i16; 32];
        let mut unsigned = [0_u16; 32];
        audio.render(&mut signed, 1, &controls);
        audio.render(&mut unsigned, 1, &controls);
        assert_eq!(signed, [0; 32]);
        assert_eq!(unsigned, [32768; 32]);
    }
}
