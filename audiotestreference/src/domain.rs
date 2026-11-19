//! Framework-independent shell DSP. No history, parameters, or mode machinery.
mod gain;

pub use self::gain::{gain, Gain};

#[derive(Debug, PartialEq)]
pub enum ConfigError {
    InvalidGain,
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Identity,
    Gain { gain: Gain },
}

#[derive(Debug, PartialEq)]
pub struct SampleResult {
    pub output: [f32; 2],
    pub faulted: bool,
}

#[derive(Debug, PartialEq)]
pub struct Processor {
    mode: Mode,
}

impl Processor {
    pub fn new() -> Self {
        Self {
            mode: Mode::Identity,
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn process_frame(&mut self, frame: [f32; 2]) -> SampleResult {
        pass_frame(frame, &self.mode)
    }
}

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}

pub fn pass_frame(frame: [f32; 2], mode: &Mode) -> SampleResult {
    if !frame[0].is_finite() || !frame[1].is_finite() {
        return SampleResult {
            output: [0.0, 0.0],
            faulted: true,
        };
    }

    let result = match mode {
        Mode::Identity => identity(frame),
        Mode::Gain { gain: factor } => gain(frame, factor),
    };

    if !result[0].is_finite()
        || result[0].abs() > f64::from(f32::MAX)
        || !result[1].is_finite()
        || result[1].abs() > f64::from(f32::MAX)
    {
        return SampleResult {
            output: [0.0, 0.0],
            faulted: true,
        };
    }

    SampleResult {
        output: [result[0] as f32, result[1] as f32],
        faulted: false,
    }
}

pub fn identity(frame: [f32; 2]) -> [f64; 2] {
    [f64::from(frame[0]), f64::from(frame[1])]
}

pub fn supports_sample_rate(rate: f32) -> bool {
    (44_100.0..=192_000.0).contains(&rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_bits_and_nonfinite_safety() {
        let mode = Mode::Identity;

        for bits in [
            0, 0x80000000, 1, 0x80000001, 0x3f800000, 0xc0200000, 0x7f7fffff,
        ] {
            let sample = f32::from_bits(bits);
            for frame in [[sample, -0.5], [-0.5, sample]] {
                let SampleResult { output, faulted } = pass_frame(frame, &mode);
                assert_eq!(output.map(f32::to_bits), frame.map(f32::to_bits));
                assert!(!faulted);
            }
        }
        for input in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for frame in [[input, -0.5], [-0.5, input]] {
                assert_eq!(
                    pass_frame(frame, &mode),
                    SampleResult {
                        output: [0.0, 0.0],
                        faulted: true,
                    }
                );
                assert_eq!(
                    pass_frame([-0.5, 0.25], &mode),
                    SampleResult {
                        output: [-0.5, 0.25],
                        faulted: false,
                    }
                );
            }
        }
    }

    #[test]
    fn activation_rate_boundaries() {
        for rate in [44_100.0, 48_000.0, 192_000.0] {
            assert!(supports_sample_rate(rate));
        }
        for rate in [44_099.0, 192_001.0, f32::NAN, f32::INFINITY] {
            assert!(!supports_sample_rate(rate));
        }
    }

    #[test]
    fn processor() {
        let mut p = Processor::new();
        let gain_mode = Mode::Gain {
            gain: Gain::new(2.0).expect("valid"),
        };
        p.set_mode(gain_mode);

        let input = [[0.0, 0.5], [-0.5, 0.0]];
        let expected = [[0.0, 1.0], [-1.0, 0.0]];
        for (frame, output) in input.into_iter().zip(expected) {
            assert_eq!(
                p.process_frame(frame),
                SampleResult {
                    output,
                    faulted: false,
                }
            );
        }
    }
}
