#!/usr/bin/env bash
# 構建網頁版到 dist/
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p game --target wasm32-unknown-unknown
rm -rf dist && mkdir -p dist
wasm-bindgen --target web --no-typescript --out-dir dist/pkg target/wasm32-unknown-unknown/release/game.wasm
cp web/* dist/
