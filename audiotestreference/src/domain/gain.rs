use super::ConfigError;

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

pub fn gain(sample: f32, factor: &Gain) -> f64 {
    let sample64 = f64::from(sample);
    sample64 * factor.value()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{pass_sample, Mode, SampleResult};

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
