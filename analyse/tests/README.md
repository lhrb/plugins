# Tests

Run from the repository root with dependencies cached for `--offline`.

## Headless checks

```sh
cargo test --manifest-path analyse/Cargo.toml --locked --offline --lib --test measurement --test run_state
```

`measurement.rs` covers DSP accuracy, routing, validation, reset/history behavior,
and numerical edge cases.
`run_state.rs` covers snapshots, stale results, cancellation, progress, and partial failures.
Neither requires GUI or plugin binaries.
See the tests for assertions and tolerances, and the [feature plan](../../docs/features/plugin-inspector/plan.md) for agreed scope.

## Opt-in CLAP integration

`real_workers.rs` loads two explicitly selected trusted binaries and injects a
worker crash and timeout.
Worker isolation protects availability, not filesystem access or security.
The test is ignored by default and requires `gui` to build the worker binary.
Without `gui`, this target contains zero tests; that is not CLAP verification.

Set all eight environment variables below.
Replace example paths, IDs, and parameter values for your plugins.
Choose exposed writable parameter IDs and valid plain values different from their
loaded values; the test verifies readback.
No plugin discovery or fallback paths are used.

```sh
(
  ulimit -c 0 # Fault injection aborts a worker; disable core dumps.
  export INSPECTOR_TEST_A_PATH="/absolute/path/to/oxide-tape.clap"
  export INSPECTOR_TEST_A_ID="com.tapeworks.oxide"
  export INSPECTOR_TEST_A_PARAMETER="1141971201"
  export INSPECTOR_TEST_A_VALUE="0.4"
  export INSPECTOR_TEST_B_PATH="/usr/lib/clap/CHOWTapeModel.clap"
  export INSPECTOR_TEST_B_ID="org.chowdsp.CHOWTapeModel"
  export INSPECTOR_TEST_B_PARAMETER="3188564941"
  export INSPECTOR_TEST_B_VALUE="0.4"
  cargo test --manifest-path analyse/Cargo.toml --locked --offline --features gui \
    --test real_workers -- \
    --ignored --exact loaded_pair_keeps_settings_and_completed_results_across_worker_crash_and_timeout \
    --nocapture
)
```

The test checks independent parameter edits, a shared sweep, progress, and retention
of the other slot's completed result and usable worker after each fault.
It uses the shared reset/silent-warm-up procedure and retains history between frequencies.
It does not establish real-plugin repeatability or analytical DSP accuracy.

## Coverage limits

These tests do not verify GPUI rendering, responsiveness, visible labels/plots,
or GUI cancellation and run-state wiring.
A two-real-plugin GUI walkthrough remains necessary to verify those behaviors.
