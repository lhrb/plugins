# Clean-tape feasibility checkpoint

## Verdict

**Revise, then review; do not authorize slice 2 yet.** The isolated candidate demonstrates level-dependent HF compression with nearly unity low-level response and modest measured CPU. It does not establish acceptable sound quality. Strong high-frequency excitation produces substantial alias energy without oversampling. CHOW comparison and human listening remain blocked/pending. No acceptance limits or product controls were approved here.

Production DSP, plugin, UI, identities, and plan statuses remain unchanged.

## Reproduce

From the repository root:

```sh
cd tape
cargo test --locked --no-default-features --lib --example clean_tape_probe
cargo run --locked --release --no-default-features --example clean_tape_probe -- /tmp/tape-74-probe --tones-only
cargo run --locked --release --no-default-features --example clean_tape_probe -- /tmp/tape-74-probe
rustfmt --edition 2021 --check examples/clean_tape_probe.rs
```

`data/tones.csv`, `data/fixtures.csv`, and `data/cpu.csv` preserve this run. The example regenerates CSV and 18 stereo float WAVs in the supplied directory. No new dependencies. The existing `analyse/src/lib.rs` provides fixed-48k input/difference helpers, not the required measurements or hosting; importing it would add coupling without useful reuse. It and the inspector work were left untouched.

## Models and settings

- Current: existing portable `TapeProcessor`, actual defaults (Drive +6 dB, Output −6 dB, neutral Tone, Wow 15%, 15 ips, wet 100%). Driven comparison adds 12 dB Drive and subtracts 12 dB Output. These are not proposed Tape 74 settings.
- Candidate: complementary one-pole split at 1.5 kHz, `low += alpha*(x-low)`, `high=x-low`; output `tanh(a*low)/a + tanh(b*high)/b`. `a=0.35*10^(drive/20)`, `b=3*10^(drive/20)*(1+0.2*bias)`.
- Candidate experimental settings: Drive 0 or +12 dB; Bias −1, 0, +1 (HF nonlinear strength 2.4/3/3.6 before drive). No static HF cut, noise, modulation, oversampling, control automation, or claimed physical bias simulation. Output trim is unity. Finite input guards and channel-independent history are local to the probe.
- This deliberately minimal nonlinear split tests the mechanism, not a product-ready architecture. It omits alias suppression to expose whether further quality safeguards are necessary, not to endorse removing them for CPU savings. Bias changes nonlinear strength rather than small-signal EQ.

## Direct mechanism evidence

Coherent 100 ms analysis after 200 ms warmup; input levels are sine **peak** dBFS, not RMS. Gain uses sine/cosine projection at the fundamental. All combinations of 44.1/48/96 kHz, 100/1k/9k Hz, −60/−36/−24/−18/−12/−6/0 dBFS, two drive settings, and three candidate Bias settings appear in `tones.csv`.

48 kHz, neutral Bias, default drive, fundamental gain (dB):

| Model / input | 100 Hz | 1 kHz | 9 kHz |
|---|---:|---:|---:|
| Current / −60 | −0.167 | −0.056 | −2.904 |
| Current / −18 | −0.302 | −0.191 | −3.039 |
| Current / −6 | −1.979 | −1.868 | −4.716 |
| Candidate / −60 | ~0 | ~0 | ~0 |
| Candidate / −18 | −0.004 | −0.024 | −0.213 |
| Candidate / −6 | −0.066 | −0.320 | −2.502 |

The current processor adds substantial static HF loss and approximately broadband compression. The candidate's HF loss grows with level while its low-level response stays near unity, directly demonstrating the requested distinction. This does not prove perceptual tape realism.

At −6 dBFS/9 kHz, candidate gains across Bias −1/0/+1:

| Rate | Bias −1 | Bias 0 | Bias +1 |
|---|---:|---:|---:|
| 44.1 kHz | −1.729 | −2.445 | −3.165 |
| 48 kHz | −1.771 | −2.502 | −3.236 |
| 96 kHz | −2.025 | −2.846 | −3.663 |

Bias has little low-level effect but audible cleanliness is **unverified**. Rate-dependent split response changes the nonlinear result; the candidate is not rate-invariant.

## Distortion and aliasing

`thd_2_to_9_db` sums projected harmonics 2–9 **only below Nyquist**, relative to the fundamental. It omits higher harmonics and can contain folded aliases at legitimate harmonic bins. Near-Nyquist THD alone is misleading. At 48 kHz/1 kHz/−18 dBFS, current THD is −46.13 dBc; candidate is −56.68 dBc. At −6 dBFS these become −23.89 and −34.49 dBc.

`folded_odd_3_to_15_db` sums distinct bins predicted by folding odd harmonics 3–15 above Nyquist. It excludes DC and bins occupied by legitimate harmonics (including the fundamental), and deduplicates collisions. Excluded collisions are **unresolvable**, not alias-free. This is sparse alias evidence, not a full alias measurement or upper bound. No high-rate/downsampled reference null was performed. A value of −300 is the display floor and may mean no eligible bins, not measured absence of aliases.

At 48 kHz/9 kHz/−18 dBFS, folded-bin evidence is −48.40 dBc current versus −41.73 dBc candidate; at −6 dBFS, −26.09 versus −20.97 dBc. Driven candidate at −6 dBFS reaches −13.01 dBc. This is a concrete quality concern. At 96 kHz/default/−6 dBFS the candidate proxy is −53.36 dBc, but different harmonics cross Nyquist there, so that is not a directly equivalent alias ceiling comparison.

