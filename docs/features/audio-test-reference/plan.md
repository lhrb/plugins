# AudioTestReference

Goal: Build a Linux Rust CLAP/VST3 reference plugin with eight reproducible stereo modes for developing `analyse/`. The [processing contract](reference.md) defines the algorithms and guarantees. Keep DSP independent of the plugin APIs and GPUI. Do not change `analyse/` or depend on another plugin's internals.

## Status

Slices 1–4 are implemented: CLAP shell with embedded GPUI, Identity, Gain, and Delay. Gain and Delay have host and editor controls. The requester accepted the shell in Bitwig. Existing checks cover Identity/Gain fixtures, CLAP smoke, and an optional GPUI-to-CLAP Gain edit probe. Delay host qualification remains open.

Low-pass is **in progress**. `audiotestreference/src/domain/low_pass.rs` defines and tests coefficient calculation and validation, and defines per-channel history. It does not yet apply the filter equation or connect Low-pass to the domain processor, host parameters, or editor.

The bundled GPUI helper embeds through X11/XWayland; native Wayland is deferred. The pinned NIH-plug wrapper maps at attachment and returns false from CLAP `show`/`hide`; hosts requiring those callbacks are unsupported. XWayland is not qualified. The current CLAP identity (`org.audio-test-reference.shell-experiment`) is experimental, not a production session compatibility promise.

## Next: 5 — Finish Low-pass

Depends on the existing domain processor and host/editor wiring. Status: **In progress**.

**Outcome:** Select Low-pass and adjust cutoff/Q in the host or editor to hear the contracted stereo biquad response.

- Apply the [Low-pass difference equation](reference.md#low-pass) to each channel with independent x/y history. Wire the mode into the domain processor. Cutoff/Q changes preserve history; mode changes and lifecycle resets clear it. The domain and editor reject invalid updates without changing the active configuration. Keep the pinned CLAP wrapper’s existing range clamping at host ingress; strict rejection of malformed host events is not part of this slice.
- Expose mode and cutoff/Q through host parameters and the editor. Show effective cutoff as a fraction and in Hz, and show the normalized coefficients read-only.
- Preserve existing modes, parameter IDs, zero reported latency, and real-time callback guarantees.

**Done when:** Independent impulse and frequency-response checks verify the filter, stereo isolation, and history behavior across parameter changes and resets. Run existing mode regressions. In Bitwig, verify Low-pass selection, host/editor cutoff/Q edits and readback, and coefficient display.

**Review:** Inspect the response checks and Bitwig controls before moving on to other modes.

**Deferred contract work:** The CLAP adapter still reads parameters per block (`SAMPLE_ACCURATE_AUTOMATION = false`), so sample-offset conformance is not verified. Keep that work and explicit reset separate from finishing Low-pass. Before implementing explicit reset, agree how the host or analyzer triggers it, including offset, repeat and re-arm semantics. The existing host lifecycle reset is separate.

## Later (provisional)

| Slice | Outcome | Depends on |
| --- | --- | --- |
| 6 | Harmonics | 3 |
| 7 | Clipper | 3 |
| 8 | Linked stereo Compressor | 4 |
| 9 | Stereo Matrix | 3 |
| 10 | Refine workflow, controls and state recall | 4–9 |
| 11 | Qualify formats, hosts and analyzer integration | 10 |

Qualification still needs independent Delay checks for asymmetric stereo impulses, zero and maximum delay, unchanged length, resets, state recall, and sample-offset events across block partitions. In Bitwig, verify host/editor readback, measured delay, and zero reported latency.

Completion requires versioned state and documented algorithms, controls and reset semantics; identifiable CLAP/VST3 bundles with build/install instructions; and independent DSP and adapter verification. Check both formats in real hosts for automation/readback, reset, persistence, latency, editor lifecycle and failure, concurrent instances and clean shutdown. Check focus, numeric entry, scaling and resizing. Measure cold/warm editor opening, input-to-visible latency, idle CPU and per-instance memory/GPU cost before tuning or expanding GPUI adoption; the requester noticed an opening delay in Bitwig. Reproduce supported Identity/Gain/Delay/Low-pass and zero-latency checks through existing `analyse/` CLAP hosting. Record unsupported and untested configurations; no analyzer changes, native Wayland or broader platform qualification in scope.
