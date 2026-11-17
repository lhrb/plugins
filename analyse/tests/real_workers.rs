#![cfg(feature = "gui")]

//! Opt-in trusted-plugin adapter check, not a GUI test or a repeatability claim.
use clap_probe::comparison::{Outcome, RunId, RunState, Settings, Status};
use clap_probe::hosting::{Worker, WorkerError};
use clap_probe::measurement::{measure_with_progress, Conditions, Processor, Response};
use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(5);

struct Selection {
    path: PathBuf,
    id: String,
    parameter: u32,
    value: f64,
}
impl Selection {
    fn from_env(slot: &str) -> Self {
        let required = |suffix| {
            let name = format!("INSPECTOR_TEST_{slot}_{suffix}");
            env::var(&name).unwrap_or_else(|_| panic!("set {name}; see tests/README.md"))
        };
        let path = PathBuf::from(required("PATH"));
        assert!(
            path.is_absolute() && path.is_file(),
            "explicit absolute trusted binary required: {path:?}"
        );
        let id = required("ID");
        assert!(!id.is_empty(), "explicit plugin ID required");
        Self {
            path,
            id,
            parameter: required("PARAMETER")
                .parse()
                .expect("parameter ID must be u32"),
            value: required("VALUE")
                .parse()
                .expect("plain parameter value must be f64"),
        }
    }
    fn spawn(&self) -> Worker {
        Worker::spawn(
            std::path::Path::new(env!("CARGO_BIN_EXE_plugin-inspector")),
            &self.path,
            &self.id,
            TIMEOUT,
        )
        .unwrap_or_else(|error| panic!("load {} from {:?}: {error}", self.id, self.path))
    }
    fn settings(&self, worker: &Worker) -> Settings {
        Settings {
            identity: format!("{} ({})", self.id, self.path.display()),
            parameters: writable_values(worker),
        }
    }
    fn change_parameter(&self, worker: &mut Worker) {
        let parameter = worker
            .parameters
            .iter()
            .find(|p| p.id == self.parameter)
            .expect("explicit parameter ID must be exposed");
        assert!(!parameter.readonly, "selected parameter must be writable");
        assert!(self.value.is_finite() && (parameter.min..=parameter.max).contains(&self.value));
        assert!(
            (parameter.value - self.value).abs() > 1e-6,
            "choose a VALUE different from the loaded value to exercise a real edit"
        );
        worker
            .set(self.parameter, self.value)
            .expect("apply exposed setting");
        let readback = worker
            .parameters
            .iter()
            .find(|p| p.id == self.parameter)
            .unwrap()
            .value;
        assert!(
            (readback - self.value).abs() <= 1e-6,
            "{} parameter {}: requested {}, read back {readback}",
            self.id,
            self.parameter,
            self.value
        );
    }
}

fn writable_values(worker: &Worker) -> Vec<(u32, f64)> {
    worker
        .parameters
        .iter()
        .filter(|p| !p.readonly)
        .map(|p| (p.id, p.value))
        .collect()
}

fn complete_slot(state: &mut RunState, run: RunId, slot: usize, worker: &mut Worker) {
    let conditions = state.view().snapshot.unwrap().conditions;
    let mut reported = Vec::new();
    let measurement = measure_with_progress(&conditions, worker, |completed| {
        reported.push(completed);
        state.progress(run, slot, completed);
        assert_eq!(state.view().progress[slot], completed);
        assert_eq!(state.view().status, Status::Running);
    })
    .expect("real worker completes shared sweep");
    assert_eq!(
        reported,
        (1..=conditions.frequencies_hz.len()).collect::<Vec<_>>()
    );
    assert_eq!(measurement.conditions, conditions);
    assert_eq!(
        measurement
            .points
            .iter()
            .map(|p| p.frequency_hz)
            .collect::<Vec<_>>(),
        conditions.frequencies_hz
    );
    assert!(!measurement.history_warning.trim().is_empty());
    for point in &measurement.points {
        for response in &point.channels {
            if let Response::Available { gain_db, phase_deg } = response {
                assert!(gain_db.is_finite() && phase_deg.is_finite(), "{point:?}");
            }
        }
    }
    // Unavailable fundamentals are legitimate real-plugin outcomes, not zeroes.
    state.finish(run, slot, Outcome::Complete(measurement));
}