`residual_db` is total measured power minus fundamental power, relative to fundamental. It includes harmonics, aliases, modulation and numerical error, not just aliases. Rectangular coherent measurements avoid ordinary tone leakage; current wow modulation is intentionally noncoherent. Values near numerical floors are not trustworthy distortion specifications. No broadband/intermodulation or exhaustive spectral reference comparison was attempted.

## Fixtures and listening pending

The example generates deterministic 3 s/48 kHz synthetic transient and sustained fixtures: stereo harmonic stacks at 110/146.83 Hz plus 7.3 kHz content; transient envelopes decay every 0.5 s. These are instrument-like stimuli, not real recordings. Each model processes identical sources at default and driven settings, with fresh state on each of eight serial passes and no interpass normalization.

`fixtures.csv` records raw output RMS change and peak. Candidate default RMS changes after 1/8 passes are −0.017/−0.125 dB transient and −0.034/−0.236 dB sustained. Driven results are −0.205/−1.152 and −0.370/−2.033 dB. Current default results are −0.646/−3.643 and −0.930/−4.791 dB. These measure accumulated level change, not perceived dulling.

For user review: `/tmp/tape-74-probe/{transient,sustained}_dry.wav` and corresponding `*_current_*_matched.wav` / `*_candidate_*_matched.wav`. Each processed file at pass 1 or 8 receives a single post-render RMS matching gain against its dry source. Files are 32-bit float, stereo, 48 kHz, 3 s; no limiter. Matching gains are recorded. Compare default/driven, current/candidate and pass 1/8 against dry at a safe playback level. **No human listening occurred.** Matching RMS does not guarantee matching perceived loudness. WAVs live in temporary storage; regenerate before they disappear.

## CPU, latency and safety

Hardware: AMD Ryzen 5 3600, 6 cores/12 threads, Linux x86_64, boost enabled. Rust `1.99.1 (58cf8f9c3)`, release with repository thin LTO and one codegen unit. Timing: `Instant` wall time around serial processing of 3 s stereo, blocks of 256, 1 or 64 independent instances, three trials after 4096-frame warmup per instance. Input generation/model allocation are outside timing; dispatch, iteration and `black_box` are inside. The shared enum wrapper reserves the current processor's ~16.5 KB state footprint even in candidate slots; cache layout and dispatch overhead therefore belong to these timings, not to a standalone optimized candidate. A targeted `large_enum_variant` allowance preserves this measured layout without boxing. No affinity, real-time scheduling, frequency locking, DAW overhead, or deadline-jitter measurement. Trials share a normal multitasking machine.

64-instance serial CPU as percent of one core's real-time duration, median (min–max):

| Rate | Current | Candidate |
|---|---:|---:|
| 44.1 kHz | 15.00 (14.78–15.62) | 14.38 (13.85–15.43) |
| 48 kHz | 16.69 (16.66–16.92) | 15.33 (15.21–15.51) |
| 96 kHz | 34.05 (33.63–34.65) | 31.68 (31.48–33.04) |

Raw single-instance trials and ns/stereo-frame values are in `cpu.csv`. These are measured feasibility costs, not an agreed budget. Candidate `f64` arithmetic and tanh calls may cost more on other hardware. No oversampling cost is included.

Candidate adds zero buffered samples of latency by construction; nonlinear filters still have frequency-dependent phase. Current has an immediate filter path plus a blended ~2 ms modulation delay; this experiment did not measure host-reported latency or group delay. Neither path is a lookahead design.

Portable baseline: all 7 tests pass. Probe: 1 test passes, covering deterministic output, channel isolation, finite extreme/nonfinite-input handling and silence decay at 8/44.1/48/96/384 kHz and Bias extremes. Mechanism assertions require near-unity quiet HF response and greater loud HF than LF compression; they are experiment sanity checks, not acceptance limits. Boundary rates are stability-only, not sound-quality certification. Fresh construction clears candidate history; production reset/rate/automation integration is not implemented or implied.

Two independent release runs produced byte-identical tone CSV, fixture CSV and all 18 WAVs. CPU timings intentionally differ. `ffprobe` confirmed float PCM, stereo, 48 kHz and 3 s duration. Example formatting passes.

## Reference blocker and next checkpoint

Installed reference: `/usr/lib/clap/CHOWTapeModel.clap` (5,935,424 bytes; file date Jan 26 2025), plus `/usr/bin/CHOWTapeModel` and REAPER. The standalone `--help` attempt did not provide a usable offline path and timed out. REAPER advertises rendering an existing configured `.rpp`, but no clean reference project/settings/capture existed. FFmpeg exposes LADSPA, not CLAP. Do not infer a CHOW version or clean parameter configuration from the filename/date. No CHOW audio or comparison was captured; no settings were fabricated. No host or GUI automation framework was built.

Next: have the user review the matched files and supply/capture a documented clean CHOW configuration through an existing host. Review whether this split model's sound warrants a bounded alias-controlled revision. Then agree on nominal calibration, Drive/Bias defaults/ranges, clean tolerance, minimum HF compression, alias ceiling, many-instance CPU budget and latency limit before authorizing slice 2. Current evidence supports the mechanism, not acceptance of this unprotected candidate.
