//! Framework-independent shell DSP. No history, parameters, or mode machinery.

/// Preserve every finite input bit, including signed zero. Latch faults in the adapter.
pub fn pass_sample(sample: f32) -> (f32, bool) {
    if sample.is_finite() {
        (sample, false)
    } else {
        (0.0, true)
    }
}

pub fn supports_sample_rate(rate: f32) -> bool {
    (44_100.0..=192_000.0).contains(&rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_bits_and_nonfinite_safety() {
        for bits in [
            0, 0x80000000, 1, 0x80000001, 0x3f800000, 0xc0200000, 0x7f7fffff,
        ] {
            let (output, fault) = pass_sample(f32::from_bits(bits));
            assert_eq!(output.to_bits(), bits);
            assert!(!fault);
        }
        for input in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(pass_sample(input), (0.0, true));
            assert_eq!(pass_sample(-0.5), (-0.5, false));
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
}
