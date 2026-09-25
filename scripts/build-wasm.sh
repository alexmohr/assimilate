#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
#
# Builds crates/domain-wasm into frontend/src/wasm/generated/. The output is
# committed so the frontend builds without a Rust toolchain; CI rebuilds it
# and fails on any diff, so the build must be reproducible: the toolchain and
# wasm-bindgen-cli are pinned, and machine-specific paths are remapped.
# The host architecture also changes the emitted code, so on anything but
# x86_64 Linux (what CI runs) the build re-runs itself in an x86_64 container.
#
# Usage: scripts/build-wasm.sh
# Requires: on x86_64 Linux, rustup and wasm-bindgen-cli at WASM_BINDGEN_VERSION
#   (cargo install wasm-bindgen-cli --version <WASM_BINDGEN_VERSION> --locked);
#   anywhere else, Docker.
set -euo pipefail

TOOLCHAIN="nightly-2026-08-21"
# Must match the `wasm-bindgen` pin in crates/domain-wasm/Cargo.toml.
WASM_BINDGEN_VERSION="0.2.129"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
out_dir="$repo_root/frontend/src/wasm/generated"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"

if [[ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]]; then
  exec docker run --rm --platform linux/amd64 \
    -v "$repo_root":/src -w /src \
    -v assimilate-wasm-cargo:/usr/local/cargo/registry \
    -v assimilate-wasm-target:/tmp/target -e CARGO_TARGET_DIR=/tmp/target \
    rust:1-slim bash -c "
      cargo install wasm-bindgen-cli --version $WASM_BINDGEN_VERSION --locked --quiet &&
      scripts/build-wasm.sh"
fi

installed="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)"
if [[ "$installed" != "$WASM_BINDGEN_VERSION" ]]; then
  echo "error: wasm-bindgen-cli $WASM_BINDGEN_VERSION required (found: ${installed:-none})" >&2
  echo "  cargo install wasm-bindgen-cli --version $WASM_BINDGEN_VERSION --locked" >&2
  exit 1
fi

rustup toolchain install "$TOOLCHAIN" --no-self-update --profile minimal --target wasm32-unknown-unknown >/dev/null

cd "$repo_root"
RUSTFLAGS="--remap-path-prefix=$repo_root=/assimilate --remap-path-prefix=$cargo_home=/cargo" \
  cargo "+$TOOLCHAIN" build --locked -p domain-wasm --target wasm32-unknown-unknown --profile wasm

rm -rf "$out_dir"
wasm-bindgen --target web --omit-default-module-path --out-dir "$out_dir" \
  "$target_dir/wasm32-unknown-unknown/wasm/domain_wasm.wasm"
