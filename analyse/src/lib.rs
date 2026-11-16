//! Small, format-independent experiment helpers; not a measurement framework.
pub const BLOCK: usize = 256;
pub const RATE: f64 = 48_000.0;
pub const PRE_BLOCKS: usize = 64;
pub const RENDER_BLOCKS: usize = 128;
pub const TOLERANCE: f64 = 1e-6;

/// Same finite stereo input for every instance. Channels intentionally differ.
pub fn input(block: usize) -> [[f32; BLOCK]; 2] {
    std::array::from_fn(|ch| {
        std::array::from_fn(|i| {
            let n = (block * BLOCK + i) as f64;
            (0.1 * (std::f64::consts::TAU * [997.0, 1499.0][ch] * n / RATE).sin()) as f32
        })
    })
}

pub fn difference(a: &[f32], b: &[f32]) -> (f64, f64) {
    assert_eq!(a.len(), b.len());
    assert!(!a.is_empty());
    assert!(a.iter().chain(b).all(|x| x.is_finite()));
    let mut peak: f64 = 0.0;
    let mut sum = 0.0;
    for (&a, &b) in a.iter().zip(b) {
        let d = a as f64 - b as f64;
        peak = peak.max(d.abs());
        sum += d * d;
    }
    (peak, (sum / a.len() as f64).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryGain {
        gain: f32,
        history: f32,
    }
    impl MemoryGain {
        fn reset(&mut self) {
            self.history = 0.0;
        }
        fn render(&mut self) -> Vec<f32> {
            input(0)[0]
                .iter()
                .map(|x| {
                    self.history = 0.99 * self.history + *x * self.gain;
                    self.history
                })
                .collect()
        }
    }
    #[test]
    fn deterministic_reset_and_independent_settings_and_history() {
        let mut a = MemoryGain {
            gain: 0.5,
            ..Default::default()
        };
        let mut b = MemoryGain {
            gain: 0.75,
            ..Default::default()
        };
        let mut control = MemoryGain {
            gain: 0.75,
            ..Default::default()
        };
        let first = a.render();
        a.reset();
        assert_eq!(difference(&first, &a.render()), (0.0, 0.0));
        assert_eq!(b.render(), control.render());
        a.gain = 2.0;
        a.reset();
        assert!(difference(&first, &a.render()).0 > 0.01);
        let continued = b.render();
        assert_eq!(continued, control.render());
        assert_eq!(b.gain, 0.75);
        b.reset();
        // This catches a test that accidentally resets away the history under test.
        assert!(difference(&continued, &b.render()).0 > 0.001);
    }
    #[test]
    fn input_and_difference_are_sensitive() {
        assert_eq!(input(3), input(3));
        assert_ne!(input(0), input(1));
        assert_ne!(input(0)[0], input(0)[1]);
        assert_eq!(difference(&[0.0, 1.0], &[0.0, 0.0]), (1.0, 0.5_f64.sqrt()));
    }
}
