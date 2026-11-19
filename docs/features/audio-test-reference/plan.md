# AudioTestReference

Goal: Build a Linux Rust CLAP/VST3 reference plugin with eight reproducible stereo modes for developing `analyse/`. The [processing contract](reference.md) defines the algorithms and guarantees. Keep DSP independent of the plugin APIs and GPUI. Do not change `analyse/` or depend on another plugin's internals.

## Status

Slices 1–3 are implemented: CLAP shell with embedded GPUI, Identity, and Gain with host parameters and editor edits routed back to the host. The requester accepted the shell in Bitwig. Existing checks cover Identity/Gain fixtures, CLAP smoke, and an optional GPUI-to-CLAP Gain edit probe. Full host qualification remains open. The CLAP adapter currently reads parameters per block (`SAMPLE_ACCURATE_AUTOMATION = false`); that does not meet the sample-offset contract for stateful modes.

The bundled GPUI helper embeds through X11/XWayland; native Wayland is deferred. The pinned NIH-plug wrapper maps at attachment and returns false from CLAP `show`/`hide`; hosts requiring those callbacks are unsupported. XWayland is not qualified. The current CLAP identity (`org.audio-test-reference.shell-experiment`) is experimental, not a production session compatibility promise.

## Next: 4 — Delay and reproducible reset

Depends on Slice 3. **Open decision:** agree how the host or analyzer issues an explicit reset, including offset, repeat and re-arm semantics, before implementing that control. This is separate from the existing host lifecycle reset callback.

- Add stereo Delay with integer `N` (0–192,000 samples, default 0). Expose the effective value in the host and editor. Report zero compensation latency; show `N` separately as intentional signal delay.
- Provide explicit reset from the host and editor. Apply events and clear histories as specified in [the processing contract](reference.md). Mode changes and explicit reset clear all history; changing `N` clears only delay history, even while inactive. An unchanged `N` does not clear a running delay. Reset preserves settings; state recall restores settings but not history.
- Keep the audio callback free of allocation, locks, I/O and editor dependencies. Preserve Identity/Gain behavior and parameter IDs.

**Done when:** Independent exact vectors verify asymmetric stereo impulses, `N=0`, maximum `N`, and mid-block mode/length/reset events across different block partitions. Verify unchanged-value events, activation, lifecycle reset, state recall and the agreed explicit reset control. Exercise the CLAP adapter with the same expectations. In Bitwig, check host/editor readback, repeatable reset, zero reported latency and measured `N`-sample delay. Review that evidence before starting Low-pass.

## Later (provisional)

| Slice | Outcome | Depends on |
| --- | --- | --- |
| 5 | Low-pass with cutoff/Q and read-only coefficients | 4 |
| 6 | Harmonics | 3 |
| 7 | Clipper | 3 |
| 8 | Linked stereo Compressor | 4 |
| 9 | Stereo Matrix | 3 |
| 10 | Refine workflow, controls and state recall | 4–9 |
| 11 | Qualify formats, hosts and analyzer integration | 10 |

Completion requires versioned state and documented algorithms, controls and reset semantics; identifiable CLAP/VST3 bundles with build/install instructions; and independent DSP and adapter verification. Check both formats in real hosts for automation/readback, reset, persistence, latency, editor lifecycle and failure, concurrent instances and clean shutdown. Check focus, numeric entry, scaling and resizing. Measure cold/warm editor opening, input-to-visible latency, idle CPU and per-instance memory/GPU cost before tuning or expanding GPUI adoption; the requester noticed an opening delay in Bitwig. Reproduce supported Identity/Gain/Delay/Low-pass and zero-latency checks through existing `analyse/` CLAP hosting. Record unsupported and untested configurations; no analyzer changes, native Wayland or broader platform qualification in scope.
