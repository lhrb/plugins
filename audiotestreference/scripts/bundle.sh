#!/usr/bin/env bash
# Build only; never installs plugins or changes host preferences.
set -euo pipefail
cd "$(dirname "$0")/.."
profile=debug
flags=()
if [[ ${1:-} == --release && $# == 1 ]]; then
  profile=release
  flags=(--release)
elif [[ $# != 0 ]]; then
  echo "Usage: $0 [--release]" >&2
  exit 2
fi
target=$(realpath -m "${CARGO_TARGET_DIR:-target}")
cargo build --locked --target-dir "$target" --lib --bin audio-test-reference-editor "${flags[@]}"
mkdir -p target/bundled
install -m755 "$target/$profile/libaudio_test_reference.so" target/bundled/audio-test-reference.clap
install -m755 "$target/$profile/audio-test-reference-editor" target/bundled/audio-test-reference-editor
(cd target/bundled && sha256sum audio-test-reference.clap audio-test-reference-editor > SHA256SUMS)
printf 'Built %s/target/bundled/{audio-test-reference.clap,audio-test-reference-editor}\n' "$PWD"

mkdir -p "$HOME/.clap"
install -m755 target/bundled/audio-test-reference.clap \
    target/bundled/audio-test-reference-editor "$HOME/.clap/"
