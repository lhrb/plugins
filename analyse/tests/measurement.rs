use clap_probe::measurement::*;
use std::f64::consts::TAU;

struct Fixture {
    matrix: [[f64; 2]; 2],
    delay: usize,
    filter: bool,
    harmonic: bool,
    drift: bool,
    nonfinite: bool,
    supports_reset: bool,
    resets: usize,
    calls: Vec<usize>,
    received: Vec<[f64; 2]>,
    memory: [f64; 2],
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            matrix: [[1.0, 0.0], [0.0, 1.0]],
            delay: 0,
            filter: false,
            harmonic: false,
            drift: false,
            nonfinite: false,
            supports_reset: true,
            resets: 0,
            calls: vec![],
            received: vec![],
            memory: [0.0; 2],
        }
    }
}
impl Processor for Fixture {
    fn reset(&mut self) -> bool {
        self.resets += 1;
        if self.supports_reset {
            self.memory = [0.0; 2];
            self.received.clear();
        }
        self.supports_reset
    }
    fn process(&mut self, input: &[[f64; 2]], output: &mut [[f64; 2]]) -> Result<(), String> {
        assert_eq!(input.len(), output.len());
        self.calls.push(input.len());
        for (frame, out) in input.iter().zip(output) {
            self.received.push(*frame);
            let n = self.received.len() - 1;
            let delayed = n
                .checked_sub(self.delay)
                .map(|i| self.received[i])
                .unwrap_or([0.0; 2]);
            for (ch, sample) in out.iter_mut().enumerate() {
                let mut y = self.matrix[ch][0] * delayed[0] + self.matrix[ch][1] * delayed[1];
                if self.filter {
                    y = 0.2 * y + 0.8 * self.memory[ch];
                    self.memory[ch] = y;
                }
                if self.harmonic {
                    y += 2.0 * y * y; // DC and second harmonic, unchanged fundamental.
                }
                if self.drift {
                    y *= 1.0 + n as f64 / 4800.0;
                }
                *sample = if self.nonfinite { f64::NAN } else { y };
            }
        }
        Ok(())
    }
}
fn response(point: &Point, ch: usize) -> (f64, f64) {
    match point.channels[ch] {
        Response::Available { gain_db, phase_deg } => {
            assert!(gain_db.is_finite() && phase_deg.is_finite(), "{point:?}");
            (gain_db, phase_deg)
        }
        ref other => panic!(
            "expected usable response at {} Hz channel {ch}: {other:?}",
            point.frequency_hz
        ),
    }
}
fn assert_response(point: &Point, ch: usize, gain_db: f64, phase_deg: f64) {
    let (gain, phase) = response(point, ch);
    let phase_error = (phase - phase_deg + 180.0).rem_euclid(360.0) - 180.0;
    assert!(
        (gain - gain_db).abs() <= 0.1,
        "{} Hz ch {ch}: gain {gain}, expected {gain_db}",
        point.frequency_hz
    );
    assert!(
        phase_error.abs() <= 1.0,
        "{} Hz ch {ch}: phase {phase}, expected {phase_deg}",
        point.frequency_hz
    );
}
fn sweep(fixture: &mut Fixture, conditions: &Conditions) -> Measurement {
    let result = measure(conditions, fixture).expect("valid fixture sweep");
    assert_eq!(
        result.conditions, *conditions,
        "retain actual conditions without normalization"
    );
    assert_eq!(
        result
            .points
            .iter()
            .map(|p| p.frequency_hz)
            .collect::<Vec<_>>(),
        conditions.frequencies_hz
    );
    result
}

#[test]
fn identity_gain_and_polarity_keep_absolute_level_and_phase() {
    for gain in [1.0, 0.25, -0.5] {
        let mut fixture = Fixture {
            matrix: [[gain, 0.0], [0.0, gain]],
            ..Default::default()
        };
        let result = sweep(&mut fixture, &Conditions::default());
        for point in &result.points {
            assert_response(
                point,
                0,
                20.0 * gain.abs().log10(),
                if gain < 0.0 { 180.0 } else { 0.0 },
            );
            assert_eq!(
                point.channels[1],
                Response::Unavailable(Unavailable::BelowFloor),
                "no phase at a null"
            );
        }
    }
}

#[test]
fn delay_recovery_is_independent_of_frequency_order_and_keeps_raw_phase() {
    for frequencies_hz in [
        vec![375.0, 997.0, 6000.0],
        vec![6000.0, 997.0, 375.0],
        vec![997.0, 375.0, 6000.0],
    ] {
        let conditions = Conditions {
            frequencies_hz,
            ..Default::default()
        };
        let result = sweep(
            &mut Fixture {
                delay: 17,
                ..Default::default()
            },
            &conditions,
        );
        // Check phase before delay recovery so a search failure cannot hide compensation.
        for point in &result.points {
            assert_response(
                point,
                0,
                0.0,
                -360.0 * point.frequency_hz * 17.0 / conditions.sample_rate as f64,
            );
        }
        assert!(
            result
                .delay_samples
                .is_some_and(|delay| (delay - 17.0).abs() <= 1.0),
            "same pure delay must be recovered in supplied order {:?}: {:?}",
            conditions.frequencies_hz,
            result.delay_samples
        );
    }
}

