# AudioTestReference processing contract

Status: Processing approved with the reset/tolerance rules below; [plan.md](plan.md) approves X11/XWayland helper embedding. Finalize host-triggered reset semantics before introducing reset. Behavior contract, not implementation.

## Domain and precision

- One mode per frame, no chaining. No hidden gain, smoothing, dither, noise, oversampling, DC removal, limiter or bypass processing.
- Linux x86_64; stereo float32 I/O; Fs=44,100–192,000 Hz inclusive. Reject unsupported layouts/rates at activation. Mono/float64 buffers are deferred.
- Inputs/effective parameters are exact binary values; equations define real arithmetic. Use binary64 processing/state, rounding output to binary32, nearest ties-to-even. Transcendentals need not be bit-identical across machines.
- Fresh state, explicit reset, activation, sample-rate change and successful preset/state restoration zero all signal histories/envelopes. Reset preserves parameters. Persist mode and every mode's parameters, not signal history.
- Mode changes/explicit reset clear all histories before the first affected sample. Delay-length changes clear only delay history, even while inactive. Other changes preserve history, including biquad x/y and compressor envelope. Inactive settings persist; unchanged values do not reset. No transition fades.
- Host events apply at supplied sample offsets, before that sample. At each offset, apply final values in host event order, then required resets once; explicit reset acts even with unchanged values. UI changes apply at the next available audio boundary, without exact click-to-sample timing. Empty blocks advance no signal state.
- Reject invalid configurations atomically, retaining the last valid one; never silently clamp accepted mathematical parameters. Unknown modes/incompatible saved-state versions fail explicitly without changing configuration.
- Replace nonfinite input with zero. Nonfinite channel output/state or output beyond finite binary32 range produces zero and clears that channel's histories before the next sample. A shared compressor-envelope fault clears both channels and outputs zero for that frame. Indicate faults outside the audio callback. Do not saturate ordinary finite output to ±1.
- Every mode, including Delay, reports **zero host compensation latency**. Separately expose intentional delay N in samples; reported zero does not mean no signal delay. Biquad/envelope phase/settling is not compensation latency.

Favor deterministic, inspectable behavior; defer variants until justified by a measurement gap.

## Parameters

Units are shown below. Fresh instances default to Identity and these values. Integer controls reject nonintegers.

| Mode | Parameters: range; default |
| --- | --- |
| Gain | Linear gain g: −16…16; 1 |
| Delay | N: integer 0…192,000 samples; 0 |
| Low-pass | Cutoff fraction u = fc/Fs: 0.0001…0.49; 0.1. Q: 0.1…20; 1/sqrt(2) |
| Harmonics | a, b: −4…4; 0, 0 |
| Clipper | Linear threshold T: 0.000001…4; 1 |
| Compressor | Threshold θ: −120…0 dBFS peak; −18. Ratio R: 1…100; 4. Attack τa: 0…10 s; 0.01. Release τr: 0…10 s; 0.1 |
| Stereo Matrix | mLL, mLR, mRL, mRR: −4…4; 1, 0, 0, 1 |

Store defaults at declared parameter precision. Display effective values with round-trip precision, low-pass cutoff as both u and fc=uFs in Hz, and read-only effective normalized biquad coefficients. Cutoff/Q controls are approved; direct coefficient editing is deferred.

## Equations and initial state

n counts frames since reset. Histories are channel-independent except the linked compressor detector. xL/xR are original current-frame inputs, never partially overwritten outputs.

### Identity

`y[n] = x[n]`.

Finite binary32 inputs, including signed zero, pass bit-exactly; only the nonfinite-input policy differs.

### Gain

`y[n] = g*x[n]`.

No processing-path dB conversion; derive dB display from g when meaningful.

### Delay

`y[n] = x[n−N]`, with `x[k]=0` for `k<0`.

N=0 is Identity; N is independent of Fs. Changing N clears both delay histories; no old samples survive.

### Low-pass

RBJ-style digital low-pass using effective parameters at active Fs:

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

