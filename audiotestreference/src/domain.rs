//! Framework-independent stereo DSP and mode state.
mod delay;
mod gain;
mod low_pass;

use self::{delay::Delay, low_pass::LowPass};

pub use self::{
    delay::MAX_DELAY_SAMPLES,
    gain::{gain, Gain},
    low_pass::{low_pass_control, Coefficients, CUTOFF_RANGE, DEFAULT_CUTOFF, DEFAULT_Q, Q_RANGE},
};

#[derive(Debug, PartialEq)]
pub enum ConfigError {
    InvalidGain,
    InvalidDelay,
    InvalidCoefficient,
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    Identity,
    Gain { gain: Gain },
    Delay { n: usize },
    LowPass { cutoff: f64, q: f64 },
}

#[derive(Debug, PartialEq)]
pub struct SampleResult {
    pub output: [f32; 2],
    pub faulted: bool,
}

#[derive(Debug)]
pub struct Processor {
    mode: Mode,
    delay: Delay,
    low_pass: LowPass,
}

impl Processor {
    pub fn new() -> Self {
        Self {
            mode: Mode::Identity,
            delay: Delay::new(0).expect("zero is a valid delay length"),
            low_pass: LowPass::new(),
        }
    }

    pub fn set_mode(&mut self, mode: Mode) -> Result<(), ConfigError> {
        match mode {
            Mode::Delay { n } => self.delay.change_delay(n)?,
            Mode::LowPass { cutoff, q } if self.mode != mode => {
                self.low_pass.coefficients = Coefficients::from_cutoff_q(cutoff, q)?;
            }
            _ => {}
        }
        if std::mem::discriminant(&self.mode) != std::mem::discriminant(&mode) {
            self.reset();
        }
        self.mode = mode;
        Ok(())
    }

    pub(crate) fn set_delay_samples(&mut self, n: usize) -> Result<(), ConfigError> {
        self.delay.change_delay(n)?;
        if let Mode::Delay { n: active_n } = &mut self.mode {
            *active_n = n;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.delay.reset();
        self.low_pass.reset();
    }

    pub fn process_frame(&mut self, frame: [f32; 2]) -> SampleResult {
        let (frame64, input_faulted) = convert_32_to_64(frame);

        let result = match &self.mode {
            Mode::Identity => identity(frame64),
            Mode::Gain { gain: factor } => gain(frame64, factor),
            Mode::Delay { .. } => self.delay.process_frame(frame64),
            Mode::LowPass { .. } => self.low_pass.process_frame(frame64),
        };

        let (frame32, [left_fault, right_fault]) = convert_64_to_32(result);
        if matches!(self.mode, Mode::LowPass { .. }) {
            if left_fault {
                self.low_pass.reset_channel(0);
            }
            if right_fault {
                self.low_pass.reset_channel(1);
            }
        }

        SampleResult {
            output: frame32,
            faulted: input_faulted || left_fault || right_fault,
        }
    }
}

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}

pub fn identity(frame: [f64; 2]) -> [f64; 2] {
    frame
}

pub fn supports_sample_rate(rate: f32) -> bool {
    (44_100.0..=192_000.0).contains(&rate)
}

fn convert_32_to_64(frame: [f32; 2]) -> ([f64; 2], bool) {
    let [(left, left_fault), (right, right_fault)] = frame.map(|sample| {
        if sample.is_finite() {
            (f64::from(sample), false)
        } else {
            (0.0, true)
        }
    });

    let frame64 = [left, right];
    let input_faulted = left_fault || right_fault;
    (frame64, input_faulted)
}

fn convert_64_to_32(frame: [f64; 2]) -> ([f32; 2], [bool; 2]) {
    let [(left, left_fault), (right, right_fault)] = frame.map(|sample| {
        if sample.is_finite() && sample.abs() <= f64::from(f32::MAX) {
            (sample as f32, false)
        } else {
            (0.0, true)
        }
    });

    ([left, right], [left_fault, right_fault])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_bits_and_nonfinite_safety() {
        let mut processor = Processor::new();

        for sample in [
            0.0,
            -0.0,
            f32::from_bits(1),
            f32::from_bits(0x80000001),
            1.0,
            -2.5,
            f32::MAX,
            -f32::MAX,
        ] {
            for frame in [[sample, -0.5], [-0.5, sample]] {
                let SampleResult { output, faulted } = processor.process_frame(frame);
                assert_eq!(output.map(f32::to_bits), frame.map(f32::to_bits));
                assert!(!faulted, "frame {frame:?}");
            }
        }
        for input in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for (frame, output) in [([input, -0.5], [0.0, -0.5]), ([-0.5, input], [-0.5, 0.0])] {
                assert_eq!(
                    processor.process_frame(frame),
                    SampleResult {
                        output,
                        faulted: true,
                    },
                    "frame {frame:?}"
                );
                assert_eq!(
                    processor.process_frame([-0.5, 0.25]),
                    SampleResult {
                        output: [-0.5, 0.25],
                        faulted: false,
                    }
                );
            }
        }
    }

    #[test]
    fn delay_advances_on_fault_and_queues_healthy_channel() {
        let mut processor = Processor::new();
        processor
            .set_mode(Mode::Delay { n: 1 })
            .expect("one frame is a valid delay");

        for (frame, output, faulted) in [
            ([1.0, -0.5], [0.0, 0.0], false),
            ([f32::NAN, 0.25], [1.0, -0.5], true),
            ([0.0, 0.0], [0.0, 0.25], false),
        ] {
            assert_eq!(
                processor.process_frame(frame),
                SampleResult { output, faulted },
                "frame {frame:?}"
            );
        }
    }

    #[test]
    fn unchanged_config_preserves_delay_but_mode_changes_and_reset_clear_it() {
        let mut processor = Processor::new();
        let silence = [0.0; 2];
        let impulse = [1.0, -0.5];

        processor.set_mode(Mode::Delay { n: 1 }).unwrap();
        assert_eq!(processor.process_frame(impulse).output, silence);
        processor.set_mode(Mode::Delay { n: 1 }).unwrap();
        assert_eq!(processor.process_frame(impulse).output, impulse);

        processor.set_mode(Mode::Identity).unwrap();
        processor.set_mode(Mode::Delay { n: 1 }).unwrap();
        assert_eq!(processor.process_frame(impulse).output, silence);

        processor.set_mode(Mode::Identity).unwrap();
        processor.set_delay_samples(2).unwrap();
        processor.set_mode(Mode::Delay { n: 2 }).unwrap();
        assert_eq!(processor.process_frame(impulse).output, silence);
        processor.reset();
        assert_eq!(processor.process_frame(silence).output, silence);
        assert_eq!(processor.process_frame(silence).output, silence);
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
        p.set_mode(gain_mode).expect("valid gain mode");

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
