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

pub fn gain(frame: [f32; 2], factor: &Gain) -> [f64; 2] {
    let left = f64::from(frame[0]);
    let right = f64::from(frame[1]);
    [left * factor.value(), right * factor.value()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{pass_frame, Mode, SampleResult};

    #[test]
    fn test_gain() {
        let g = Gain::new(0.5).expect("valid gain");
        let g2 = Gain::new(-1.0).expect("valid gain");
        let g3 = Gain::new(2.0).expect("valid gain");
        let g4 = Gain::new(0.0).expect("valid gain");
        assert_eq!([0.25, -0.125], gain([0.5, -0.25], &g));
        assert_eq!([-0.5, 0.25], gain([0.5, -0.25], &g2));
        assert_eq!([1.5, -0.5], gain([0.75, -0.25], &g3));
        assert_eq!([0.0, 0.0], gain([0.5, -0.25], &g4));
    }

    #[test]
    fn call_gain() {
        let g = Gain::new(2.0).expect("valid gain");

        let mode = &Mode::Gain { gain: g };
        assert_eq!(
            pass_frame([0.5, -0.25], mode),
            SampleResult {
                output: [1.0, -0.5],
                faulted: false,
            }
        );

        for frame in [[f32::MAX, -0.25], [0.5, -f32::MAX]] {
            assert_eq!(
                pass_frame(frame, mode),
                SampleResult {
                    output: [0.0, 0.0],
                    faulted: true,
                }
            );
        }
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
