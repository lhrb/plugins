use std::{
    f64::consts::{FRAC_1_SQRT_2, PI},
    ops::RangeInclusive,
};

use super::ConfigError;

pub const CUTOFF_RANGE: RangeInclusive<f64> = 0.0001..=0.49;
pub const Q_RANGE: RangeInclusive<f64> = 0.1..=20.0;
pub const DEFAULT_CUTOFF: f64 = 0.1;
pub const DEFAULT_Q: f64 = FRAC_1_SQRT_2;

/// Convert a plain f32 control value to the domain value, preserving exact endpoints.
pub fn low_pass_control(value: f32, range: RangeInclusive<f64>) -> Option<f64> {
    let (min, max) = (*range.start(), *range.end());
    if !(min as f32..=max as f32).contains(&value) {
        None
    } else if value == min as f32 {
        Some(min)
    } else if value == max as f32 {
        Some(max)
    } else {
        Some(f64::from(value))
    }
}

#[derive(Debug)]
pub struct Coefficients {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coefficients {
    pub fn from_cutoff_q(u: f64, q: f64) -> Result<Self, ConfigError> {
        if !CUTOFF_RANGE.contains(&u) || !Q_RANGE.contains(&q) {
            return Err(ConfigError::InvalidCoefficient);
        }

        let w = 2.0 * PI * u;
        let alpha = w.sin() / (2.0 * q);
        let cos_w = w.cos();
        let a0 = 1.0 + alpha;
        let b0 = ((1.0 - cos_w) / 2.0) / a0;

        Ok(Self {
            b0,
            b1: (1.0 - cos_w) / a0,
            b2: b0,
            a1: -2.0 * cos_w / a0,
            a2: (1.0 - alpha) / a0,
        })
    }
}

#[derive(Debug, Default)]
struct History {
    x1: f64, // previous input
    x2: f64, // input two samples ago
    y1: f64, // previous output
    y2: f64, // output two samples ago
}

#[derive(Debug)]
pub(super) struct LowPass {
    pub(super) coefficients: Coefficients,
    history: [History; 2],
}

impl LowPass {
    pub(super) fn new() -> Self {
        Self {
            coefficients: Coefficients::from_cutoff_q(DEFAULT_CUTOFF, DEFAULT_Q)
                .expect("valid Low-pass defaults"),
            history: Default::default(),
        }
    }

    pub(super) fn process_frame(&mut self, [left, right]: [f64; 2]) -> [f64; 2] {
        let Coefficients { b0, b1, b2, a1, a2 } = self.coefficients;
        let mut channel = |index: usize, x: f64| {
            let h = &mut self.history[index];
            let y = b0 * x + b1 * h.x1 + b2 * h.x2 - a1 * h.y1 - a2 * h.y2;
            *h = History {
                x1: x,
                x2: h.x1,
                y1: y,
                y2: h.y1,
            };
            y
        };
        [channel(0, left), channel(1, right)]
    }

    pub(super) fn reset(&mut self) {
        self.history = Default::default();
    }

    pub(super) fn reset_channel(&mut self, index: usize) {
        self.history[index] = History::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_1_SQRT_2;

    #[test]
    fn default_coefficients_match_butterworth_reference() {
        let Coefficients { b0, b1, b2, a1, a2 } =
            Coefficients::from_cutoff_q(0.1, FRAC_1_SQRT_2).expect("valid defaults");

        // Reference: bilinear transform of 1/(s² + √2s + 1), prewarped at u = 0.1.
        for (name, actual, expected) in [
            ("b0", b0, 0.0674552738890719),
            ("b1", b1, 0.1349105477781438),
            ("b2", b2, 0.0674552738890719),
            ("a1", a1, -1.1429805025399011),
            ("a2", a2, 0.4128015980961886),
        ] {
            assert!(
                (actual - expected).abs() < 1e-12,
                "{name}: expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn rejects_each_invalid_parameter_independently() {
        for (u, q) in [
            (0.00009, 1.0),
            (0.49001, 1.0),
            (f64::NAN, 1.0),
            (f64::NEG_INFINITY, 1.0),
            (f64::INFINITY, 1.0),
            (0.1, 0.099),
            (0.1, 20.001),
            (0.1, f64::NAN),
            (0.1, f64::NEG_INFINITY),
            (0.1, f64::INFINITY),
        ] {
            assert!(
                matches!(
                    Coefficients::from_cutoff_q(u, q),
                    Err(ConfigError::InvalidCoefficient)
                ),
                "expected InvalidCoefficient for u={u}, q={q}"
            );
        }
    }

    #[test]
    fn accepts_inclusive_parameter_bounds() {
        for (u, q) in [(0.0001, 1.0), (0.49, 1.0), (0.1, 0.1), (0.1, 20.0)] {
            assert!(Coefficients::from_cutoff_q(u, q).is_ok(), "u={u}, q={q}");
        }
    }
}