#[test]
fn near_nyquist_responses_are_accurate_or_explicitly_unavailable() {
    let conditions = Conditions {
        // First tone has eight beat cycles in each observation half. The others
        // progressively lose that resolution, including the closest legal f64.
        frequencies_hz: vec![
            23_953.125,
            23_999.0,
            23_999.99,
            23_999.999_999,
            f64::from_bits(24_000.0_f64.to_bits() - 1),
        ],
        ..Default::default()
    };
    for filter in [false, true] {
        let result = sweep(
            &mut Fixture {
                delay: if filter { 0 } else { 17 },
                filter,
                ..Default::default()
            },
            &conditions,
        );
        for (index, point) in result.points.iter().enumerate() {
            let w = TAU * point.frequency_hz / conditions.sample_rate as f64;
            let (gain, phase) = if filter {
                let real = 1.0 - 0.8 * w.cos();
                let imag = 0.8 * w.sin();
                (
                    20.0 * (0.2 / real.hypot(imag)).log10(),
                    -imag.atan2(real).to_degrees(),
                )
            } else {
                (
                    0.0,
                    -360.0 * point.frequency_hz * 17.0 / conditions.sample_rate as f64,
                )
            };
            // A resolved stationary control must remain usable. Unresolved tones
            // may be unavailable, but available values still owe the same accuracy.
            if index == 0 || matches!(point.channels[0], Response::Available { .. }) {
                assert_response(point, 0, gain, phase);
            }
        }
    }
}

#[test]
fn one_pole_filter_matches_analytical_complex_response() {
    let conditions = Conditions::default();
    let result = sweep(
        &mut Fixture {
            filter: true,
            ..Default::default()
        },
        &conditions,
    );
    for point in &result.points {
        let w = TAU * point.frequency_hz / conditions.sample_rate as f64;
        let real = 1.0 - 0.8 * w.cos();
        let imag = 0.8 * w.sin();
        assert_response(
            point,
            0,
            20.0 * (0.2 / real.hypot(imag)).log10(),
            -imag.atan2(real).to_degrees(),
        );
    }
}

#[test]
fn independent_excitation_resolves_each_direct_and_cross_channel_path() {
    for input_channel in 0..2 {
        let conditions = Conditions {
            input_channel,
            ..Default::default()
        };
        let mut fixture = Fixture {
            matrix: [[0.5, -0.25], [0.125, 0.75]],
            ..Default::default()
        };
        let result = sweep(&mut fixture, &conditions);
        assert!(fixture.received.iter().all(|x| x[1 - input_channel] == 0.0));
        for point in &result.points {
            for ch in 0..2 {
                let gain = fixture.matrix[ch][input_channel];
                assert_response(
                    point,
                    ch,
                    20.0 * gain.abs().log10(),
                    if gain < 0.0 { 180.0 } else { 0.0 },
                );
            }
        }
    }
}

#[test]
fn fundamental_measurement_does_not_confuse_harmonics_or_dc_with_gain() {
    let conditions = Conditions {
        level_dbfs_peak: -12.0,
        ..Default::default()
    };
    let result = sweep(
        &mut Fixture {
            harmonic: true,
            ..Default::default()
        },
        &conditions,
    );
    for point in &result.points {
        assert_response(point, 0, 0.0, 0.0);
    }
}

#[test]
fn reset_then_fixed_silent_warmup_preserves_history_within_sweep_and_repeats() {
    let conditions = Conditions::default();
    let mut a = Fixture {
        filter: true,
        ..Default::default()
    };
    let first = sweep(&mut a, &conditions);
    assert_eq!(a.resets, 1, "never reset between frequencies");
    let expected_frames = conditions.warmup_frames
        + conditions.frequencies_hz.len()
            * (conditions.settling_frames + conditions.observation_frames);
    assert_eq!(
        a.received.len(),
        expected_frames,
        "one fixed warmup, no hidden history restoration"
    );
    assert!(a.received[..conditions.warmup_frames]
        .iter()
        .all(|x| *x == [0.0; 2]));
    assert!(a.calls.iter().all(|n| *n == conditions.block_frames));
    let amplitude = 10_f64.powf(conditions.level_dbfs_peak / 20.0);
    assert!(a.received.iter().all(|x| x[0].abs() <= amplitude + 1e-12));
    assert!(a.received.iter().any(|x| x[0].abs() > 0.99 * amplitude));
    let input = a.received.clone();
    let second = sweep(&mut a, &conditions);
    assert_eq!(a.resets, 2);
    assert_eq!(
        a.received, input,
        "deterministic repeat uses identical excitation"
    );
    for (x, y) in first.points.iter().zip(&second.points) {
        let (gx, px) = response(x, 0);
        let (gy, py) = response(y, 0);
        assert!((gx - gy).abs() <= 1e-6 && (px - py).abs() <= 1e-6);
    }
    let mut b = Fixture {
        filter: true,
        ..Default::default()
    };
    sweep(&mut b, &conditions);
    assert_eq!(
        a.received, b.received,
        "separate instances receive identical input, not the other's output"
    );
    assert!(first.reset_supported);
    assert!(
        !first.history_warning.trim().is_empty(),
        "disclose retained-history dependence even after reset"
    );
}

