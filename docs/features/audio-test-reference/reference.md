# AudioTestReference processing contract

Status: Processing choices approved with the reset/tolerance revisions below. Initial X11/XWayland helper-based GPUI embedding is approved in `plan.md`. Host-triggered reset semantics remain to be finalized before slice 2. This document defines behavior, not a DSP implementation.

## Domain and precision

- One mode processes each frame; modes are not chained. No hidden gain, smoothing, dither, noise, oversampling, DC removal, limiter, or bypass processing.
- Initial scope: Linux x86_64, stereo float32 input/output, sample rates 44,100–192,000 Hz inclusive. Reject unsupported layouts/rates during activation. Mono and float64 host buffers are deferred.
- Interpret input samples and effective parameter values as exact binary values. Equations below define the real-arithmetic reference. Use binary64 processing/state, then round output to binary32, round-to-nearest ties-to-even. Do not promise cross-machine bit identity for transcendental functions.
- Fresh state, explicit reset, activation, sample-rate change, and successful preset/state restoration set all signal histories and compressor envelopes to zero. Reset preserves parameters. Persist mode and every mode's parameter values, not live signal history.
- Mode changes and explicit reset clear all signal histories immediately before the first affected sample. Delay-length changes clear only delay history, even when Delay is inactive. Other parameter changes preserve history, including biquad input/output histories and compressor envelope. Inactive-mode settings persist. Unchanged values do not reset history. No transition fades or smoothing.
- Host parameter events take effect at their supplied sample offsets, before that sample. At one offset, apply final parameter values in host event order, then perform any required resets once; an explicit reset still takes effect when parameter values are unchanged. UI changes apply at the next available audio boundary; exact click-to-sample timing is not promised. Empty blocks advance no signal state.
- Invalid-value policy: reject invalid configurations atomically, retaining the last valid configuration. Never silently clamp accepted mathematical parameters. Unknown modes and incompatible saved-state versions fail explicitly without changing the active configuration.
- Sample fault policy: replace nonfinite input with zero. If a channel's computed output/state becomes nonfinite or output exceeds finite binary32 range, output zero and clear that channel's histories before the next sample. A shared compressor envelope fault clears both channels and outputs zero for that frame. Expose a fault indication outside the audio callback. Do not saturate ordinary finite output to ±1.
- All modes report **zero host compensation latency**, including Delay. Separately expose intentional delay N in samples; never label host-reported zero as absence of signal delay. Biquad/envelope phase or settling is not compensation latency.

The initial version favors deterministic, inspectable behavior; additional processing variants are deferred until a measurement gap justifies them.

## Parameters

All continuous controls have the units shown. Defaults apply to a fresh instance; Identity is the initial mode. Integer controls accept integers only.

| Mode | Parameters: range; default |
| --- | --- |
| Gain | Linear gain g: −16…16; 1 |
| Delay | N: integer 0…192,000 samples; 0 |
| Low-pass | Cutoff fraction u = fc/Fs: 0.0001…0.49; 0.1. Q: 0.1…20; 1/sqrt(2) |
| Harmonics | a, b: −4…4; 0, 0 |
| Clipper | Linear threshold T: 0.000001…4; 1 |
| Compressor | Threshold θ: −120…0 dBFS peak; −18. Ratio R: 1…100; 4. Attack τa: 0…10 s; 0.01. Release τr: 0…10 s; 0.1 |
| Stereo Matrix | mLL, mLR, mRL, mRR: −4…4; 1, 0, 0, 1 |

Effective stored defaults use the declared parameter precision. Display actual effective values with enough digits for round-trip identification. Display low-pass cutoff in Hz as fc=uFs as well as u. Provide read-only effective normalized biquad coefficients. Cutoff/Q controls are approved for the initial version; direct coefficient editing is deferred.

## Equations and initial state

n is the frame index since reset. Channels have independent histories except the linked compressor detector. xL and xR always refer to the original inputs of the current frame, not partially overwritten outputs.

### Identity

`y[n] = x[n]`.

Finite binary32 inputs, including signed zero, pass bit-exactly. The nonfinite-input policy is the sole exception.

### Gain

`y[n] = g*x[n]`.

No dB conversion occurs in the processing path; dB display, when meaningful, is derived from g.

### Delay

`y[n] = x[n−N]`, with `x[k]=0` for `k<0`.

N=0 is Identity. N is a sample count independent of Fs. Changing N clears both delay histories; no old samples survive.

### Low-pass

RBJ-style digital low-pass, coefficients calculated from the effective parameters at the active sample rate:

```text
fc = u*Fs
ω = 2π*fc/Fs
α = sin(ω)/(2Q)
B0 = (1−cos(ω))/2
B1 = 1−cos(ω)
B2 = (1−cos(ω))/2
A0 = 1+α
A1 = −2cos(ω)
A2 = 1−α
b0=B0/A0; b1=B1/A0; b2=B2/A0
a1=A1/A0; a2=A2/A0

y[n] = b0*x[n] + b1*x[n−1] + b2*x[n−2]
       − a1*y[n−1] − a2*y[n−2]
```

