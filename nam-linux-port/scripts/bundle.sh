#!/usr/bin/env bash
# Build relocatable bundles. This does not install into the user's plugin folders.
set -euo pipefail
cd "$(dirname "$0")/.."
profile=release
flags=(--release)
if [[ ${1:-} == --debug ]]; then
  profile=debug
  flags=()
elif [[ $# != 0 ]]; then
  echo "Usage: $0 [--debug]" >&2
  exit 2
fi
cargo build --locked --bin nam-linux-editor "${flags[@]}"
cargo xtask bundle nam-linux-port "${flags[@]}"
# The CLAP shared object and helper travel together.
install -m755 "target/$profile/nam-linux-editor" target/bundled/nam-linux-editor
# VST3 carries its helper beside its module inside the bundle.
while IFS= read -r -d '' module; do
  install -m755 "target/$profile/nam-linux-editor" "$(dirname "$module")/nam-linux-editor"
done < <(find target/bundled/nam-linux-port.vst3 -type f -name '*.so' -print0)
printf '\nBuilt target/bundled/nam-linux-port.clap + nam-linux-editor\nBuilt target/bundled/nam-linux-port.vst3/\n'
