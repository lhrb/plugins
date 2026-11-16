# Slice 1: two-plugin CLAP hosting experiment

## Checkpoint outcome

**The selected Linux CLAP hosting route works for both real binaries at the tested configuration.**
Both plugins remained loaded in independent workers and processed identical generated input, never each other's output.
Parameter edits took effect and left the other slot's complete exposed-parameter snapshot unchanged.
Matched-history controls produced bit-identical continuations after edits/reset in the other slot.
The parent retained a completed reference result across an injected worker crash and configured timeout.

**CHOWTape `reset()` plus the tested preconditioning does not reproduce an arbitrary preceding trial within `1e-6`.**
Fresh instances with matched command histories did reproduce exactly in this session.
This establishes the chosen mechanism, not complete state restoration, general plugin compatibility, or a future application's reset policy.
Stop here; no UI, response-measurement slice, alternative plugin, or format was implemented/run.
Slice 1 is Done in `plan.md`; slice 2 is dependency-ready but has not started.
Agree on its reset/preconditioning policy before implementation; this experiment does not establish arbitrary-history restoration.

## Reproducible identities and prerequisites

- Repository source baseline: `155fdf6490fd38669d59b851d6ccc98469a93fc7`; existing user plan changes were preserved.
- OS: `Linux 7.2.15-rt3-arch1-1-rt x86_64 GNU/Linux`.
- Toolchain: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`; `cargo 1.99.1 (5f94df478 2026-08-27)`.
- OXIDE: `/home/lukas/workspace/plugins/tape/target/bundled/oxide-tape.clap`, explicit ID `com.tapeworks.oxide`, descriptor version `0.1.0`, CLAP `1.2.2`.
- CHOWTape: `/usr/lib/clap/CHOWTapeModel.clap`, explicit ID `org.chowdsp.CHOWTapeModel`, descriptor version `2.11.4`, CLAP `1.1.8`; installed package `chowtapemodel-bin 2.11.4-1`.
- Both binaries existed after the build; `ldd` reported no missing libraries.
- OXIDE SHA-256: `c7596d8d21fcb1edcb3db70ad1bee8dc3a4c48012037f9e54bac30feb5ab9f4f`.
- CHOWTape SHA-256: `3b0ca8c063b5d37a7da421ad3f1dde88a6c9d229ef4e4234dfdadebf8651cbf3`.
- `analyse/Cargo.lock` SHA-256: `48ac70fe1199f3cd0f2f32fb858e37057e449150d8c7f304c208284bc2abe212`.
- ABI bindings: `clap-sys 0.5.0`, pinned git revision `25d7f53fdb6363ad63fbd80049cb7a42a97ac156`; loader `libloading 0.8.9`.

Built with the existing xtask; `tape/` source and lock remained unchanged.
The xtask also generated VST3 as its normal side effect; no VST3 was loaded.
The build succeeded with the known upstream NIH-plug VST3 future-incompatibility macro warnings.
No privileged installs or added dependencies in `tape/` were needed.

## Exact commands

Run from `/home/lukas/workspace/plugins`:

```sh
(cd tape && cargo xtask bundle oxide-tape --release)
cargo build --manifest-path analyse/Cargo.toml --offline
analyse/target/debug/clap-probe inspect \
  /home/lukas/workspace/plugins/tape/target/bundled/oxide-tape.clap \
  com.tapeworks.oxide 5000 > /tmp/oxide-inspect.log 2>&1
analyse/target/debug/clap-probe inspect \
  /usr/lib/clap/CHOWTapeModel.clap org.chowdsp.CHOWTapeModel 5000 \
  > /tmp/chow-inspect.log 2>&1