Initial x and y histories are zero. Cutoff/Q changes recalculate coefficients before the affected sample while retaining prior x/y samples; the new difference equation uses those preserved histories. The feedback sign convention and coefficient normalization above are authoritative. Read-only coefficient order is b0,b1,b2,a1,a2, with a0=1. No extra dry path or compensation. Frequency response is `H(z)=(b0+b1*z^-1+b2*z^-2)/(1+a1*z^-1+a2*z^-2)`.

### Harmonics

`y[n] = x[n] + a*x[n]^2 + b*x[n]^3`.

Retain generated DC and all sampled aliases. a and b are polynomial coefficients, not requested H2/H3 amplitudes or THD values; cubic processing also changes the fundamental.

### Clipper

`y[n] = min(T,max(−T,x[n]))`.

Endpoints are unchanged. No antialiasing or oversampling; folded harmonics are intentional reference behavior.

### Compressor

Stereo-linked, feed-forward peak detector; hard knee, no lookahead, makeup gain, hold, sidechain filter, gain smoothing, or extra release stage. Attack/release describe the amplitude detector's one-pole e-folding time, not a universal measured gain-response time.

Calculate coefficients on a rate/time change:

```text
ca = 0 if τa=0, otherwise exp(−1/(Fs*τa))
cr = 0 if τr=0, otherwise exp(−1/(Fs*τr))
```

For each frame, in this order:

```text
p[n] = max(abs(xL[n]), abs(xR[n]))
c[n] = ca if p[n] > e[n−1], otherwise cr
e[n] = c[n]*e[n−1] + (1−c[n])*p[n]

if e[n] = 0:
    G[n] = 1
else:
    L[n] = 20*log10(e[n])
    D[n] = 0                         if L[n] <= θ
           (θ−L[n])*(1−1/R)          otherwise
    G[n] = 10^(D[n]/20)

yL[n] = G[n]*xL[n]
yR[n] = G[n]*xR[n]
```

Initial envelope `e[−1]=0`. Threshold/ratio/time changes retain the envelope. Recalculate time coefficients before the affected frame; calculate that frame's envelope and gain from the retained envelope and new parameters. Equality uses the release branch. Updated envelope determines gain for the current frame. R=1 produces unity gain. Zero times track the selected branch immediately. Detector linkage intentionally couples stereo gain reduction; it does not mix audio channels.

### Stereo Matrix

```text
yL[n] = mLL*xL[n] + mLR*xR[n]
yR[n] = mRL*xL[n] + mRR*xR[n]
```

No row normalization, pan law, or implicit clipping.

## Verification contract

- Reference vectors identify algorithm version, effective parameter values, Fs, input samples, event sample offsets/order, reset history, expected outputs and any fault recovery. Include impulse, silence, DC, signed samples, and channel-asymmetric signals.
- Exact checks: finite Identity, zero-delay identity, integer delay positions/zero prefix, reset state, matrix routing, event offsets, and reported latency. Verify numerical outputs independently of the production DSP; do not generate expected vectors with the implementation under test.
- Use mode-specific tolerances validated against independent reference calculations, not a universal noise floor. The initial `1e−6 + 1e−5*abs(reference)` bound is a starting ceiling, not a sufficient low-level acceptance criterion. Publish each mode's absolute/relative or ULP limits, tested amplitudes, near-zero behavior, and reference method before claiming it verified. Check low-level signals, cancellation, long filter/envelope tails, and parameter extremes; tighten limits to avoid masking measurement errors. Failure requires investigation, not automatic tolerance relaxation.
- Retain exact comparisons wherever guaranteed: finite Identity, Delay's copied samples, and exactly representable arithmetic/routing fixtures. Do not apply approximate tolerance to excuse failures of these guarantees.
- Repeated runs of the same binary/environment and identical complete event/input history produce identical samples. Arbitrary block partitions preserve samples for the same sample-indexed events. Host format, editor visibility, instance order, and other instances do not change DSP results.
- Check the biquad impulse and analytical complex response at resolvable frequencies. Check polynomial DC/H2/H3/fundamental terms without treating aliased bins as unaliased harmonics.
- Check compressor attack and release against the explicit envelope recurrence, knee equality, zero-time cases, R=1, silence, and asymmetric stereo peaks. Check history preservation across threshold/ratio/time changes and coefficient changes, selective delay reset, and full reset on mode changes or explicit reset. Compare eventful sequences against an independent stateful reference, not just fresh-state vectors.
- Cover activation/rejection boundaries, state recall and malformed state, nonfinite samples/parameters, output overflow recovery, multiple instances, and events on block boundaries. No allocation, locks, I/O, GUI dependency, or process launching in the audio callback.

Out of scope: analyzer measurement features, analyzer accuracy inferred solely from shared code, sound-quality optimization, oversampling, bit identity across architectures, arbitrary injected live state, and backward compatibility with unreleased prototypes. Once published, identities, parameter IDs/units, state format and algorithm semantics become compatibility contracts; incompatible changes require explicit versioning.
