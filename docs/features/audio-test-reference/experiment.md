# Slice 1 experiment — accepted

## Recommendation

**Proceed to planning Slice 2.** The requester accepts the working shell and explicitly waives further checkpoint investigation. Slice 1 is Done; no later implementation has started.
The CLAP shell builds and the actual bundled GPUI helper attaches/detaches under a synthetic X11 parent.
The requester reports successful Bitwig loading, text in a floating editor window, stereo pass-through, editor open/close and clean removal/host shutdown.
A floating host-owned plugin window is compatible with embedding; placement inside that window still needs confirmation.
The requester also confirms uninterrupted audio during repeated editor open/close cycles.
Host-displayed latency remains unconfirmed.

**Consequential compatibility gap:** pinned NIH-plug returns **false from both CLAP `show` and `hide`**.
It maps the editor during `set_parent` instead.
Three successful attach/destroy cycles do not satisfy the CLAP visibility contract.
If Bitwig depends on those callbacks, revise or stop this experiment before dependent implementation.
No dependency patch or wider host qualification was attempted.

## Build and environment observed — 2026-11-17

- Linux `7.2.15-rt3-arch1-1-rt`, x86_64, glibc `2.44+r78+gfcfb867c58ab-1`.
- Rust `1.100.0 (0ef1bc386 2026-11-09)`; Cargo `1.100.0 (3acc79c0c 2026-11-05)`.
- Installed Bitwig package `6.1.4-1`; executable `/usr/bin/bitwig-studio`. The agent did not launch Bitwig or confirm its running About version.
- `XDG_SESSION_TYPE=x11`, `DISPLAY=:0`, `WAYLAND_DISPLAY` unset. XWayland/native Wayland were not tested.
- GPUI `0.2.2`, X11 + font-kit only, no default features; x11rb `0.13.2`; libc `0.2.183`.
- NIH-plug `de421011f41a6d10fc8c7a6084e4f4dee0143683` (unmodified upstream); clap-sys `25d7f53fdb6363ad63fbd80049cb7a42a97ac156`, CLAP 1.2.2. VST3 disabled. Cargo.lock records the complete graph.
- pkg-config: X11 `1.8.14`, XCB `1.17.0`, xkbcommon/xkbcommon-x11 `1.13.2`, fontconfig `2.18.3`, FreeType `26.7.20` (pkg-config ABI version; installed FreeType package `2.15.0-1`).
- Vulkan loader `1.4.363`; AMD Radeon RX 6700 XT, RADV NAVI22, Mesa `26.2.7-arch1.1`, Vulkan device API `1.4.354`.
- Xvfb and vulkaninfo available. Approximately 35 GiB free before build. Reused `nam-linux-port/target` as a Cargo artifact cache only; initial build still recompiled dependencies, increasing it from about 3.2 to 4.9 GiB. No NAM source/dependency changes.

## Commands and results

Run in `audiotestreference/` unless stated otherwise.

