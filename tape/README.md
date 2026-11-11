# OXIDE · Tape Works

A Rust stereo tape-coloration plugin and an analog tape-deck interface built with GPUI.

**Status:** CLAP/VST3 processing and the standalone GPUI audition app are implemented. The GPUI window is **not embedded in the plugin**. DAWs use their generic parameter editor. Standalone controls do not control a loaded plugin instance.

## Run the tape deck

```sh
cd tape
cargo run --locked --no-default-features --features ui --bin oxide-ui
```

Press **PLAY DEMO** to audition a quiet synthesized stereo chord through the same DSP used by the plugin. The app opens only the default audio output; it never requests microphone access. Playback starts muted. Meters show actual audition output. Demo output has a protective ±0.15 peak clamp; plugin output does not.

- Drag knobs vertically or scroll to adjust.
- Hold Shift while dragging for fine adjustment.
- Double-click a knob to reset it.
- Switch between 15 and 30 IPS for different saturation and bandwidth.
- Compare processed and dry audio with **BYPASS**.

The UI remains available if no audio output device can open. Device failure details appear in the terminal. The audition source is built in; audio-file loading and live input are not implemented.

## Build CLAP and VST3

```sh
cargo xtask bundle oxide-tape --release
```

Find `oxide-tape.clap` and `oxide-tape.vst3` in `target/bundled/`. Copy the bundles into your DAW's plugin directories and rescan.

| Platform | CLAP | VST3 |
| --- | --- | --- |
| Linux | `~/.clap/` | `~/.vst3/` |
| macOS | `~/Library/Audio/Plug-Ins/CLAP/` | `~/Library/Audio/Plug-Ins/VST3/` |
| Windows | `%COMMONPROGRAMFILES%\CLAP\` | `%COMMONPROGRAMFILES%\VST3\` |

The plugin supports a stereo input/output layout at 8–384 kHz. Formats are **VST3**, not legacy VST2. Host automation and preset/state persistence use NIH-plug. The UI and plugin can build independently.

## Controls

| Control | Range | Default |
| --- | --- | --- |
| Record level / drive | 0 to +24 dB | +6 dB |
| Tone | −1 (dark) to +1 (bright) | Neutral |
| Wow / flutter | 0–100% | 15% |
| Output | −24 to +6 dB | −6 dB |
| Blend | 0–100% wet | 100% |
| Tape speed | 15 / 30 IPS | 15 IPS |

Output gain follows the blend. For transparent dry audio, set blend to 0% and output to 0 dB. Continuous parameters smooth over approximately 20 ms. The creative tape model combines soft saturation, a broad tone shelf, speed-dependent low-pass filtering, and subtle shared transport modulation. It does not simulate a particular tape machine. It currently lacks oversampling, so strong high-frequency drive can alias.

## Development

```sh
cargo test --locked --no-default-features --lib  # portable DSP
cargo test --locked --features ui              # DSP + plugin + audition tests
cargo check --locked --features ui             # all adapters
cargo fmt --all -- --check
```

Use a recent stable Rust toolchain. GPUI needs a supported GPU or software Vulkan implementation on Linux. Linux builds also need ALSA, X11/Wayland, font, and keyboard development libraries. On Debian/Ubuntu, a typical starting set is:

```sh
sudo apt install build-essential pkg-config clang libasound2-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-dev libxcb1-dev libfontconfig1-dev libfreetype6-dev libvulkan1
```

On macOS, install Xcode command-line tools. On Windows, use the MSVC toolchain and Windows SDK. Only Linux builds have been checked in this environment. Runtime host compatibility and listening tests remain necessary.

Keep `Cargo.lock`: it pins NIH-plug and the GPUI dependency graph. In particular, `nix` 0.31.2 and `libc` 0.2.183 avoid an upstream compatibility break between GPUI's transitive `xattr` 0.2 dependency and newer `libc` releases.

## Layout

```text
src/features/saturation/domain.rs  Framework-independent, allocation-free DSP
src/features/saturation/plugin.rs  NIH-plug host adapter and parameter mapping
src/main.rs                        GPUI tape-deck presentation and interaction
src/preview.rs                     Output-only CPAL audition adapter
xtask/                             CLAP/VST3 bundle tooling
```

The domain has no GPUI, CPAL, or NIH-plug dependencies. Plugin processing allocates no memory and takes no locks.

## Remaining integration

Embedding GPUI requires a host-parented window adapter, host UI-thread lifecycle handling, and parameter gesture synchronization. GPUI 0.2.2 does not provide a ready-made NIH-plug editor integration. This project does not substitute a floating preview window for an embedded editor.

## License

GPL-3.0-or-later. See `LICENSE`. Review NIH-plug's VST3 licensing notes before distributing binaries.
