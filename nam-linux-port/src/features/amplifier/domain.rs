//! Framework-independent, allocation-free amplifier controls and tone shaping.

#[derive(Clone, Copy)]
pub struct Controls {
    pub input: f32,
    pub threshold: f32,
    pub bass: f32,
    pub middle: f32,
    pub treble: f32,
    pub output: f32,
    pub gate_enabled: bool,
    pub eq_enabled: bool,
    pub ir_enabled: bool,
}

pub fn db_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

pub struct Domain {
    smoothing: f32,
    low_coefficient: f32,
    high_coefficient: f32,
    input_gain: f32,
    output_gain: f32,
    gate: f32,
    eq_mix: f32,
    ir_mix: f32,
    envelope: f32,
    low: [f32; 2],
    high: [f32; 2],
}

impl Domain {
    pub fn new(rate: f32) -> Self {
        Self {
            smoothing: 1.0 - (-1.0 / (rate * 0.005)).exp(),
            low_coefficient: 1.0 - (-std::f32::consts::TAU * 250.0 / rate).exp(),
            high_coefficient: 1.0 - (-std::f32::consts::TAU * 2500.0 / rate).exp(),
            input_gain: 1.0,
            output_gain: 1.0,
            gate: 1.0,
            eq_mix: 1.0,
            ir_mix: 1.0,
            envelope: 0.0,
            low: [0.0; 2],
            high: [0.0; 2],
        }
    }
    pub fn reset(&mut self) {
        self.envelope = 0.0;
        self.low = [0.0; 2];
        self.high = [0.0; 2];
        self.gate = 1.0;
    }
    pub fn input(&mut self, sample: f32, gain: f32, threshold: f32, enabled: bool) -> f32 {
        self.input_gain += self.smoothing * (gain - self.input_gain);
        let sample = sample * self.input_gain;
        self.envelope = sample.abs().max(self.envelope * (1.0 - self.smoothing));
        let target = if !enabled || self.envelope >= threshold {
            1.0
        } else {
            0.0
        };
        self.gate += self.smoothing * (target - self.gate);
        sample * self.gate
    }
    /// Complementary one-pole bands, not an emulation of NAM's official tone stack.
    pub fn eq(&mut self, samples: [f32; 2], gains: [f32; 3], enabled: bool) -> [f32; 2] {
        self.eq_mix += self.smoothing * (enabled as u8 as f32 - self.eq_mix);
        let mut out = samples;
        for channel in 0..2 {
            self.low[channel] += self.low_coefficient * (samples[channel] - self.low[channel]);
            self.high[channel] += self.high_coefficient * (samples[channel] - self.high[channel]);
            let low = self.low[channel];
            let mid = self.high[channel] - low;
            let high = samples[channel] - self.high[channel];
            let wet = low * gains[0] + mid * gains[1] + high * gains[2];
            out[channel] += self.eq_mix * (wet - samples[channel]);
        }
        out
    }
    pub fn output(&mut self, dry: [f32; 2], wet: [f32; 2], gain: f32, ir: bool) -> [f32; 2] {
        self.output_gain += self.smoothing * (gain - self.output_gain);
        self.ir_mix += self.smoothing * (ir as u8 as f32 - self.ir_mix);
        [0, 1].map(|i| (dry[i] + self.ir_mix * (wet[i] - dry[i])) * self.output_gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_eq_and_gain_are_transparent() {
        let mut dsp = Domain::new(48000.0);
        for _ in 0..1000 {
            let x = dsp.input(0.3, 1.0, db_gain(-80.0), true);
            let y = dsp.eq([x, -x], [1.0; 3], true);
            let z = dsp.output(y, y, 1.0, true);
            assert!((z[0] - 0.3).abs() < 1e-6);
            assert!((z[1] + 0.3).abs() < 1e-6);
        }
    }
    #[test]
    fn gate_closes_and_bypass_opens() {
        let mut dsp = Domain::new(48000.0);
        let mut x = 0.0;
        for _ in 0..5000 {
            x = dsp.input(0.001, 1.0, 0.01, true);
        }
        assert!(x.abs() < 1e-8);
        for _ in 0..5000 {
            x = dsp.input(0.001, 1.0, 0.01, false);
        }
        assert!((x - 0.001).abs() < 1e-7);
    }
    #[test]
    fn gain_and_tone_settle() {
        let mut dsp = Domain::new(48000.0);
        let mut y = [0.0; 2];
        for _ in 0..10000 {
            let x = dsp.input(0.1, 2.0, 0.0, false);
            y = dsp.eq([x; 2], [2.0, 1.0, 1.0], true);
            y = dsp.output(y, y, 0.5, false);
        }
        assert!((y[0] - 0.2).abs() < 1e-5);
    }
}
