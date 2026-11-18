//! Framework-independent shell DSP. No history, parameters, or mode machinery.

#[derive(Debug, PartialEq)]
pub enum ConfigError {
    InvalidGain,
}

#[derive(Debug, PartialEq)]
pub struct Gain(f64);

impl Gain {
    pub fn new(value: f64) -> Result<Self, ConfigError> {
        if !value.is_finite() || value > 16.0 || value < -16.0 {
            return Err(ConfigError::InvalidGain);
        }

        Ok(Self(value))
    }

    pub fn value(&self) -> f64 {
        self.0
    }
}

pub enum Mode {
    Identity,
    Gain { gain: Gain },
}

#[derive(Debug, PartialEq)]
pub struct SampleResult {
    pub output: f32,
    pub faulted: bool,
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

pub fn gain(sample: f32, factor: &Gain) -> f64 {
    let sample64 = f64::from(sample);
    sample64 * factor.value()
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
    fn test_gain() {
        let g = Gain::new(0.5).expect("valid gain");
        let g2 = Gain::new(-1.0).expect("valid gain");
        let g3 = Gain::new(2.0).expect("valid gain");
        let g4 = Gain::new(0.0).expect("valid gain");
        assert_eq!(0.25, gain(0.5, &g));
        assert_eq!(-0.5, gain(0.5, &g2));
        assert_eq!(1.5, gain(0.75, &g3));
        assert_eq!(0.0, gain(0.5, &g4));
    }

    #[test]
    fn call_gain() {
        let g = Gain::new(2.0).expect("valid gain");

        let mode = &Mode::Gain { gain: g };
        let SampleResult { output, faulted } = pass_sample(0.5, mode);
        assert_eq!(1.0, output);
        assert!(!faulted);

        let overflow = pass_sample(f32::MAX, mode);
        assert_eq!(0.0, overflow.output);
        assert!(overflow.faulted)
    }

    #[test]
    fn parse_gain() {
        let gain = Gain::new(16.0).expect("16.0 should be accepted");

        assert_eq!(gain.value(), 16.0)
    }

    #[test]
    fn reject_invalid_gain() {
        assert_eq!(Gain::new(16.1), Err(ConfigError::InvalidGain));
    }
}