| Command/check | Result |
| --- | --- |
| `rustc --version`; `cargo --version`; pkg-config; `vulkaninfo --summary`; package queries | Prerequisites available; versions above |
| `CARGO_TARGET_DIR=../nam-linux-port/target ./scripts/bundle.sh` | Passed; debug-profile module + helper, copied into this experiment's bundle |
| `CARGO_TARGET_DIR=../nam-linux-port/target cargo test --locked --lib` | 2 passed: finite-bit/nonfinite safety fixtures and rate boundaries |
| `CARGO_TARGET_DIR=../nam-linux-port/target cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `cargo fmt --check` | Passed |
| `python3 tests/clap_smoke.py target/bundled/audio-test-reference.clap` | Passed: actual CLAP factory, experimental ID/version output, stereo float32-only port advertisement, accepted/rejected rates, zero latency, asymmetric sample fixtures, nonfinite replacement, separate/in-place buffers, start/reset/stop/deactivate/destroy |
| `timeout 90s xvfb-run -a -s '-screen 0 1280x800x24' python3 tests/gui_smoke.py target/bundled/audio-test-reference.clap` | Failed: hardware-only driver could not present on Xvfb (`No DRI3 support`, `Failed to create surface: PlatformNotSupported`); no embedded child appeared. No software Vulkan ICD installed |
| `timeout 75s python3 tests/gui_smoke.py target/bundled/audio-test-reference.clap` on `:0` | Passed: plugin launched its sibling helper; actual X11 child appeared under temporary test parent and disappeared on destroy, three cycles at 1×. No DAW launched, input injected, or host settings changed. Probe waited for initial frame submission but did not visually certify contents |
| `ps -eo pid,args` after successful probe | No `audio-test-reference-editor --parent` process remained |
| `ldd` on both artifacts | All linked libraries resolved; see runtime dependencies below |

Cargo reports a dependency future-compatibility warning for `proc-macro-error2 2.0.1`; compilation and Clippy pass today.
The callback code contains only slice iteration, finite sample replacement and one conditional relaxed atomic fault store.
No runtime allocation/lock instrumentation or full wrapper real-time certification was performed.
The status writer/reader, process launch, X11 operations, renderer and logging all run outside DSP.

## Artifacts

Debug-profile build, plugin/helper version `0.1.0`, CLAP ID `org.audio-test-reference.shell-experiment`:

```text
/home/lukas/workspace/plugins/audiotestreference/target/bundled/audio-test-reference.clap
  5.1 MiB; SHA-256 b45c6324fe596b853c82a1486743f59e346079be915c65d4c4a697d43dfdc675
/home/lukas/workspace/plugins/audiotestreference/target/bundled/audio-test-reference-editor
  69 MiB; SHA-256 7dbbe16e6a91e12c1b8c4cf0e31d866ba01d47dab047c30d1eda52f2520490da
```

`target/bundled/SHA256SUMS` identifies the built pair; build output is intentionally gitignored.
The module directly links libc, libm and libgcc_s.
The helper additionally links xkbcommon, xkbcommon-x11, xcb, xcb-xkb, Xau and Xdmcp.
GPUI also needs dynamically loaded X11/font/graphics facilities, installed fonts and a working Vulkan loader/driver with X11 presentation.
The bundled helper is native Rust/GPUI, not a browser and not a standalone substitute.
The pair is relocatable together; neither is a universal Linux binary or statically bundled graphics stack.

## Manual Bitwig evidence to fill in

Follow the exact [build/install/load/open/close instructions](../../../audiotestreference/README.md#manual-bitwig-checkpoint).

- Running host version/About: **pending**.
- Session, host window backend, GPU/driver, display scaling, sample rate: **pending**.
- Plugin discovery/instantiation: **passed, requester-reported**; plugin loaded successfully.
- Stereo pass-through: **passed, requester-reported**; measurement method not specified.
- Host-reported latency: **pending**; requester could not locate the host readout. Automated CLAP query returned zero; the panel's static label is not host evidence.
- Editor opening/content: **passed, requester-reported**; text appears in a floating plugin window, not the device panel. Whether the window is host-owned remains unconfirmed.
- Repeated editor open/close during playback: **passed, requester-reported**; audio continued without interruption. Exact cycle count not specified.
- Opening responsiveness: requester notices a short click-to-window delay and requests the fastest practical opening. No duration measured; optimization is not part of this checkpoint.
- Remove instance/normal host shutdown: **passed, requester-reported**, no problems reported. Orphan helper inspection not confirmed.
- Requester's proceed/revise/stop decision: **proceed**. Accept the successful manual use and automated zero-latency query without requiring a screenshot, further embedding confirmation or the Bitwig latency readout. These remain evidence limitations, not verified claims.

Remaining gaps are intentional: arbitrary-host visibility/focus/scaling/resizing, XWayland, native Wayland, two instances, renderer-failure qualification, performance/resource measurements, full Identity/reset/state certification, later modes, automation and VST3.
Helper exit leaves DSP unchanged by construction, but the host may retain a blank panel until it closes/recreates the editor; automatic renderer recovery is not implemented.
Do not infer GUI/audio concurrency or Bitwig acceptance from sequential synthetic tests.
Slice 1 is **Done** following requester acceptance of these evidence limitations. Stop implementation here; plan Slice 2 separately.
