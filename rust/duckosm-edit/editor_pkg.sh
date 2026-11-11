#!/usr/bin/env bash
# The level editor's WebAssembly package (card 8 of `duckosm studio`): roadstyle's build_editor.sh, run in the roadstyle checkout of the pin
# (rust/duckosm-edit/Cargo.toml). roadstyle-serve's build.rs then builds the package into the program (`roadstyle_serve::embedded`,
# level_editor/pkg/). Run it before `cargo build -p duckosm-cli` or the Python extension. Needs the target wasm32-unknown-unknown and
# wasm-bindgen of the version in roadstyle's Cargo.lock (with none, this script downloads it). Without the package, card 8 says so.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
cargo fetch --manifest-path "$here/Cargo.toml" >/dev/null
manifest=$(cargo metadata --format-version 1 --manifest-path "$here/Cargo.toml" | python3 -c "import json, sys
print(next(p['manifest_path'] for p in json.load(sys.stdin)['packages'] if p['name'] == 'roadstyle-serve'))")
root="$(cd "$(dirname "$manifest")/../.." && pwd)"           # the roadstyle checkout (rust/roadstyle-serve/Cargo.toml)
cargo fetch --manifest-path "$root/rust/Cargo.toml" >/dev/null
v=$(grep -m1 -A1 '^name = "wasm-bindgen"$' "$root/rust/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')
if [ "$(wasm-bindgen --version 2>/dev/null | cut -d' ' -f2)" != "$v" ]; then
  bin="${TMPDIR:-/tmp}/wasm-bindgen-$v-x86_64-unknown-linux-musl"
  [ -x "$bin/wasm-bindgen" ] || curl -sSfL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$v/wasm-bindgen-$v-x86_64-unknown-linux-musl.tar.gz" | tar xz -C "${TMPDIR:-/tmp}"
  export PATH="$bin:$PATH"
fi
bash "$root/rust/roadstyle-wasm/build_editor.sh"
