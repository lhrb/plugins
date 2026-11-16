# Slice 1 CLAP probe

Headless Linux hosting experiment for explicitly selected, trusted CLAP binaries.
No discovery, audio device, UI, measurement application, or general host framework.

```sh
# Repository root; build the existing plugin without changing its source/lock.
(cd tape && cargo xtask bundle oxide-tape --release)
cargo build --manifest-path analyse/Cargo.toml --locked --offline

analyse/target/debug/clap-probe inspect \
  /home/lukas/workspace/plugins/tape/target/bundled/oxide-tape.clap \
  com.tapeworks.oxide 5000
analyse/target/debug/clap-probe inspect \
  /usr/lib/clap/CHOWTapeModel.clap org.chowdsp.CHOWTapeModel 5000

mkdir -p analyse/runs/final
# Fault injection intentionally aborts one worker; disable core files.
(ulimit -c 0; analyse/target/debug/clap-probe run \
  /home/lukas/workspace/plugins/tape/target/bundled/oxide-tape.clap \
  com.tapeworks.oxide 1141971201 0.4 \
  /usr/lib/clap/CHOWTapeModel.clap \
  org.chowdsp.CHOWTapeModel 3188564941 0.4 \
  analyse/runs/final 5000 > analyse/runs/final/transcript.log 2>&1)
```

Replace OXIDE's absolute path if the checkout moves.
`run` takes path, plugin ID, parameter ID, and plain CLAP value for each slot, then an existing output directory and per-command timeout in milliseconds.
The experiment sequence and labels intentionally target this pair.
Output paths must not contain whitespace.
Use a new output directory to preserve earlier runs; existing render filenames are overwritten.

Each slot stays loaded in its own worker.
Additional workers provide same-plugin history controls.
Reset trials use 64 preconditioning blocks and 128 recorded blocks at stereo float32/48 kHz/256 frames.
Continuations have no reset or preconditioning.
Raw files contain interleaved little-endian float32 L/R samples without normalization or delay compensation.
`input.f32le` includes both preconditioning and recorded input.

Only `1e-6` absolute peak error defines the declared repeat tolerance.
`COMPARE ... within=false` records variation, including intentional parameter/history effects; it does not make the process exit unsuccessfully.
Lifecycle, configuration, readback, settings isolation, process, and recovery errors do fail the run.
Read the transcript and [experiment](../docs/features/plugin-inspector/experiment.md), not just its exit code.

```sh
cargo test --manifest-path analyse/Cargo.toml --locked --offline
cargo check --manifest-path analyse/Cargo.toml --locked --offline
cargo clippy --manifest-path analyse/Cargo.toml --locked --offline --all-targets -- -D warnings
cargo fmt --manifest-path analyse/Cargo.toml -- --check
```

Workers isolate process failure, not filesystem/network access or malicious code.
Plugin initialization can read local preferences.
No plugin state save/load, editors, timers, dynamic restart, or alternate bus configurations are implemented.
CHOWTape reset-only repeatability is **not** established; see the recorded limitation.
Stop at slice 1.
