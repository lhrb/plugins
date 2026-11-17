# Audio plugin profiling plan

Goal: Deliver the [profiling system](../../../analyse/README.md) through small, understandable, plot-first steps.

## Approach

- Implement only the agreed slice; show plots, evidence, and limitations, then wait for feedback.
- Detail only the next slice; split later milestones into small slices before implementation.
- Reuse existing code and preserve valid tests/guarantees; avoid a rewrite or general host framework.
- Keep signal generation/analysis independent of plugin formats and UI.
- Start with Rust/GPUI on Linux, stereo, 48 kHz, 256-frame blocks.
- Load explicitly selected trusted binaries in isolated workers; this contains failures, not malicious code.
- Record versioned conditions and actual settings from the first measurement, before building a database.
- Preserve absolute levels, raw timing, and per-channel results; label any normalization or compensation.
- Use the same documented reset → silent warm-up → ordered measurements procedure; retain history between frequency steps and disclose repeat variation.
- Support both CLAP and VST3 in the initial version; a first CLAP plot is only a feedback checkpoint.

## Roadmap

| Step | Observable result | Depends on | Status |
| --- | --- | --- | --- |
| 1 | One CLAP plugin with validated, understandable gain/phase plots | — | Ready |
| 2 | VST3 passes equivalent hosting and plotted-response checks | 1 + feedback | Blocked |
| 3 | Automated frequency profiles with conditions/settings snapshots and repeat overlays | 2 + feedback | Blocked |
| 4 | Spectra and harmonic maps across frequency and input level | 3 + feedback | Blocked |
| 5 | Transient/dynamics plots: waveforms, envelopes, level transfer, onset, recovery | 4 + feedback | Blocked |
| 6 | Local database stores/retrieves profiles with standardized, versioned conditions | 5 + storage decisions + feedback | Blocked |
| 7 | Saved-profile comparisons show overlays, differences, and condition-specific similarities | 6 + feedback | Blocked |

Hosting checks must cover lifecycle, parameter readback/output effect, latency, finite offline output, shutdown, and crash/timeout recovery for named binaries/configurations—not universal compatibility.
Harmonic plots must state dBc/dBFS conventions, numerical floor, DC, and Nyquist exclusions.
Dynamics plots must state envelope method and time resolution, not claim universal attack/release constants.
Database records must identify plugin ID/version/hash, settings, protocol/analysis versions, conditions, and validity/history caveats.
Decide storage, raw-capture retention, and schema compatibility before persistence work.
Comparisons must flag incompatible conditions and explain measured tradeoffs, not infer perceptual equivalence or an overall quality score.

Next: Agree on slice 1, implement only it, then review its plots together.
No consequential questions currently block slice 1; select VST3 validation binaries before step 2.

## Slice 1 — Understand one plugin's frequency response

Depends on: None; reuse existing CLAP hosting and measurement code.

### Outcome

Select one CLAP plugin and inspect its stepped-sine fundamental gain and phase under visible conditions.

### Guarantees

- One selected plugin is sufficient; do not remove existing two-slot behavior.
- Explain the signal path: test tone → plugin → output fundamental → gain/phase, including what these plots miss.
- Show actual plugin identity/settings, input dBFS peak, sampled frequencies, routing, sample rate/block size, warm-up/settling/observation times, reset/history procedure, and reported latency.
- Plot each output channel's gain (dB) and phase (degrees) against logarithmic frequency with readable ticks, units, legends, sampled points, and inspectable values.
- Keep raw gain/phase; a quiet sweep is not proof of linearity.
- Reject invalid conditions; identify unavailable points and reasons without fabricated values or lines bridging missing data.
- Keep the UI responsive with progress and distinct completion, cancellation, invalid-input, plugin-failure, and timeout states.
- Snapshot settings/conditions; edits mark results stale, and cancelled/superseded work cannot overwrite current results.
- Preserve independent settings/history and completed peer results wherever existing two-slot paths are used.
- Repeat the real-plugin trial and report differences; reset is not proof that hidden history is erased.

### Scope boundary

No new simultaneous-comparison requirement, VST3, persistence, harmonic/dynamics modes, discovery, native editors, audio devices, automatic scoring, or broad host refactor.

### Done when

- Existing headless tests pass, including routing, history, cancellation, stale/partial results, and numerical edge cases.
- Identity, known gain/delay, and analytical-filter fixtures meet 0.1 dB gain / 1° phase accuracy at declared resolvable frequencies; integer delay is within one sample. Null/undefined cases remain unavailable.
- A real-plugin walkthrough demonstrates selection, completed plots, parameter edit → stale result → rerun, and cancellation/error reporting.
- Show the requester plots with tested binary/configuration and repeat-run observations; explain one measured change and one limitation without claiming a unique algorithm or audibility.

Stop for feedback on clarity before expanding.
If GUI build/integration is blocked, report and reassess rather than substituting unrelated hardening.

## Useful findings and remaining gaps

- Prior Linux CLAP trials hosted OXIDE 0.1.0 and CHOWTape 2.11.4 independently at stereo float32 / 48 kHz / 256 frames; parameter edits and injected crash/timeout isolation worked in that experiment.
- OXIDE reset trials matched exactly; CHOWTape differed by 4.936e−5 peak against a 1e−6 tolerance after reset/preconditioning. Matched full histories reproduced exactly in that session; arbitrary-history restoration remains unverified.
- Evidence: [host transcript](../../../analyse/evidence/slice1.log) and [identity/artifact hashes](../../../analyse/evidence/SHA256SUMS). These predate the current frequency-sweep procedure and do not verify the GUI.
- Last headless baseline: 2 library + 11 measurement + 6 run-state tests passed. [Test commands and coverage limits](../../../analyse/tests/README.md).
- GUI integration remains unverified; an earlier build hit `xattr 0.2.3` / `libc::ENOATTR` incompatibility. Recheck before treating it as a current blocker.
- VST3 hosting, harmonic/dynamics analysis, and the database remain unimplemented.

Outside the initial version: automatic parameter matching, native editors, production-audio validation, perceptual ranking, and exhaustive plugin/OS/configuration coverage.