Initial x/y histories are zero. Cutoff/Q changes recalculate coefficients before the affected sample; the new equation uses preserved x/y histories. Signs/normalization above are authoritative. Read-only order: b0,b1,b2,a1,a2; a0=1. No extra dry path or compensation. Response: `H(z)=(b0+b1*z^-1+b2*z^-2)/(1+a1*z^-1+a2*z^-2)`.

### Harmonics

`y[n] = x[n] + a*x[n]^2 + b*x[n]^3`.

Retain generated DC and all sampled aliases. a/b are polynomial coefficients, not H2/H3 amplitudes or THD; the cubic also changes the fundamental.

### Clipper

`y[n] = min(T,max(−T,x[n]))`.

Endpoints unchanged; no antialiasing/oversampling. Folded harmonics are intentional.

### Compressor

Stereo-linked feed-forward peak detector, hard knee. No lookahead, makeup gain, hold, sidechain filter, gain smoothing or extra release stage. Attack/release are detector one-pole amplitude e-folding times, not universal measured gain-response times.

Recalculate on rate/time changes:

```text
ca = 0 if τa=0, otherwise exp(−1/(Fs*τa))
cr = 0 if τr=0, otherwise exp(−1/(Fs*τr))
```

Per frame, in order:

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

Initial `e[−1]=0`. Threshold/ratio/time changes retain the envelope. Before the affected frame, update time coefficients; use retained envelope/new parameters for that frame's envelope and gain. Equality selects release; updated envelope determines current-frame gain. R=1 gives unity gain; zero times track the selected branch immediately. Linkage couples stereo gain reduction, not audio channels.

### Stereo Matrix

```text
yL[n] = mLL*xL[n] + mLR*xR[n]
yR[n] = mRL*xL[n] + mRR*xR[n]
```

No row normalization, pan law, or implicit clipping.

## Verification contract

- Vectors record algorithm version, effective parameters, Fs, inputs, event offsets/order, reset history, expected outputs and fault recovery. Include impulse, silence, DC, signed and channel-asymmetric signals. Expected values must be independent of production DSP, never generated by the implementation under test.
- Exact checks: finite Identity, zero-delay identity, Delay's copied samples/positions/zero prefix, reset state, matrix routing, exactly representable arithmetic/routing fixtures, event offsets and reported latency. Approximate tolerances cannot excuse exact failures.
- Independently validate per-mode tolerances, not a universal noise floor. Initial `1e−6 + 1e−5*abs(reference)` is a starting ceiling, insufficient for low-level acceptance. Before claiming verification, publish absolute/relative or ULP limits, tested amplitudes, near-zero behavior and reference method. Check low levels, cancellation, long filter/envelope tails and parameter extremes; tighten limits to expose measurement errors. Investigate failures; never automatically relax tolerances.
- Identical complete event/input history in the same binary/environment produces identical samples. Arbitrary block partitions preserve samples for identical sample-indexed events. Host format, editor visibility, instance order and other instances cannot change DSP results.
- Check biquad impulse/analytical complex response at resolvable frequencies; polynomial DC/H2/H3/fundamental terms without labeling aliased bins as unaliased harmonics.
- Check compressor attack/release against the envelope recurrence, knee equality, zero times, R=1, silence and asymmetric stereo peaks. Verify preserved history across threshold/ratio/time/coefficient changes, selective delay reset and full mode-change/explicit reset. Compare eventful sequences with an independent stateful reference, not only fresh-state vectors.
- Cover activation/rejection boundaries, state recall/malformed state, nonfinite samples/parameters, overflow recovery, multiple instances and block-boundary events. No allocation, locks, I/O, GUI dependency or process launching in audio callbacks.

Out of scope: analyzer measurement features, analyzer accuracy inferred solely from shared code, sound-quality optimization, oversampling, cross-architecture bit identity, arbitrary injected live state and compatibility with unreleased prototypes. Published identities, parameter IDs/units, state format and algorithm semantics are compatibility contracts; incompatible changes require explicit versioning.
