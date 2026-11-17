use clap_probe::comparison::*;
use clap_probe::measurement::*;

fn settings(identity: &str, value: f64) -> Settings {
    Settings {
        identity: identity.into(),
        parameters: vec![(17, value)],
    }
}
fn configured() -> RunState {
    let mut session = RunState::default();
    session.set_slot(0, settings("OXIDE", 0.25));
    session.set_slot(1, settings("reference", 0.75));
    session.set_conditions(Conditions::default());
    session
}
fn result(gain_db: f64) -> Outcome {
    Outcome::Complete(Measurement {
        conditions: Conditions::default(),
        points: vec![Point {
            frequency_hz: 997.0,
            channels: [
                Response::Available {
                    gain_db,
                    phase_deg: -13.0,
                },
                Response::Unavailable(Unavailable::BelowFloor),
            ],
        }],
        delay_samples: Some(2.0),
        reset_supported: true,
        history_warning: "Frequency steps retain history".into(),
    })
}
fn complete(session: &mut RunState, run: RunId) {
    session.finish(run, 0, result(-3.0));
    session.finish(run, 1, result(-6.0));
    assert_eq!(session.view().status, Status::Success);
}

#[test]
fn start_snapshots_independent_settings_and_conditions_and_reports_progress() {
    let mut session = configured();
    let run = session.start().unwrap();
    let initial = session.view();
    assert_eq!(initial.run, Some(run));
    assert_eq!(initial.status, Status::Running);
    assert_eq!(initial.outcomes, [None, None]);
    assert_eq!(initial.stale, [false, false]);
    assert_eq!(initial.progress, [0, 0]);
    assert_eq!(
        initial.snapshot,
        Some(Snapshot {
            conditions: Conditions::default(),
            slots: [settings("OXIDE", 0.25), settings("reference", 0.75)],
        })
    );
    session.progress(run, 0, 1);
    session.progress(run, 0, 0); // Late progress must not move backwards.
    assert_eq!(session.view().progress, [1, 0]);
    session.set_slot(0, settings("OXIDE", 0.5));
    assert_eq!(
        session.view().snapshot,
        initial.snapshot,
        "edits cannot rewrite a running snapshot"
    );
    assert_eq!(session.view().stale, [true, false]);
    session.finish(run, 0, result(-3.0));
    assert_eq!(
        session.view().status,
        Status::Running,
        "one result is not a completed comparison"
    );
    session.finish(run, 1, result(-6.0));
    assert_eq!(
        session.view().outcomes,
        [Some(result(-3.0)), Some(result(-6.0))]
    );
    assert_eq!(
        session.view().stale,
        [true, false],
        "late completion cannot clear an edit's stale marker"
    );
}

#[test]
fn slot_edits_stale_only_that_result_shared_edits_stale_both_and_new_run_uses_edits() {
    let mut session = configured();
    let run = session.start().unwrap();
    complete(&mut session, run);
    let finished = session.view();
    session.set_slot(1, settings("reference", 0.9));
    assert_eq!(session.view().stale, [false, true]);
    assert_eq!(
        session.view().outcomes,
        finished.outcomes,
        "keep stale results readable"
    );
    assert_eq!(session.view().snapshot, finished.snapshot);
    let changed = Conditions {
        level_dbfs_peak: -24.0,
        input_channel: 1,
        ..Default::default()
    };
    session.set_conditions(changed.clone());
    assert_eq!(session.view().stale, [true, true]);
    assert_eq!(session.view().outcomes, finished.outcomes);
    let next = session.start().unwrap();
    assert_ne!(run, next);
    assert_eq!(
        session.view().snapshot,
        Some(Snapshot {
            conditions: changed,
            slots: [settings("OXIDE", 0.25), settings("reference", 0.9)],
        })
    );
    assert_eq!(session.view().stale, [false, false]);
}

#[test]
fn cancel_retains_identified_partial_result_and_ignores_all_late_events() {
    let mut session = configured();
    let run = session.start().unwrap();
    session.finish(run, 0, result(-3.0));
    session.cancel();
    let cancelled = session.view();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.outcomes, [Some(result(-3.0)), None]);
    assert!(cancelled.snapshot.is_some());
    session.finish(run, 1, result(-6.0));
    session.progress(run, 1, 3);
    assert_eq!(session.view(), cancelled);
    let next = session.start().unwrap();
    assert_ne!(next, run);
    let current = session.view();
    session.finish(run, 0, Outcome::Timeout);
    assert_eq!(
        session.view(),
        current,
        "cancelled old run cannot overwrite a new run"
    );
}

#[test]
fn superseded_results_progress_and_duplicate_completion_cannot_replace_newer_results() {
    let mut session = configured();
    let old = session.start().unwrap();
    let current = session.start().unwrap();
    assert_ne!(old, current);
    session.progress(current, 1, 1);
    let before = session.view();
    session.finish(old, 0, result(90.0));
    session.finish(old, 1, Outcome::Timeout);
    session.progress(old, 0, 3);
    assert_eq!(session.view(), before);
    complete(&mut session, current);
    let completed = session.view();
    session.finish(old, 0, result(80.0));
    session.finish(current, 0, result(70.0));
    assert_eq!(session.view(), completed, "first terminal result wins");
}

#[test]
fn plugin_failure_and_timeout_preserve_the_other_result_in_either_completion_order() {
    for failure in [
        Outcome::PluginFailure("worker exited".into()),
        Outcome::Timeout,
    ] {
        for failed_slot in 0..2 {
            for failure_first in [true, false] {
                let mut session = configured();
                let run = session.start().unwrap();
                let good_slot = 1 - failed_slot;
                if failure_first {
                    session.finish(run, failed_slot, failure.clone());
                    assert_ne!(session.view().status, Status::Success);
                    session.finish(run, good_slot, result(-3.0));
                } else {
                    session.finish(run, good_slot, result(-3.0));
                    assert_eq!(session.view().status, Status::Running);
                    session.finish(run, failed_slot, failure.clone());
                }
                let view = session.view();
                assert_eq!(view.status, Status::PartialFailure);
                assert_eq!(view.outcomes[good_slot], Some(result(-3.0)));
                assert_eq!(view.outcomes[failed_slot], Some(failure.clone()));
                assert_eq!(
                    view.snapshot.unwrap().slots,
                    [settings("OXIDE", 0.25), settings("reference", 0.75)]
                );
            }
        }
    }
}

#[test]
fn invalid_run_configuration_is_distinct_from_cancellation_and_plugin_failure() {
    let mut session = configured();
    session.set_conditions(Conditions {
        frequencies_hz: vec![24_000.0],
        ..Default::default()
    });
    assert_eq!(session.start(), Err(MeasureError::InvalidConfiguration));
    let view = session.view();
    assert_eq!(view.status, Status::InvalidConfiguration);
    assert_eq!(view.outcomes, [None, None]);
}
