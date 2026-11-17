//! Stepped-sine measurements, independent of plugin formats and UI.
use std::f64::consts::TAU;

#[derive(Clone, Debug, PartialEq)]
pub struct Conditions {
    pub frequencies_hz: Vec<f64>,
    pub level_dbfs_peak: f64,
    pub input_channel: usize,
    pub sample_rate: u32,
    pub block_frames: usize,
    pub warmup_frames: usize,
    pub settling_frames: usize,
    pub observation_frames: usize,
}
impl Default for Conditions {
    fn default() -> Self {
        Self {
            frequencies_hz: vec![375., 997., 6000.],
            level_dbfs_peak: -36.,
            input_channel: 0,
            sample_rate: 48_000,
            block_frames: 256,
            warmup_frames: 4096,
            settling_frames: 8192,
            observation_frames: 16384,
        }
    }
}
impl Conditions {
    pub fn validate(&self) -> Result<(), MeasureError> {
        let valid = self.sample_rate == 48_000
            && self.block_frames == 256
            && self.input_channel < 2
            && self.level_dbfs_peak.is_finite()
            && (-100.0..=0.0).contains(&self.level_dbfs_peak)
            && !self.frequencies_hz.is_empty()
            && self.frequencies_hz.iter().all(|f| {
                f.is_finite()
                    && *f > 0.
                    && *f < self.sample_rate as f64 / 2.
                    && *f * self.observation_frames as f64 / self.sample_rate as f64 >= 8.
            })
            && [
                self.warmup_frames,
                self.settling_frames,
                self.observation_frames,
            ]
            .iter()
            .all(|n| *n > 0 && *n % self.block_frames == 0)
            && self.observation_frames >= 1024;
        if valid {
            Ok(())
        } else {
            Err(MeasureError::InvalidConfiguration)
        }
    }
}
pub trait Processor {
    fn reset(&mut self) -> bool;
    fn process(&mut self, input: &[[f64; 2]], output: &mut [[f64; 2]]) -> Result<(), String>;
}
#[derive(Clone, Debug, PartialEq)]
pub enum MeasureError {
    InvalidConfiguration,
    ProcessorFailure(String),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    pub conditions: Conditions,
    pub points: Vec<Point>,
    pub delay_samples: Option<f64>,
    pub reset_supported: bool,
    pub history_warning: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub frequency_hz: f64,
    pub channels: [Response; 2],
}
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Available { gain_db: f64, phase_deg: f64 },
    Unavailable(Unavailable),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Unavailable {
    BelowFloor,
    Unstable,
    NonFinite,
}

// Least-squares DC + sine/cosine fit. Unlike a raw DFT, this does not assume
// integer cycles; fit DC separately rather than leaking it into the fundamental.
fn fundamental(
    samples: &[[f64; 2]],
    channel: usize,
    start: usize,
    omega: f64,
) -> Option<(f64, f64)> {
    let mut a = [[0.; 4]; 3];
    for (i, frame) in samples.iter().enumerate() {
        let phase = omega * (start + i) as f64;
        let basis = [phase.sin(), phase.cos(), 1.];
        for r in 0..3 {
            for c in 0..3 {
                a[r][c] += basis[r] * basis[c];
            }
            a[r][3] += basis[r] * frame[channel];
        }
    }
    for pivot in 0..3 {
        let row = (pivot..3).max_by(|&r, &s| a[r][pivot].abs().total_cmp(&a[s][pivot].abs()))?;
        a.swap(pivot, row);
        let d = a[pivot][pivot];
        // Reject a singular/ill-conditioned fit rather than publishing NaNs or
        // amplifying round-off into a finite-looking response.
        if !d.is_finite() || d.abs() < samples.len() as f64 * 1e-8 {
            return None;
        }
        for c in pivot..4 {
            a[pivot][c] /= d;
        }
        for r in 0..3 {
            if r != pivot {
                let d = a[r][pivot];
                for c in pivot..4 {
                    a[r][c] -= d * a[pivot][c];
                }
            }
        }
    }
    let fit = (a[0][3], a[1][3]);
    (fit.0.is_finite() && fit.1.is_finite()).then_some(fit)
}

pub fn measure(c: &Conditions, processor: &mut dyn Processor) -> Result<Measurement, MeasureError> {
    measure_with_progress(c, processor, |_| {})
}

