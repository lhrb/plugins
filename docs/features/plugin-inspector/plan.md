# Visual plugin inspector

Goal: Build a standalone Rust/GPUI application in `analyse/` that runs generated signals through a plugin and plots its frequency, nonlinear, and transient behavior.

## Decisions and assumptions

- Start with inspection at one selected parameter setting, not fingerprints, automatic knob sweeps, or reference matching.
- Host plugins directly; no DAW exports or audio hardware are required.
- Keep signal generation and measurement in a Rust library independent of GPUI and plugin formats. Accept processor adapters so another project's tests can use it later.
- Reuse the repository's GPUI approach and architecture guidelines. Keep existing projects unchanged.
- Initial target: native Linux, stereo input/output, 48 kHz, 256-frame processing blocks. Reject unsupported configurations explicitly.
- Use generic controls for host-exposed parameters initially. Native plugin editors, preset browsers, and unexposed settings are deferred.
- Run selected plugin code in a separate worker process. This protects application availability, not filesystem access or security; load only trusted binaries.
- No claim of complete characterization, algorithm identification, or perceptual equivalence.

Open question: Which plugin format and first real reference plugin must work? Recommend CLAP first because `tape/` already exports it; this does not establish compatibility with the intended reference.

| Order | Feature | Depends on | Status |
| --- | --- | --- | --- |
| 1 | Hosting feasibility experiment | Format/reference choice | Blocked |
| 2 | Visual frequency-response inspection | 1 | Blocked |
| 3 | Nonlinear and transient inspection | 2 | Blocked |

Next: None. Confirm the first reference plugin and format, then run the hosting experiment.

## 1. Hosting feasibility experiment

Depends on: Format/reference choice.

### Outcome

Establish that the selected format can reliably process generated signals through a real plugin before investing in the inspector.

### Guarantees

- Explicitly select a local plugin and, for multi-plugin containers, a plugin identity; do not execute plugins through automatic discovery.
- Enumerate and set exposed parameters, initialize the supported audio configuration, process finite offline input, and shut down correctly under the format's lifecycle/threading rules.
- Establish reset, parameter application, and preconditioning behavior. Identify state that cannot be reproduced rather than assuming a fresh audio buffer resets the plugin.
- Exercise the existing `tape/` export as a controlled fixture when supported, plus the requested real reference.
- Demonstrate that a worker crash or configured timeout is reported without terminating or indefinitely blocking the parent.

### Scope boundary

No plotting, broad plugin compatibility, editor embedding, or general host framework.

### Done when

Record the tested plugin/version, format, configuration, parameter change, latency reporting, reset behavior, and repeated-render results. Controlled deterministic fixtures must repeat within a declared numerical tolerance; variation from the reference is recorded, not automatically treated as a host defect.

Success establishes the chosen hosting route for these plugins only. Failure identifies the unsupported behavior and blocks dependent work. Reassess host dependencies, lifecycle support, and format scope before proceeding; do not silently substitute another reference or format.

## 2. Visual frequency-response inspection

Depends on: 1.

### Outcome

Select a plugin, configure its exposed parameters, run a sine sweep, and inspect measured gain and phase in GPUI.

### Guarantees

- Create an independently buildable Rust project under `analyse/`, with a headless measurement library and GPUI/hosting adapters. GUI and hosting dependencies are not required to test the measurement library.
- Let the user select a plugin, edit exposed parameters, configure a frequency range and input level, and run/cancel a measurement. No microphone, speaker output, or DAW is needed.
- Begin with a stepped-sine sweep: measure the output fundamental against the input at each frequency, keeping harmonics separate. Show the actual sampled frequencies rather than implying continuous coverage.
- Plot gain in dB and phase in degrees against logarithmic frequency, separately by output channel. Identify the input routing used; allow independent left/right excitation to expose cross-channel response.
- Display input level as dBFS peak, actual sample rate/block size, plugin identity/settings, settling and observation durations, and latency handling. Default to a quiet sweep; do not label it linear merely because its level is low.
- Apply a documented reset/preconditioning sequence before independent trials; preserve history within each trial. Include that sequence in the displayed run conditions.
- Retain raw timing and gain. Do not silently normalize output or align away plugin delay. Any latency-compensated view must be labeled and retain the uncompensated result.
- Reject frequencies at/above Nyquist and invalid levels/durations. Mark measurements below the usable signal floor or without a stable fundamental unavailable; never plot fabricated zeroes or infinite values.
- Keep the interface responsive while processing. Show progress and distinguish success, cancellation, invalid configuration, plugin failure, and timeout.
- Snapshot settings at run start. Later edits mark results stale; cancelled or superseded work cannot replace a newer result. Failed runs cannot appear successful.