cargo fmt --manifest-path analyse/Cargo.toml
cargo test --manifest-path analyse/Cargo.toml --locked --offline
cargo check --manifest-path analyse/Cargo.toml --locked --offline
cargo clippy --manifest-path analyse/Cargo.toml --locked --offline --all-targets -- -D warnings
cargo fmt --manifest-path analyse/Cargo.toml -- --check
cargo build --manifest-path analyse/Cargo.toml --locked --offline
mkdir -p analyse/runs/final
(ulimit -c 0; analyse/target/debug/clap-probe run \
  /home/lukas/workspace/plugins/tape/target/bundled/oxide-tape.clap \
  com.tapeworks.oxide 1141971201 0.4 \
  /usr/lib/clap/CHOWTapeModel.clap \
  org.chowdsp.CHOWTapeModel 3188564941 0.4 \
  analyse/runs/final 5000 > analyse/runs/final/transcript.log 2>&1)
```

Earlier exploratory runs used `analyse/runs/first` and `analyse/runs/verified` with the same plugin arguments.
They exposed a mismatched control history: the reference primary had preceding trials that its fresh control lacked.
Final evidence mirrors those trials and retains a monotonic host sample timeline across reset.
Only the final transcript and final hashes support the results below.

Tracked evidence: [`analyse/evidence/slice1.log`](../../../analyse/evidence/slice1.log) contains full descriptors, all 6 OXIDE and 52 CHOWTape parameter definitions, actual value snapshots, edits, latencies, comparisons, and recovery output.
[`analyse/evidence/SHA256SUMS`](../../../analyse/evidence/SHA256SUMS) records binary/source/input/output/transcript hashes.
Raw input/output files remain locally under ignored `analyse/runs/final/`; they are not committed.
Each output contains 262,144 bytes; the input including preconditioning contains 393,216 bytes.

## Lifecycle, threading, and signal configuration

The parent never loads a plugin library and performs no directory discovery.
Each worker opens only its explicit absolute binary path, enumerates that container's descriptors, and requires the exact requested ID.
It performs entry init → factory/create → plugin init → port/parameter queries → offline render mode → activate → start processing.
Normal shutdown performs stop processing → deactivate → destroy → entry deinit → library unload before acknowledging completion.
Failed initialization tears down completed lifecycle stages.
The parent's response deadline covers initialization, processing, and teardown.
Injected fatal failures necessarily bypass graceful teardown; the OS reclaims that worker.

All main-thread calls use the worker's original main OS thread.
Audio calls temporarily mark that same thread as the symbolic audio thread; none overlap for an instance.
The thread-check extension reports these roles.
Plugin callback requests use atomic flags; the main thread services them between blocks and at 10 ms idle intervals.
Restart requests fail explicitly as unsupported rather than silently processing with stale configuration.

Both plugins reported one stereo main input bus and one stereo main output bus.
Both accepted offline render mode and activation at **48,000 Hz, fixed 256-frame blocks, float32, separate input/output buffers**.
No audio device, transport, GUI, or automatic parameter mapping was involved.
The host sample counter increases through every processed block, including parameter-application blocks; DSP reset does not rewind it.

For reset trials:

1. Call CLAP `reset()` on the active instance without changing parameters.
2. Precondition with 64 blocks / 16,384 frames / 0.341333 seconds of the generated input.
3. Record 128 blocks / 32,768 frames / 0.682667 seconds of its continuation.
4. Reject unwritten/nonfinite output and assert the complete exposed-parameter snapshot remains unchanged.

Input is `0.1 * sin(2π f n / 48000)`, with `f=997 Hz` left and `1499 Hz` right, converted to float32.
The level convention is 0.1 peak / −20 dBFS peak per channel, not normalized output.
All workers generate the same indexed samples from the same Rust function.
Input SHA-256 is `248fcb2ced3d622cd3822dc4f57920425999936cd8660b754968286f671c2b57`.
Continuation trials perform **no reset and no additional preconditioning** and replay the recorded input segment beginning at index 16,384 while retaining preceding DSP history.
The timeline remains monotonic even when the input segment repeats.

## Parameter and latency observations

`get_info` defaults are not assumed to be current settings.
CHOWTape's queried current Tape Mode was `2`, although its advertised default was `0`.
Its current Tape Speed was `0.5`, although its advertised default was approximately `0.74810147`.
The transcript records every actual setting at initialization/render boundaries; local preferences/presets may affect another machine's initialization.
No preset or state file was loaded by the host.

| Plugin | Explicit edit | Readback | Plugin-formatted value | Reported latency |
| --- | --- | --- | --- | --- |
| OXIDE | Output `1141971201`: `0.600000023842` → requested `0.4` | `0.4000000059604645` | −12.0 dB (initial −6 dB by mapping) | 0 samples |
| CHOWTape | Output Gain `3188564941`: `0.5` → requested `0.4` | `0.4000000059604645` | −6.00 dB (initial 0 dB by mapping) | 39 samples / 0.8125 ms |

These are plain CLAP values in each plugin's exposed 0–1 range, not a host-invented dB conversion.
Each change uses a global `CLAP_EVENT_PARAM_VALUE` at frame 0 in one 256-frame zero-input process call.
Readback must match the request within `1e-6` before proceeding.
A subsequent reset/preconditioning trial measures its effect after smoothing settles.
Latency remained 0/39 at every queried render boundary, including edited settings.
Raw timing was retained; no latency compensation, normalization, or cross-plugin equivalence calculation was applied.

## Real-plugin mechanism evidence

Declare **absolute per-sample peak difference ≤ `1e-6`** for repetition/control equivalence; RMS is diagnostic.
Comparisons combine the interleaved channel samples without averaging channels away.
Parameter-effect and history-sensitivity rows should differ; their `within=false` log labels are not failed repeatability checks.

| Comparison | Peak difference | RMS difference | Interpretation |
| --- | ---: | ---: | --- |
| OXIDE reset trial 1 vs 2 | 0 | 0 | Exact reset repeat |
| CHOWTape reset trial 1 vs 2 | 4.936009645462e−5 | 1.351646322678e−5 | Exceeds tolerance; reset/preconditioning insufficient |
| CHOWTape primary vs fresh control, first trial | 0 | 0 | Exact fresh-instance reproduction |
| CHOWTape primary vs control, second trial with matched history | 0 | 0 | Exact matched-history reproduction |
| CHOWTape matched-history baseline before intervention | 0 | 0 | Equal observed starting histories |
| CHOWTape continuation after OXIDE edit/reset vs untouched control | 0 | 0 | Other slot's observed history unaffected |
| OXIDE edited-setting control baseline | 0 | 0 | Equal observed starting histories |
| OXIDE continuation after CHOWTape edit/reset vs untouched control | 0 | 0 | Other slot's observed history unaffected |
| OXIDE before/after output edit | 4.896749928594e−2 | 3.458567890968e−2 | Processed-output parameter effect, not just readback |
| CHOWTape before/after output edit | 3.890375420451e−2 | 2.741414841625e−2 | Processed-output parameter effect, with reference repeat variation also present |
| CHOWTape continuation vs baseline start segment | 1.449229270220e−1 | 3.403353889782e−3 | Retained history materially affects this input boundary |
| OXIDE continuation vs baseline start segment | 2.499371394515e−2 | 5.354747913212e−4 | Retained history materially affects this input boundary |

The history-sensitivity rows show that the continuation controls do not pass merely because this signal forgets history or because both sides were reset immediately before comparison.
Every intervention also asserts exact equality of the other slot's complete parameter snapshot.
Reference controls mirror all preceding reference renders, not only its visible settings.
OXIDE's reset permits its edited-setting control to match despite a different pre-edit render history.
These checks cover the tested edits/reset/signals, not every hidden state or future automation sequence.

### State limitations

CHOWTape's reset/preconditioning sequence retains an observable dependence on earlier processing.
The exact internal state responsible was not isolated; do not label it random noise or claim a complete DSP reset.
Matching the full prior trial sequence reproduced it exactly across independent workers in this run.
The fresh control's first trial also matched the primary's first trial exactly.
This does **not** establish cross-machine/version reproducibility, arbitrary-history restoration, or that reactivation alone resets it.
No CLAP state save/load, RNG seed control, native preset recall, or exhaustive hidden-state inspection was tested.
OXIDE reset reproducibility is established only for these settings/signals/configuration.

### Crash and timeout recovery

After both edited results completed, send `crash` to the loaded OXIDE worker.
Its explicit `abort()` produced **SIGABRT / signal 6**; the parent reported that worker failure and continued.
The OS status text mentions a core dump, but the command disabled core-file generation with `ulimit -c 0`.
Start a fresh OXIDE replacement worker, then send `hang`.
The configured **5,000 ms command deadline** expired; the parent killed/reaped it with **SIGKILL / signal 9** and reported the timeout.
These are deliberate worker faults while real plugins are loaded, not observed spontaneous plugin defects.

After each fault, the parent verified the completed CHOWTape result remained byte-for-byte unchanged and queried the still-loaded CHOWTape worker successfully.
Retained result: `chow-edited.f32le`, 262,144 bytes, SHA-256 `136f7c7a0fb25ca860c1e3fa0f3526ba278b1c907bdeccd948ec7a6bc01be826`.
Surviving reference/control workers completed normal shutdown successfully.
Recovery does not restore the failed worker's processing state or provide a filesystem/security sandbox.

## Automated fixtures and regression checks — separate from mechanism evidence

- `cargo test ... --locked --offline`: **2 fixture tests passed**.
- The deterministic in-memory gain/history fixture verifies exact reset repetition, a real gain-edit output change, untouched peer settings/history against a control, and a nonzero difference if the peer's history is reset.
- Input/difference checks verify repeatable input, distinct channels/blocks, and known peak/RMS error arithmetic.
- These fixtures do **not** exercise CLAP ABI, dynamic loading, worker recovery, or either real plugin; the run above supplies that evidence.
- Final `cargo check`, strict all-target `cargo clippy`, and `cargo fmt -- --check` for `analyse/` passed.
- The parent reported baseline portable DSP tests: **7 passed**, and plugin-enabled library tests: **7 passed**.
- The parent reported `tape/` formatting passed; its upstream VST3 macro warnings were already present.
- Parent independently reran the final probe into `analyse/runs/review/`; every numerical comparison and crash/timeout recovery outcome reproduced.
- Parent reran the 2 fixture tests, check, strict Clippy, and formatting successfully.
- `git diff -- tape` remained empty after the experiment.

## ABI references and unsupported scope

Read the cached `clap-sys` definitions and authoritative CLAP headers, cloned with:

```sh
git clone --depth 1 https://github.com/free-audio/clap.git /tmp/inspector-clap-headers
# Checked-out revision: a47f6badb49d948fd009998f28309cdab78979c9
git clone --depth 1 --branch v2.11.4 \
  https://github.com/jatinchowdhury18/AnalogTapeModel.git /tmp/inspector-chow-source
# Tag revision: 604372e4ffd9690c3e283362e4598cb43edbb475
```

CLAP `entry.h`, `plugin.h`, `host.h`, `process.h`, `audio-buffer.h`, and `ext/{params,render,latency,thread-check}.h` define the lifecycle, ABI, event, and thread contracts used here.
In particular, thread-check permits the main OS thread to act as the serialized symbolic audio thread.
The CHOWTape tag was inspected for context, not used to prove the installed binary's build provenance or the cause of reset variation.

The probe explicitly rejects missing IDs/extensions required for this experiment, non-stereo/multi-bus layouts, rejected activation/offline mode, hard realtime requirements, restart requests, failed parameter readback, failed process status, and nonfinite output.
Absent optional host extensions are reported to the plugin as unavailable.
Editors, timers, note routing, output-event recording, dynamic rescans/restarts, alternate layouts/rates/block sizes, saved state, malicious-plugin containment, and arbitrary plugin compatibility are unsupported/unverified.
No unsupported behavior was bypassed by substituting another plugin or format.