#[test]
fn unsupported_reset_is_disclosed_without_rejecting_a_usable_sweep() {
    let mut fixture = Fixture {
        supports_reset: false,
        ..Default::default()
    };
    let result = sweep(&mut fixture, &Conditions::default());
    assert_eq!(fixture.resets, 1);
    assert!(!result.reset_supported);
    assert!(!result.history_warning.trim().is_empty());
    assert_response(&result.points[0], 0, 0.0, 0.0);
}

#[test]
fn invalid_conditions_are_rejected_before_touching_processor() {
    let base = Conditions::default();
    let mut invalid = vec![];
    for frequency in [
        0.0,
        -1.0,
        24_000.0,
        f64::from_bits(24_000.0_f64.to_bits() + 1),
        24_001.0,
        f64::NAN,
        f64::INFINITY,
    ] {
        invalid.push(Conditions {
            frequencies_hz: vec![frequency],
            ..base.clone()
        });
    }
    invalid.push(Conditions {
        frequencies_hz: vec![],
        ..base.clone()
    });
    for level in [1.0, f64::NEG_INFINITY, f64::NAN] {
        invalid.push(Conditions {
            level_dbfs_peak: level,
            ..base.clone()
        });
    }
    invalid.extend([
        Conditions {
            settling_frames: 0,
            ..base.clone()
        },
        Conditions {
            observation_frames: 0,
            ..base.clone()
        },
        Conditions {
            warmup_frames: 0,
            ..base.clone()
        },
        Conditions {
            observation_frames: 1,
            ..base.clone()
        },
        Conditions {
            input_channel: 2,
            ..base.clone()
        },
        Conditions {
            sample_rate: 44_100,
            ..base.clone()
        },
        Conditions {
            block_frames: 128,
            ..base
        },
    ]);
    for conditions in invalid {
        let mut fixture = Fixture::default();
        assert_eq!(
            measure(&conditions, &mut fixture),
            Err(MeasureError::InvalidConfiguration),
            "{conditions:?}"
        );
        assert_eq!(fixture.resets, 0, "validate before reset");
        assert!(fixture.received.is_empty());
    }
}

#[test]
fn null_subfloor_nonfinite_and_drifting_outputs_are_unavailable_not_fabricated() {
    for (mut fixture, reason) in [
        (
            Fixture {
                matrix: [[0.0; 2]; 2],
                ..Default::default()
            },
            Unavailable::BelowFloor,
        ),
        (
            Fixture {
                matrix: [[1e-7, 0.0], [0.0, 1e-7]],
                ..Default::default()
            },
            Unavailable::BelowFloor,
        ),
        (
            Fixture {
                nonfinite: true,
                ..Default::default()
            },
            Unavailable::NonFinite,
        ),
        (
            Fixture {
                drift: true,
                ..Default::default()
            },
            Unavailable::Unstable,
        ),
    ] {
        let result = sweep(&mut fixture, &Conditions::default());
        for point in result.points {
            assert_eq!(
                point.channels[0],
                Response::Unavailable(reason.clone()),
                "{point:?}"
            );
        }
    }
    // A quiet but usable output must not be hidden by an excessively high floor.
    let result = sweep(
        &mut Fixture {
            matrix: [[0.01, 0.0], [0.0, 0.01]],
            ..Default::default()
        },
        &Conditions::default(),
    );
    for point in result.points {
        assert_response(&point, 0, -40.0, 0.0); // -76 dBFS output, above the declared -100 dBFS floor.
    }
}

#[test]
fn processor_errors_are_not_successful_measurements() {
    struct Broken;
    impl Processor for Broken {
        fn reset(&mut self) -> bool {
            true
        }
        fn process(&mut self, _: &[[f64; 2]], _: &mut [[f64; 2]]) -> Result<(), String> {
            Err("fixture process failed".into())
        }
    }
    assert_eq!(
        measure(&Conditions::default(), &mut Broken),
        Err(MeasureError::ProcessorFailure(
            "fixture process failed".into()
        ))
    );
}