/// Progress follows each complete frequency. Adapters implement cancellation in
/// `process`, so no framework or thread primitive enters this module.
pub fn measure_with_progress(
    c: &Conditions,
    processor: &mut dyn Processor,
    mut progress: impl FnMut(usize),
) -> Result<Measurement, MeasureError> {
    c.validate()?;
    let reset_supported = processor.reset();
    let amplitude = 10_f64.powf(c.level_dbfs_peak / 20.);
    let mut input = vec![[0.; 2]; c.block_frames];
    let mut output = input.clone();
    for _ in 0..c.warmup_frames / c.block_frames {
        processor
            .process(&input, &mut output)
            .map_err(MeasureError::ProcessorFailure)?;
    }
    let mut points = Vec::new();
    for &frequency_hz in &c.frequencies_hz {
        let omega = TAU * frequency_hz / c.sample_rate as f64;
        let mut observed = Vec::with_capacity(c.observation_frames);
        for start in (0..c.settling_frames + c.observation_frames).step_by(c.block_frames) {
            for (i, frame) in input.iter_mut().enumerate() {
                *frame = [0.; 2];
                frame[c.input_channel] = amplitude * (omega * (start + i) as f64).sin();
            }
            output.fill([f64::NAN; 2]);
            processor
                .process(&input, &mut output)
                .map_err(MeasureError::ProcessorFailure)?;
            if start >= c.settling_frames {
                observed.extend_from_slice(&output);
            }
        }
        let channels = std::array::from_fn(|ch| {
            if observed.iter().any(|x| !x[ch].is_finite()) {
                return Response::Unavailable(Unavailable::NonFinite);
            }
            // Resolve at least one cycle of both the tone and its Nyquist beat
            // in each stability window. Near Nyquist the sine/cosine bases
            // otherwise collapse, making phase and amplitude unreliable.
            let half = observed.len() / 2;
            let resolved_hz = frequency_hz.min(c.sample_rate as f64 / 2. - frequency_hz);
            if resolved_hz * half as f64 / (c.sample_rate as f64) < 1. {
                return Response::Unavailable(Unavailable::Unstable);
            }
            let Some((s, co)) = fundamental(&observed, ch, c.settling_frames, omega) else {
                return Response::Unavailable(Unavailable::Unstable);
            };
            let peak = s.hypot(co);
            if peak < 1e-5 {
                return Response::Unavailable(Unavailable::BelowFloor);
            }
            let (Some((s1, c1)), Some((s2, c2))) = (
                fundamental(&observed[..half], ch, c.settling_frames, omega),
                fundamental(&observed[half..], ch, c.settling_frames + half, omega),
            ) else {
                return Response::Unavailable(Unavailable::Unstable);
            };
            // More than 0.5 dB amplitude or 3 degrees phase drift is not stable.
            let drift_db = 20. * (s1.hypot(c1) / s2.hypot(c2)).log10();
            let drift_phase = (c1.atan2(s1) - c2.atan2(s2) + std::f64::consts::PI).rem_euclid(TAU)
                - std::f64::consts::PI;
            if !drift_db.is_finite()
                || !drift_phase.is_finite()
                || drift_db.abs() > 0.5
                || drift_phase.to_degrees().abs() > 3.
            {
                return Response::Unavailable(Unavailable::Unstable);
            }
            let gain_db = 20. * (peak / amplitude).log10();
            let phase_deg = co.atan2(s).to_degrees();
            if !gain_db.is_finite() || !phase_deg.is_finite() {
                return Response::Unavailable(Unavailable::NonFinite);
            }
            Response::Available { gain_db, phase_deg }
        });
        points.push(Point {
            frequency_hz,
            channels,
        });
        progress(points.len());
    }
    // Integer pure-delay hypothesis, not general filter group delay. Require all
    // usable points to agree, search only the lowest-tone half-cycle. Acquisition
    // order must not change the diagnostic search range or the raw phase.
    let available: Vec<_> = points
        .iter()
        .filter_map(|p| match p.channels[c.input_channel] {
            Response::Available { gain_db, phase_deg } => {
                Some((p.frequency_hz, gain_db, phase_deg))
            }
            _ => None,
        })
        .collect();
    let delay_samples = if available.len() >= 2 {
        let lowest_hz = available.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let minimum_gain = available.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let limit = (c.sample_rate as f64 / (2. * lowest_hz)).floor() as usize;
        (0..=limit.min(48000))
            .find(|delay| {
                available.iter().all(|(f, gain, phase)| {
                    let error = (phase + 360. * f * *delay as f64 / c.sample_rate as f64 + 180.)
                        .rem_euclid(360.)
                        - 180.;
                    error.abs() <= 0.5 && (gain - minimum_gain).abs() <= 0.1
                })
            })
            .map(|d| d as f64)
    } else {
        None
    };
    Ok(Measurement { conditions: c.clone(), points, delay_samples, reset_supported,
        history_warning: "Reset when supported → fixed silent warm-up → settling/observation at each frequency; retain history between steps. Reset may retain hidden state; real-plugin repeats are not guaranteed.".into() })
}
