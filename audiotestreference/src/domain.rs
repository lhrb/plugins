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
    pub output: f32,
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

    pub fn process_sample(&mut self, sample: f32) -> SampleResult {
        pass_sample(sample, &self.mode)
    }
}

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}

/// Preserve every finite input bit, including signed zero. Latch faults in the adapter.
pub fn pass_sample(sample: f32, mode: &Mode) -> SampleResult {
    if !sample.is_finite() {
        return SampleResult {
            output: 0.0,
            faulted: true,
        };
    }

    let result = match mode {
        Mode::Identity => identity(sample),
        Mode::Gain { gain: factor } => gain(sample, factor),
    };

    if !result.is_finite() || result.abs() > f64::from(f32::MAX) {
        return SampleResult {
            output: 0.0,
            faulted: true,
        };
    }

    SampleResult {
        output: result as f32,
        faulted: false,
    }
}

pub fn identity(sample: f32) -> f64 {
    f64::from(sample)
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
            let SampleResult { output, faulted } = pass_sample(f32::from_bits(bits), &mode);
            assert_eq!(output.to_bits(), bits);
            assert!(!faulted);
        }
        for input in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                pass_sample(input, &mode),
                SampleResult {
                    output: 0.0,
                    faulted: true
                }
            );
            assert_eq!(
                pass_sample(-0.5, &mode),
                SampleResult {
                    output: -0.5,
                    faulted: false
                }
            );
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

        let input: [f32; 4] = [0.0, 0.5, 0.0, -0.5];
        let expected: [f32; 4] = [0.0, 1.0, 0.0, -1.0];
        for (s, e) in input.iter().zip(expected.iter()) {
            assert_eq!(p.process_sample(*s).output, *e)
        }
    }
}
