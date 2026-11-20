use super::ConfigError;

pub const MAX_DELAY_SAMPLES: usize = 192_000;

#[derive(Debug)]
pub struct Delay {
    memory: Vec<[f64; 2]>,
    n: usize,
    cursor: usize,
    filled: usize,
}

impl Delay {
    pub fn new(n: usize) -> Result<Self, ConfigError> {
        if n > MAX_DELAY_SAMPLES {
            return Err(ConfigError::InvalidDelay);
        }

        Ok(Self {
            memory: vec![[0.0f64; 2]; MAX_DELAY_SAMPLES],
            n,
            cursor: 0,
            filled: 0,
        })
    }

    pub fn process_frame(&mut self, frame: [f64; 2]) -> [f64; 2] {
        if self.n == 0 {
            return frame;
        }

        let old = if self.filled < self.n {
            self.filled += 1;
            [0.0; 2]
        } else {
            self.memory[self.cursor]
        };

        self.memory[self.cursor] = frame;

        self.cursor += 1;
        if self.cursor >= self.n {
            self.cursor = 0;
        }

        old
    }

    pub fn reset(&mut self) {
        self.filled = 0;
        self.cursor = 0;
    }

    pub fn change_delay(&mut self, n: usize) -> Result<(), ConfigError> {
        if self.n == n {
            return Ok(());
        }

        if n > MAX_DELAY_SAMPLES {
            return Err(ConfigError::InvalidDelay);
        }

        self.reset();
        self.n = n;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_delay_preserves_frame_bits() {
        let mut delay = Delay::new(0).expect("zero is a valid delay length");
        let frame = [-0.0_f64, 0.5];
        assert_eq!(
            delay.process_frame(frame).map(f64::to_bits),
            frame.map(f64::to_bits)
        );
    }

    #[test]
    fn maximum_delay_outputs_impulse_at_frame_192_000() {
        let mut delay = Delay::new(192_000).expect("192,000 is a valid delay length");
        let impulse = [1.0, -0.5];
        let silence = [0.0; 2];
        assert_eq!(delay.process_frame(impulse), silence, "frame 0");
        for frame in 1..192_000 {
            assert_eq!(delay.process_frame(silence), silence, "frame {frame}");
        }
        assert_eq!(delay.process_frame(silence), impulse, "frame 192,000");
    }

    #[test]
    fn reset_discards_queued_frames_without_changing_delay() {
        let mut delay = Delay::new(2).expect("2 is a valid delay length");
        let silence = [0.0; 2];
        assert_eq!(delay.process_frame([1.0, -0.5]), silence);
        assert_eq!(delay.process_frame([0.5, 0.25]), silence);

        delay.reset();

        let new_frame = [-0.25, 1.0];
        assert_eq!(delay.process_frame(new_frame), silence, "frame 0");
        assert_eq!(delay.process_frame(silence), silence, "frame 1");
        assert_eq!(delay.process_frame(silence), new_frame, "frame 2");
    }

    #[test]
    fn test_delay() {
        let mut delay = Delay::new(2).expect("2 is a valid delay length");
        assert_eq!(delay.process_frame([1.0, -0.5]), [0.0, 0.0]);
        assert_eq!(delay.process_frame([0.5, 0.25]), [0.0, 0.0]);
        assert_eq!(delay.process_frame([0.0, 0.0]), [1.0, -0.5]);
        assert_eq!(delay.process_frame([0.0, 0.0]), [0.5, 0.25]);
    }

    #[test]
    fn change_delay_time() {
        let mut delay = Delay::new(1).expect("1 is a valid delay length");
        assert_eq!(delay.process_frame([1.0, -0.5]), [0.0, 0.0]);
        assert_eq!(delay.process_frame([1.0, 1.0]), [1.0, -0.5]);
        delay.change_delay(2).expect("works");
        assert_eq!(delay.process_frame([1.0, -0.5]), [0.0, 0.0]);
        assert_eq!(delay.process_frame([0.5, 0.25]), [0.0, 0.0]);
        assert_eq!(delay.process_frame([0.0, 0.0]), [1.0, -0.5]);
        assert_eq!(delay.process_frame([0.0, 0.0]), [0.5, 0.25]);
    }

    #[test]
    fn unchanged_length_does_not_clear_history() {
        let mut delay = Delay::new(1).expect("1 is a valid delay length");
        assert_eq!(delay.process_frame([1.0, -0.5]), [0.0, 0.0]);
        assert_eq!(delay.process_frame([1.0, 1.0]), [1.0, -0.5]);
        delay.change_delay(1).expect("works");
        assert_eq!(delay.process_frame([1.0, 1.0]), [1.0, 1.0]);
    }
}