### Scope boundary

No saved fingerprints, comparison engine, CLI, automatic control exploration, audio audition, or production-file analysis. Results remain available in the current session; persistent export is deferred.

### Done when

- The headless library measures identity, known gain, known sample delay, and a filter with an analytical response. At declared valid test frequencies above the measurement floor, gain error is at most 0.1 dB and phase error at most 1 degree; recovered integer delay is within one sample. Exclude undefined phase at response nulls explicitly.
- Routing fixtures verify direct and cross-channel responses without combining stereo channels.
- A GPUI walkthrough selects a real plugin, changes a parameter, runs a sweep, reads plotted values, cancels, and recovers from a simulated worker failure.
- A repeat run on a deterministic fixture gives the same measurements within declared numerical tolerances.
- Existing plugin projects remain unchanged and their relevant checks do not regress.

## 3. Nonlinear and transient inspection

Depends on: 2.

### Outcome

Use the same inspector to reveal saturation and compression behavior that a quiet sweep misses.

### Guarantees

- **Level-stepped tone:** choose a frequency and input-level range; plot output fundamental level and fundamental gain versus input level, harmonic levels versus input level, and the output spectrum for a selected step.
- State the peak/RMS convention. Display harmonics in dBc with absolute dBFS available; exclude harmonics at/above Nyquist and distinguish unresolved components from zero distortion.
- Show positive/negative waveform asymmetry and output DC separately. A spectrum alone must not imply the shape of a saturation curve.
- **Amplitude-stepped tone/burst:** choose carrier frequency, levels, hold times, and recovery time; plot input/output waveforms and envelopes against time. Let the user inspect overshoot, steady-state change, and recovery without claiming universal attack/release constants.
- Use a stated envelope method and timing reference; expose its temporal resolution. Mask output/input ratios when input is near zero.
- Allow sweep reruns at other input levels using the same conditions, exposing level-dependent frequency response without automatic knob sweeps.
- Carry forward the configuration, routing, state handling, numerical validity, cancellation, and stale-result guarantees from feature 2.
- Include brief plot guidance: frequency-response changes suggest filtering; harmonic/level changes suggest nonlinear behavior; differing onset/recovery suggests memory or detection behavior. Present these as clues, not unique algorithm diagnoses or audibility judgments.

### Done when

- Identity/gain fixtures show the expected transfer curves and no measurable added harmonics above the declared numerical floor.
- An analytical polynomial fixture produces its predicted fundamental and below-Nyquist harmonics within 0.5 dB where components exceed -80 dBFS.
- An asymmetric fixture demonstrates the expected even-harmonic/DC behavior; a known dynamic-gain fixture produces the expected envelope trajectory within a declared tolerance accounting for the envelope method's resolution.
- The real reference can be inspected with all three measurement modes, and plots respond to relevant parameter changes.

## Deferred work and reassessment

After the first real plugin works end to end, reassess which measurements answer actual production questions before expanding. Candidate additions: intermodulation/aliasing tests, noise and wow/flutter analysis, stereo-link tests, sample-rate/block-size matrices, native editors, more formats, saved profiles, reference comparisons, and production-clip validation. Library reuse is preserved now; a public stable API and integration into another project's test chain are separate work.

Status: Planning only. No implementation, tests, Cargo scaffold, or hosting dependencies have been added.
