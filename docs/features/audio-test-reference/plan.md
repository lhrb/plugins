# AudioTestReference — reference audio plugin

Goal: Build a Rust CLAP/VST3 plugin in `audiotestreference/` with embedded GPUI and mathematically specified processing for developing `analyse/`.

## Decisions and assumptions

- Target the current Linux system; host-supplied audio, no editor-owned device or generator. DSP works without the editor.
- [reference.md](reference.md) owns the accepted eight-mode algorithms, parameters, precision, reset/state, latency, fault and verification contracts. Host-triggered reset semantics remain open.
- Follow `architecture.md`: keep DSP independent of GPUI/plugin APIs. Reuse proven local approaches without shared frameworks or dependencies on other plugins' internals.
- [analyse's intended capabilities](../../../analyse/README.md) guide coverage, not a dependency or authorization to change it. Preserve existing plugin/analyzer contracts, measurements and infrastructure.
- GPUI is preferred, not proven for arbitrary hosts or guaranteed bug-free. Prioritize measured responsiveness and reliability; minimize click-to-visible editor opening time. The requester observed an opening delay in Slice 1; measure cold/warm startup before choosing optimizations. Approved: bundled native helper with X11/XWayland embedding, following `nam-linux-port/src/editor.rs`; native Wayland is deferred.
- Start with CLAP in Bitwig; VST3 remains required for completion. Retain the bounded prototype only if suitable for the product.

## Open questions

None blocking the shell. Finalize host-triggered reset edge/event semantics before introducing reset; independently validate per-mode numerical limits before claiming verification.

| Order | Outcome | Depends on | Status |
| --- | --- | --- | --- |
| 1 | Load the CLAP shell and embedded editor in Bitwig | — | Done |
| 2 | Verify the Identity contract | 1 | — |
| 3 | Gain end to end: DSP, editor, host automation/readback | 2 | — |
| 4 | Delay and reproducible reset | 3; reset contract | — |
| 5 | Low-pass with cutoff/Q and read-only coefficients | 4 | — |
| 6 | Harmonics | 3 | — |
| 7 | Clipper | 3 | — |
| 8 | Linked stereo Compressor | 4 | — |
| 9 | Stereo Matrix | 3 | — |
| 10 | Refine mode workflow, controls and state recall | 4–9 | — |
| 11 | Complete format, host/analyzer and lifecycle qualification | 10 | — |

`—` means provisional, not authorized. Add basic usable controls with each mode; reserve cohesive UX refinement for later. Detail only the next slice after review.

Next: None.
Review: Slice 1: accepted by requester; proceed to planning Slice 2. [experiment.md](experiment.md) records automated evidence, successful manual Bitwig use and accepted verification gaps. Editor opening speed remains a follow-up requirement. No later implementation slice is authorized.

## 1. Load the CLAP shell in Bitwig

Depends on: None.

### Outcome

Bitwig loads the plugin and opens its actual embedded GPUI editor without disrupting audio.

### Guarantees

- Build an identifiable CLAP artifact with its bundled helper; use minimal stereo pass-through and report zero compensation latency.
- Editor open/close leaves processing unchanged; expose failures outside the audio callback. Preserve the reference's real-time safeguards.
- Use the approved embedding, not a standalone substitute or general IPC/UI framework.

### Scope boundary

No mode controls, automation, styling, analyzer integration or full Identity certification. VST3, multi-instance/failure qualification and resource measurements remain completion requirements, not shell tasks.

### Done when

Provide the built artifact and exact build/load/open/close steps; the requester manually loads it in Bitwig and performs the host checks. Automated Bitwig loading is not required. Record binary/host versions, display backend and runtime dependencies. In Bitwig, demonstrate stereo pass-through, zero reported latency, repeated editor open/close and clean shutdown. Record unsupported behavior and a proceed/revise/stop recommendation; a negative result may answer the experiment but blocks dependent implementation.

### Review

Requester tries the shell and reviews embedding, dependencies and distribution constraints before more infrastructure. Decide whether to proceed to Identity, revise or stop. Evidence applies only to the tested host/backend, not universal Linux compatibility. Stop after this checkpoint; host verification is performed manually by the requester.

## Completion contract

Product requirements retained from the original plan, not another implementation slice.

### Outcome

All eight reference modes are reproducibly configurable through the host or embedded editor.

### Guarantees

- Implement [reference.md](reference.md), including editor-independent processing, reset/history rules, numerical boundaries and zero compensation latency with intentional Delay samples shown separately.
- Host parameters and GPUI agree on mode/effective values. Use stable parameter IDs/units and versioned state; recall every mode's settings without the editor, clearing signal history. New instances start in Identity.
- Provide editor and host-triggerable reset without changing settings; document approved edge/event semantics for analyzer use. Host reset/activation provide the same zero-state guarantee.
- Editor closure/failure cannot alter DSP; instances remain independent.
- Publish algorithm/version/parameter reference, build/install instructions, identifiable CLAP/VST3 artifacts, runtime dependencies and reproducible host/analyzer checks.

### Done when

- Independent expected-value tests cover every reference guarantee, including event/block invariance and numerical boundaries. Exercise Gain, Delay, Low-pass, nonlinear, compressor and matrix configurations through plugin adapters against the same independent oracle.
- Real-host evidence covers both formats: stereo audio, embedded editor, gestures/readback, reset, persistence, reported zero latency, actual delay, repeated editor lifecycle, two simultaneous instances, renderer failure and clean shutdown.
- Verify focus/keyboard, numeric entry, scaling, resizing and automation feedback. Record cold/warm opening time, input-to-visible-update latency, idle CPU and per-instance memory/GPU costs on this machine; review before broad GPUI adoption.
- Through existing `analyse/` CLAP hosting, reproduce supported Identity/Gain/Delay/Low-pass checks and zero reported latency. No analyzer-native editor or VST3 hosting additions; do not claim unimplemented harmonic/dynamics/VST3 analyzer coverage. Record limitations and untested hosts/backends.

Deferred: analyzer development, native Wayland, other OSes, mono/multichannel, float64 host audio, raw-coefficient editing unless requested, standalone devices, oversampling and broader compatibility certification.

## GPUI tradeoffs informing the checkpoint

- Native Rust/GPU rendering avoids a web runtime but needs working graphics/drivers. Zed documents Linux Vulkan startup/performance failures; test the pinned GPUI build, not assumptions from Zed's standalone performance.
- Plugin editors cooperate with host-owned windows/event loops. The local X11 helper owns GPUI's loop outside the DAW; it is a native embedded UI, not a browser.
- Helper isolation can contain renderer failures, not every plugin/driver failure or malicious code. Costs include packaging, IPC, startup, resources and lifecycle synchronization.
- The adapter accepts X11 parent handles. XWayland may work on Wayland desktops but is not native Wayland embedding; verify each host/backend. Do not mandate this adapter for future environments without evidence.
- Custom rendering does not guarantee native-widget focus, accessibility, input-method or scaling behavior. Check used controls; pin versions and recheck lifecycle/graphics on upgrades.

Sources: [GPUI](https://www.gpui.rs/), [Zed Linux troubleshooting](https://zed.dev/docs/linux), `nam-linux-port/src/editor.rs`. Zed's experimental [embedded_gpui](https://github.com/zed-industries/embedded_gpui) embeds guests in GPUI hosts, not evidence of arbitrary DAW embedding.
