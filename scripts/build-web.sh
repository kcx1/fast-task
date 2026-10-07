#!/usr/bin/env bash
# Build the LAN share's browser client into src/local_share/web/ (app.js +
# app_bg.wasm), where the desktop app serves it from. Run before
# `cargo build --release`: release builds embed whatever is there.
#
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version <wasm-bindgen version in Cargo.lock>
set -euo pipefail
cd "$(dirname "$0")/.."

out=src/local_share/web
profile=${1:-release}

cargo build --bin fast-task --target wasm32-unknown-unknown --profile "$profile"
wasm-bindgen \
  --target web \
  --no-typescript \
  --out-name app \
  --out-dir "$out" \
  "target/wasm32-unknown-unknown/$profile/fast-task.wasm"

ls -lh "$out"/app.js "$out"/app_bg.wasm
