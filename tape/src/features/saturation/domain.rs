//! Framework-independent, allocation-free stereo tape coloration.
//!
//! This is a creative tape model, not a calibrated machine emulation. A soft
//! saturator, broad tone shelf, speed-dependent bandwidth, and a quiet modulated
//! delay provide the coloration. Channels share transport modulation, not audio.

use std::f32::consts::TAU;

const DELAY_SIZE: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    pub drive_db: f32,
    pub output_db: f32,
    pub mix: f32,
    pub tone: f32,
    pub wow: f32,
    pub speed_30_ips: bool,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            drive_db: 6.0,
            output_db: -6.0,
            mix: 1.0,
            tone: 0.0,
            wow: 0.15,
            speed_30_ips: false,
        }
    }
}

fn bounded(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

impl Parameters {
    fn targets(self) -> [f32; 6] {
        let defaults = Self::default();
        [
            10.0_f32.powf(bounded(self.drive_db, 0.0, 24.0, defaults.drive_db) / 20.0),
            10.0_f32.powf(bounded(self.output_db, -24.0, 6.0, defaults.output_db) / 20.0),
            bounded(self.mix, 0.0, 1.0, defaults.mix),
            bounded(self.tone, -1.0, 1.0, defaults.tone),
            bounded(self.wow, 0.0, 1.0, defaults.wow),
            if self.speed_30_ips { 1.0 } else { 0.0 },
        ]
    }
}

struct Channel {
    low: f32,
    bandwidth: f32,
    delay: [f32; DELAY_SIZE],
}

impl Channel {
    fn new() -> Self {
        Self {
            low: 0.0,
            bandwidth: 0.0,
            delay: [0.0; DELAY_SIZE],
        }
    }

    fn reset(&mut self) {
        self.low = 0.0;
        self.bandwidth = 0.0;
        self.delay.fill(0.0);
    }
}

pub struct TapeProcessor {
    channels: [Channel; 2],
    target: [f32; 6],
    current: [f32; 6],
    sample_rate: f32,
    smoothing: f32,
    tone_alpha: f32,
    bandwidth_alpha: [f32; 2],
    phase: [f32; 2],
    write_index: usize,
}

impl Default for TapeProcessor {
    fn default() -> Self {
        Self::new(44_100.0)
    }
}

impl TapeProcessor {
    pub fn new(sample_rate: f32) -> Self {
        let target = Parameters::default().targets();
        let mut processor = Self {
            channels: [Channel::new(), Channel::new()],
            target,
            current: target,
            sample_rate: 44_100.0,
            smoothing: 0.0,
            tone_alpha: 0.0,
            bandwidth_alpha: [0.0; 2],
            phase: [0.0; 2],
            write_index: 0,
        };
        processor.set_sample_rate(sample_rate);
        processor
    }

    /// Rates outside 8–384 kHz are clamped; nonfinite rates use 44.1 kHz.
    /// Changing rate clears history, preventing stale delay/filter state.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = bounded(sample_rate, 8_000.0, 384_000.0, 44_100.0);
        let alpha = |hz: f32| 1.0 - (-TAU * hz / self.sample_rate).exp();
        self.smoothing = 1.0 - (-1.0 / (0.02 * self.sample_rate)).exp();
        self.tone_alpha = alpha(1_200.0);
        self.bandwidth_alpha = [alpha(9_000.0), alpha(16_000.0)];
        self.reset();
    }

    /// Set automation targets. All continuous controls use 20 ms smoothing.
    pub fn set_parameters(&mut self, parameters: Parameters) {
        self.target = parameters.targets();
    }

    /// Clear audio history and snap smoothing to the current targets.
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.reset();
        }
        self.current = self.target;
        self.phase = [0.0; 2];
        self.write_index = 0;
    }

    /// Process one stereo frame without allocations or shared mutable state.
    /// Nonfinite input becomes silence. Output gain applies after the dry/wet mix.
    pub fn process_frame(&mut self, input: [f32; 2]) -> [f32; 2] {
        for (current, target) in self.current.iter_mut().zip(self.target) {
            *current += self.smoothing * (target - *current);
            if (*current - target).abs() < 1.0e-6 {
                *current = target;
            }
        }
        let [drive, output, mix, tone, wow, speed] = self.current;
        let bandwidth_alpha =
            self.bandwidth_alpha[0] + speed * (self.bandwidth_alpha[1] - self.bandwidth_alpha[0]);
        let modulation = self.phase[0].sin() + 0.18 * self.phase[1].sin();
        let delay = self.sample_rate * (0.002 + 0.00035 * wow * modulation);
        let read = self.write_index as f32 - delay;
        let whole = read.floor();
        let index = (whole as isize).rem_euclid(DELAY_SIZE as isize) as usize;
        let fraction = read - whole;
        let mut result = [0.0; 2];
        for (channel_index, channel) in self.channels.iter_mut().enumerate() {
            let dry = if input[channel_index].is_finite() {
                input[channel_index]
            } else {
                0.0
            };
            // Limit only the nonlinear path; the dry path remains transparent.
            let driven = dry.clamp(-1.0e6, 1.0e6) * drive;
            let saturated = (driven * (1.0 - 0.12 * speed)).tanh();
            channel.low += self.tone_alpha * (saturated - channel.low);
            let high = saturated - channel.low;
            let shaped = channel.low * (1.0 - 0.45 * tone) + high * (1.0 + 0.6 * tone);
            channel.bandwidth += bandwidth_alpha * (shaped - channel.bandwidth);
            channel.delay[self.write_index] = channel.bandwidth;
            let delayed = channel.delay[index] * (1.0 - fraction)
                + channel.delay[(index + 1) % DELAY_SIZE] * fraction;
            let wet = channel.bandwidth + 0.18 * wow * (delayed - channel.bandwidth);
            // f64 mixing prevents overflow for any finite f32 dry input.
            let sample =
                ((1.0 - mix) as f64 * dry as f64 + mix as f64 * wet as f64) * output as f64;
            result[channel_index] = sample.clamp(-(f32::MAX as f64), f32::MAX as f64) as f32;
        }
        self.write_index = (self.write_index + 1) % DELAY_SIZE;
        for (phase, frequency) in self.phase.iter_mut().zip([0.63, 7.3]) {
            *phase = (*phase + TAU * frequency / self.sample_rate).rem_euclid(TAU);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(parameters: Parameters, rate: f32) -> TapeProcessor {
        let mut processor = TapeProcessor::new(rate);
        processor.set_parameters(parameters);
        processor.reset();
        processor
    }

    #[test]
    fn silence_stays_silent() {
        let mut processor = TapeProcessor::default();
        for _ in 0..100_000 {
            assert_eq!(processor.process_frame([0.0; 2]), [0.0; 2]);
        }
    }

    #[test]
    fn dry_mix_is_unity_at_zero_output_db() {
        let mut processor = configured(
            Parameters {
                mix: 0.0,
                output_db: 0.0,
                ..Parameters::default()
            },
            48_000.0,
        );
        for input in [[0.25, -0.75], [f32::MAX, f32::MIN], [0.0, 1.0]] {
            assert_eq!(processor.process_frame(input), input);
        }
    }

    #[test]
    fn invalid_controls_and_extreme_audio_remain_finite() {
        for rate in [
            f32::NAN,
            f32::INFINITY,
            -1.0,
            8_000.0,
            44_100.0,
            192_000.0,
            384_000.0,
        ] {
            let mut processor = configured(
                Parameters {
                    drive_db: f32::NAN,
                    output_db: 6.0,
                    mix: 0.5,
                    tone: f32::INFINITY,
                    wow: f32::MAX,
                    speed_30_ips: true,
                },
                rate,
            );
            for i in 0..20_000 {
                let input = [f32::MAX, f32::MIN, f32::NAN, f32::INFINITY, 1.0e-38][i % 5];
                assert!(processor
                    .process_frame([input, -input])
                    .iter()
                    .all(|x| x.is_finite()));
            }
            processor.set_sample_rate(96_000.0);
            assert_eq!(processor.process_frame([0.0; 2]), [0.0; 2]);
        }
    }

    #[test]
    fn channels_do_not_leak_and_identical_inputs_match() {
        let mut processor = TapeProcessor::default();
        for i in 0..10_000 {
            assert_eq!(
                processor.process_frame([(i as f32 * 0.1).sin(), 0.0])[1],
                0.0
            );
        }
        processor.reset();
        for i in 0..10_000 {
            let value = (i as f32 * 0.1).sin();
            let frame = processor.process_frame([value; 2]);
            assert_eq!(frame[0], frame[1]);
        }
    }

    fn render(parameters: Parameters, rate: f32, hz: f32) -> Vec<f32> {
        let mut processor = configured(parameters, rate);
        (0..(rate * 0.25) as usize)
            .map(|i| processor.process_frame([0.2 * (TAU * hz * i as f32 / rate).sin(); 2])[0])
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn controls_change_the_signal() {
        let base = Parameters {
            drive_db: 0.0,
            output_db: 0.0,
            wow: 0.0,
            ..Parameters::default()
        };
        let original = render(base, 48_000.0, 4_000.0);
        assert!(
            rms(&render(
                Parameters {
                    drive_db: 18.0,
                    ..base
                },
                48_000.0,
                4_000.0
            )) > rms(&original) * 2.0
        );
        let quiet = render(
            Parameters {
                output_db: -6.0,
                ..base
            },
            48_000.0,
            4_000.0,
        );
        assert!((rms(&quiet) / rms(&original) - 10.0_f32.powf(-6.0 / 20.0)).abs() < 0.001);
        assert!(
            rms(&render(Parameters { tone: 1.0, ..base }, 48_000.0, 4_000.0))
                > rms(&render(
                    Parameters { tone: -1.0, ..base },
                    48_000.0,
                    4_000.0
                )) * 1.3
        );
        for changed in [
            Parameters { wow: 1.0, ..base },
            Parameters {
                speed_30_ips: true,
                ..base
            },
        ] {
            let alternate = render(changed, 48_000.0, 4_000.0);
            assert!(original
                .iter()
                .zip(alternate)
                .any(|(a, b)| (a - b).abs() > 0.001));
        }
    }

    #[test]
    fn smoothing_uses_time_not_sample_count() {
        for rate in [8_000.0, 44_100.0, 96_000.0, 384_000.0] {
            let mut processor = configured(
                Parameters {
                    mix: 0.0,
                    ..Parameters::default()
                },
                rate,
            );
            processor.set_parameters(Parameters {
                mix: 1.0,
                ..Parameters::default()
            });
            for _ in 0..(rate * 0.02) as usize {
                processor.process_frame([0.0; 2]);
            }
            assert!((processor.current[2] - (1.0 - (-1.0_f32).exp())).abs() < 0.003);
        }
    }

    #[test]
    fn low_frequency_response_is_consistent_across_rates() {
        let parameters = Parameters {
            wow: 0.0,
            ..Parameters::default()
        };
        let reference = rms(&render(parameters, 44_100.0, 100.0));
        for rate in [8_000.0, 48_000.0, 96_000.0, 192_000.0, 384_000.0] {
            assert!((rms(&render(parameters, rate, 100.0)) / reference - 1.0).abs() < 0.01);
        }
    }
}
