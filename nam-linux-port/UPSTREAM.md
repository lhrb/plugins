# Vendored upstream sources

## NIH-plug lifecycle patch

Source: https://github.com/robbert-vdh/nih-plug, exact revision
`de421011f41a6d10fc8c7a6084e4f4dee0143683` (ISC license).
`vendor/nih-plug/` retains the root `Cargo.toml`, `LICENSE`, `src/`, and
`nih_plug_derive/` (manifest, source, and tests).
It contains no Git metadata, unrelated plugin examples, editor adapters, or bundler.
The root manifest's workspace members are reduced to the retained derive crate.
The application uses this local crate and its local derive dependency.
The build-only xtask remains pinned to upstream separately.

Only two Rust source files differ from that revision:

- `src/wrapper/clap/wrapper.rs`
- `src/wrapper/vst3/inner.rs`

Both `AsyncExecutor` callbacks in each wrapper capture `Weak<Wrapper>` instead
of `Arc<Wrapper>`, upgrade only while scheduling, and drop tasks if the wrapper
has already been destroyed.
This breaks the wrapper → editor → executor → wrapper ownership cycle without
changing GUI-thread scheduling or host behavior.
No show/hide behavior or native model sources are patched.
Queued application GUI tasks also hold weak GUI contexts so an unpumped task
cannot retain its wrapper through the host context.

Upstream master still pointed to this revision when checked.
The community-maintained successor (https://github.com/BillyDM/nih-plug,
now nice-plug), revision `93940e8d43c13c909510db089235f3045b02b6a4`, uses the
same weak-upgrade approach in both wrappers, but has incompatible package/API
changes; this snapshot deliberately avoids that migration.
To refresh, copy the retained paths from a fresh checkout of the exact desired
revision, reduce workspace members, reapply these four callback ownership fixes,
and rerun CLAP destruction and GUI dispatch smoke tests.
Preserve the ISC license and source headers when distributing.

# Official NAM core integration

## Pinned sources and licenses

| Component | Source | Exact revision/version | License |
| --- | --- | --- | --- |
| NeuralAmpModelerCore | https://github.com/sdatkinson/NeuralAmpModelerCore | `d3189ed482ded880c128bf312941266c4fa5ceec` | MIT, `vendor/NeuralAmpModelerCore/LICENSE` |
| Eigen | https://gitlab.com/libeigen/eigen | `bc3b39870ecb690a623a3f49149a358b95c5781d` (5.0.1; core's pinned submodule) | Primarily MPL-2.0; additional BSD/Apache/MINPACK notices in `Dependencies/eigen/COPYING.*` and source headers |
| nlohmann/json | Core's `Dependencies/nlohmann/json.hpp` | 3.12.0, exact file from core revision above | MIT, `Dependencies/nlohmann/LICENSE.MIT`; additional notices in header |

The JSON header SHA-256 is `aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63`.
The JSON license text comes from upstream tag `v3.12.0` (`LICENSE.MIT`).
Distributions must preserve notices and make the covered Eigen source available under its license.
Model files and their licenses remain the user's responsibility.

The vendor directory contains tracked source, not a submodule or nested Git repository.
NAM sources are unmodified.
The snapshot retains `NAM/`, root license/README/CMakeLists, `tools/CMakeLists.txt` for the upstream source list, Eigen's `Eigen/` and `unsupported/` headers and license texts, and nlohmann/json.
It omits examples, tools, tests, CI, Git metadata and unused dependencies.
AudioDSPTools is not compiled or vendored: the pinned core's own `NAM/wav.cpp` handles WAV loading and `NAM/linear.cpp` handles resampling.

To refresh, clone the official repository elsewhere, checkout the desired exact commit, initialize `Dependencies/eigen`, and copy only the retained paths listed above.
Update this document's revisions/hash and rerun both Rust and native tests.
Do not silently follow an upstream branch.

## Build

`build.rs` uses the `cmake` build dependency pinned by `Cargo.lock`.
The build needs CMake >=3.16, a C++20 compiler, and the Linux C++ standard library.
It performs no network access.
`native/CMakeLists.txt` compiles the same `NAM/*.cpp` and `NAM/*/*.cpp` source groups as upstream's tools, with `NAM_SAMPLE_FLOAT` and `NAM_ENABLE_A2_FAST` enabled.
The bridge uses float buffers throughout and preallocates its input buffer and channel-pointer arrays.
The static archive links whole because architectures register through static initializers.
No mock inference or Rust model reimplementation participates in processing.

## API and real-time contract

`features::model::NamModel::load(path, sample_rate, max_frames)` loads and prewarms a model.
It accepts official NAM files and mono/stereo PCM16/24/32 or float32 WAV IRs through upstream's loader.
Only one-input, one/two-output models are supported.
Mono output duplicates to both plugin channels.
WAV samples retain their unnormalized upstream gain.

`input_channels()`, `output_channels()`, `expected_sample_rate()`, `sample_rate()` and `supports_arbitrary_sample_rate()` expose model/host metadata.
Known mismatched rates fail unless the architecture advertises arbitrary-rate support.
Linear/WAV models resample their IR through upstream `Reset`; neural architectures without that support do not silently change pitch.
Unknown training rates remain `None`; the adapter cannot detect a mismatch for these legacy models.

Load, reset/prewarm and destruction belong on the control/worker thread, never the audio callback.
`NamModel` is `Send` but not `Sync`.
Its exclusive `process(input, left, right)` requires equal buffer lengths no larger than the prepared maximum.
The bridge copies into preallocated input storage because upstream takes mutable pointers.
Successful adapter processing allocates nothing.
Processing failures return static Rust messages without formatting or allocating in the bridge.
A core exception faults the instance until a successful off-thread reset or reload.
Outputs are unspecified on failure; the caller must replace them with silence or its chosen bypass signal.

C++ exceptions never cross the C ABI.
This is not a sandbox: upstream minimally validates NAM JSON and some constructors trust weight lengths/configuration.
Only load trusted local model files; malformed native model data can cause failures beyond catchable exceptions.

## Verification

Rust tests in `src/features/model/tests.rs` exercise real Linear identity/gain and WaveNet inference, repeated variable-sized blocks, mono duplication, reset, invalid/missing files, invalid buffers, unknown rates, unsupported channels, LSTM rate rejection, mono/stereo WAV convolution and WAV rate adaptation.
Run these with the parent crate's `cargo test`.

Native allocation regression:

```sh
cmake -S native -B /tmp/nam-native-build -DCMAKE_BUILD_TYPE=Release -DNAM_BRIDGE_TESTS=ON
cmake --build /tmp/nam-native-build -j2
ctest --test-dir /tmp/nam-native-build --output-on-failure
```

The Linux native test intercepts `new`, `malloc`, `calloc` and `realloc` during 2,000 variable-sized blocks each for direct Linear, 4096-tap FFT Linear and LSTM.
These representative checks do not establish allocation behavior or CPU budgets for every upstream architecture/model.
