# AudioTestReference

Experimental Rust CLAP plugin with Identity, Gain, Delay and a bundled X11 GPUI editor. It processes stereo float32 at 44,100–192,000 Hz and reports zero compensation latency. Delay Time (`delay_samples`) controls intentional signal delay from 0 to 192,000 whole samples, default 0. DSP works without the editor. See the [development plan](../docs/features/audio-test-reference/plan.md) and [processing contract](../docs/features/audio-test-reference/reference.md).

This build applies parameter changes at block boundaries, not sample offsets. Delay history clears on mode or length changes, lifecycle reset, and successful state recall. There is no separate explicit reset control.

## Build

Use stable Rust, a linker/C compiler, pkg-config and Linux development libraries for X11/XCB, xkbcommon/X11, fontconfig and FreeType.
For example on Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libx11-dev libxcb1-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libfontconfig1-dev libfreetype6-dev \
  libvulkan1 mesa-vulkan-drivers
cd audiotestreference
./scripts/bundle.sh                  # debug-profile checkpoint artifact
# ./scripts/bundle.sh --release      # optional; not the recorded tested artifact
```

Cargo needs network access on the first build unless dependencies are cached.
Cargo.lock pins the resolved dependency graph.
To reuse this workspace's existing cache without depending on another plugin's code:

```sh
CARGO_TARGET_DIR=../nam-linux-port/target ./scripts/bundle.sh
```

Both variants place these files in **`audiotestreference/target/bundled/`**:

- `audio-test-reference.clap`
- `audio-test-reference-editor` (must remain executable and beside the module)
- `SHA256SUMS`

The script also copies the plugin and helper to `~/.clap/`. It does not change DAW settings.
The bundle is relocatable as a pair, not statically self-contained.
Runtime needs glibc/libm/libgcc, X11/XCB and xkbcommon libraries, fontconfig/FreeType, installed fonts, the Vulkan loader and a working presentation-capable Vulkan driver.
Font/graphics libraries can load dynamically and therefore do not all appear in `ldd`.
No NAM model, audio device, browser runtime or sibling plugin is required.
Preserve `LICENSE` and dependency license notices when redistributing; this experiment is not a distribution certification.

## Manual Bitwig checkpoint

No Bitwig host behavior has been verified by the implementation agent.
The installed package is Bitwig Studio **6.1.4-1**; confirm the running version in Bitwig's About dialog.
Use an X11 session, or a host that supplies X11 parents under XWayland; native Wayland embedding is unsupported.

1. Build and install the pair with the script above. If `~/.clap/` is not in Bitwig's CLAP search paths, add it or use the absolute `target/bundled` directory instead.
   Avoid duplicate installations; do not move or rename the helper independently.
2. Start Bitwig normally, rescan plugins, and insert **AudioTestReference (Shell Experiment)** on a stereo audio track in a disposable project.
   Do not save a production session against this experimental ID.
3. Feed known stereo host audio at 44.1 or 48 kHz, preferably with different left/right content and conservative monitoring levels.
   Confirm both channels pass unchanged using the host's meters/listening or a dry-vs-processed null route.
   Record the method and result; listening alone is not bit-exact certification.
4. Inspect Bitwig's reported plugin latency and record **0 samples** or the actual result.
5. Open the custom editor using Bitwig's plugin-window control.
   Confirm the GPUI panel appears **inside the host editor**, not as a separate application window.
   Select Gain and change its value. Select Delay and drag the Delay Time slider. Change Delay Time from the host too. Check that the host and editor agree on the mode and values.
6. Close and reopen the editor at least three times while audio plays.
   Confirm audio continues, the child window disappears on close, and no helper remains after the last close.
   On Linux, `ps -eo pid,args | grep '[a]udio-test-reference-editor --parent'` can inspect helpers without controlling the UI.
7. Remove the plugin, close the disposable project and quit Bitwig normally.
   Record crashes, stuck windows, audio interruptions, stderr errors, and orphan helpers, including negative results.
8. Record the running Bitwig version, session/backend, audio rate, display scaling and GPU/driver with the results.

If the panel is blank, launch Bitwig from a terminal and capture inherited stderr.
A missing/unlaunchable helper, unsupported parent, or renderer failure reports outside the audio callback and does not change DSP.
The plugin still advertises its editor when the helper is missing, so a blank host editor is possible.
The pinned wrapper maps at attachment; CLAP `show` and `hide` return false.
This is a known compatibility gap, not a successful visibility contract.

## Checks without Bitwig

```sh
cargo test --locked --lib
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
python3 tests/clap_smoke.py target/bundled/audio-test-reference.clap
# Optional: creates a temporary X11 test parent; no DAW, input injection or host settings.
timeout 75s python3 tests/gui_smoke.py target/bundled/audio-test-reference.clap
```

The ABI smoke checks stereo float32 ports, supported rates, zero latency, selected finite/nonfinite fixtures, separate/in-place buffers, lifecycle, and Delay timing and history across blocks.
The optional GUI probe attaches and destroys the helper through CLAP three times. To test sliders, pass `--gain-edit` or `--delay-edit` on an isolated Xvfb display with `xdotool` and a Vulkan driver that can present there, such as SwiftShader.
Neither proves Bitwig integration, exhaustive Identity, concurrent processing/editor behavior, failure recovery, resource budgets or arbitrary-host compatibility.

Implementation: `src/domain.rs` owns framework-free sample/rate rules; `src/lib.rs` adapts CLAP through NIH-plug; `src/editor.rs` owns helper lifecycle; `src/bin/editor.rs` and `src/embedding.rs` own GPUI/X11.
