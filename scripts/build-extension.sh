#!/usr/bin/env bash
# Build the rex wasm engine and the Chrome extension.
#   scripts/build-extension.sh          -> extension/dist (normal build)
#   REX_E2E=1 scripts/build-extension.sh -> extension/dist with test-only host access
# Requires: rustup target wasm32-unknown-unknown, wasm-bindgen CLI 0.2.123, pnpm, protoc.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
CARGO_INCREMENTAL=0 cargo build -q -p rex-wasm --target wasm32-unknown-unknown --profile wasm
wasm-bindgen --target web --out-dir extension/wasm target/wasm32-unknown-unknown/wasm/rex_wasm.wasm
cd extension
pnpm install --silent
./node_modules/.bin/tsc --noEmit -p tsconfig.json
if [[ "${REX_E2E:-}" == "1" ]]; then pnpm -s run build:e2e; else pnpm -s run build; fi