#[test]
#[ignore = "loads explicitly trusted CLAP binaries and injects abort/hang; see tests/README.md"]
fn loaded_pair_keeps_settings_and_completed_results_across_worker_crash_and_timeout() {
    // Read every required selection before launching any plugin code.
    let selections = [Selection::from_env("A"), Selection::from_env("B")];
    let mut a = selections[0].spawn();
    let mut b = selections[1].spawn();
    let untouched_b = writable_values(&b);
    selections[0].change_parameter(&mut a);
    b.info().expect("B remains loaded while A changes");
    assert_eq!(
        writable_values(&b),
        untouched_b,
        "A's edit must not change B"
    );
    let changed_a = writable_values(&a);
    selections[1].change_parameter(&mut b);
    a.info().expect("A remains loaded while B changes");
    assert_eq!(writable_values(&a), changed_a, "B's edit must not change A");

    let mut state = RunState::default();
    state.set_conditions(Conditions::default()); // Generic reset, fixed silence, retained-history steps.
    state.set_slot(0, selections[0].settings(&a));
    state.set_slot(1, selections[1].settings(&b));
    let run = state.start().unwrap();
    complete_slot(&mut state, run, 0, &mut a);
    assert_eq!(state.view().status, Status::Running);
    complete_slot(&mut state, run, 1, &mut b);
    assert_eq!(state.view().status, Status::Success);
    assert_eq!(
        state.view().progress,
        [Conditions::default().frequencies_hz.len(); 2]
    );

    for timeout in [false, true] {
        if timeout {
            // Only replace the failed slot; A stays loaded for the whole test.
            b = selections[1].spawn();
            state.set_slot(1, selections[1].settings(&b));
        }
        let run = state.start().unwrap();
        complete_slot(&mut state, run, 0, &mut a);
        let before_fault = state.view();
        let started = Instant::now();
        let error = b
            .verify_fault(timeout)
            .expect_err("injected fault cannot succeed");
        assert!(
            started.elapsed() < TIMEOUT + Duration::from_secs(5),
            "fault must return within a bounded deadline"
        );
        let failure = match error {
            WorkerError::Timeout => {
                assert!(timeout, "crash must be distinguished from timeout");
                Outcome::Timeout
            }
            WorkerError::Failure(message) => {
                assert!(!timeout, "hang must report timeout, got {message}");
                assert!(!message.is_empty());
                Outcome::PluginFailure(message)
            }
        };
        state.finish(run, 1, failure.clone());
        let failed = state.view();
        assert_eq!(failed.status, Status::PartialFailure);
        assert_eq!(
            failed.snapshot, before_fault.snapshot,
            "retain identities/settings/conditions"
        );
        assert_eq!(
            failed.outcomes[0], before_fault.outcomes[0],
            "retain surviving completed measurement"
        );
        assert_eq!(failed.outcomes[1], Some(failure));
        a.info()
            .expect("surviving worker remains responsive after peer fault");
        assert_eq!(
            writable_values(&a),
            changed_a,
            "surviving settings remain intact"
        );
        let mut output = [[f64::NAN; 2]; 256];
        a.process(&[[0.0; 2]; 256], &mut output)
            .expect("surviving worker still processes audio");
        assert!(output.iter().flatten().all(|sample| sample.is_finite()));
        assert_eq!(
            state.view(),
            failed,
            "worker liveness check cannot discard results"
        );
    }
    a.shutdown().expect("surviving worker shuts down cleanly");
}
