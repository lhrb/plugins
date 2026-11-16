# Tape 74 — clean recording tape

Goal: A lightweight, fixed-30-ips tape effect for individual instrument tracks, with subtle default coloration and level-dependent high-frequency compression.

## Decisions and assumptions

- Product name: **Tape 74**, replacing OXIDE across plugin identity, standalone presentation, build artifacts, and documentation owned by `tape/`.
- Controls: Drive, Bias, Output. Drive combines record-level gain and tape excitation; no separate input-gain or saturation control.
- Bias offers restrained clean calibration variations, not underbiased distortion or a disguised static treble EQ.
- Speed is fixed at 30 ips. The default setting is **Clean Record**.
- Existing sessions are unused; preserving OXIDE identity, parameters, presets, or sound is unnecessary. Give Tape 74 its own host identities.
- Working assumptions: fully wet processing; no added hiss, wow/flutter, wear, widening, output clipper, or automatic gain control. Output is an independent post-processing trim.
- Prefer a lightweight perceptual approximation; do not promise a particular machine, physical simulation, or CHOW-equivalent sound. Airwindows and CHOW are references, not predetermined dependencies.
- Preserve stereo operation, 8–384 kHz host acceptance, automation/state persistence, finite-output safeguards, channel isolation, and allocation-/lock-free audio processing. Continue using the same DSP in plugin and audition app.

Open questions: Slice 1 demonstrates the mechanism but lacks CHOW capture and human listening. Its unprotected candidate shows substantial aliasing. Sound, aliasing, CPU, calibration, and latency limits remain unapproved; slice 2 stays blocked.

| Order | Feature | Depends on | Status |
| --- | --- | --- | --- |
| 1 | Clean-tape feasibility experiment | — | Blocked |
| 2 | Tape 74 recording effect | 1 | Blocked |

Next: None Ready. Review [experiment evidence](experiment.md), listen to the generated comparisons, and obtain a documented clean CHOW capture. Decide whether to authorize a bounded alias-controlled probe revision; do not start slice 2.

Checkpoint: Mechanism and CPU measurements exist in `data/`; portable tests (7), probe check (1), formatting, and targeted Clippy pass. Slice 1 is not Done because reference comparison and listening remain unverified.

## 1. Clean-tape feasibility experiment

Depends on: None.

### Outcome

Determine whether a lightweight model can deliver useful clean tape coloration and high-frequency compression at an acceptable many-instance CPU cost.

### Guarantees

- Compare the current DSP, a minimal candidate, and a documented clean CHOW configuration using identical inputs and level-matched listening. Record versions and settings; do not treat CHOW as ground truth.
- Measure fundamental gain versus input level at low, mid, and high frequencies. Distinguish level-dependent high-frequency compression from static filtering and broadband saturation.
- Evaluate default and driven settings on representative instrument transients and sustained material, including repeated processing to expose cumulative dulling or distortion.
- Report low-level response, harmonic distortion, aliasing, output-level change, and behavior across the proposed clean Bias range. Include 44.1, 48, and 96 kHz; report stability separately at supported rate boundaries.
- Benchmark release-build audio processing, including many instances, with hardware, sample rate, block size, instance count, and timing method recorded. Report CPU and any latency alongside sound-quality tradeoffs.
- Keep the experiment bounded to sound and resource feasibility. Reuse existing measurement facilities where adequate; do not make the separate inspector project a dependency or build a new measurement framework.

### Scope boundary

No complete plugin/UI rewrite, full physical tape model, or exact reference matching.

### Done when

Publish reproducible settings, measurements, listening observations, and a proceed/revise/stop recommendation. Before slice 2, agree on nominal input calibration, control ranges/defaults, clean-level tolerance, minimum high-frequency compression behavior, aliasing ceiling, CPU budget, and latency limit.

Success establishes a viable candidate under explicit acceptance limits, not universal tape realism. Failure blocks product work and identifies the unmet sound/resource requirement. Reassess model complexity and quality safeguards before substantial infrastructure; do not trade away alias control solely to reduce CPU.

## 2. Tape 74 recording effect

Depends on: Slice 1 and approval of its acceptance limits.

### Outcome

Use Tape 74 as a consistent, clean recording-tape stage with optional additional drive.

### Guarantees

- Hosts and the audition app expose Drive, Bias, and Output with the same meanings and defaults; no adjustable speed, tone, modulation, or blend remains.
- A fresh instance and the Clean Record setting restore the approved clean calibration at fixed 30 ips, with near-unity level under the agreed reference conditions.
- Drive moves from subtle coloration to stronger saturation and high-frequency compression; reducing incoming level restores the cleaner response. Loud high-frequency content softens more than quiet content, within approved limits.
- Bias changes nonlinear character within the approved clean range. Output changes level after the tape behavior without changing its excitation.
- No generated noise, transport modulation, intentional crosstalk, or stereo widening is introduced. Silence remains silent after prior signal state has decayed.
- Parameter automation remains smooth; state restoration recalls all controls. Reset and sample-rate changes clear incompatible history. Invalid controls and extreme/nonfinite input do not poison later processing or produce nonfinite output.
- CLAP/VST3 identity, build/install instructions, artifacts, and standalone branding consistently identify Tape 74. Old OXIDE state migration is not required.
- Preserve existing host formats, supported stereo/rate boundaries, independent builds, and shared audition/plugin DSP. The standalone app remains output-only and is not presented as an embedded plugin editor.

### Scope boundary

No embedded editor, mono-layout expansion, machine selection, additional speeds, degradation effects, separate saturation control, or inspector changes. UI styling and internal DSP design belong to the builder.

### Done when

Verify the approved slice-1 sound/resource limits, control behavior, automation/state recall, identity/build outputs, and relevant existing safety tests. Check rate boundaries and normal production rates. Confirm plugin discovery and processing in a host, plus audition parity; record unverified platforms explicitly.

Deferred: broader format/platform validation and features outside this focused recording-tape role.
