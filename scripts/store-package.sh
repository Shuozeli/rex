#!/usr/bin/env bash
# The ZIP to upload to the Chrome Web Store (store/README.md): the wasm engine, the embedded recipes and the store
# build, made with the committed config (config.example.json), never a machine's config.local.json. Checks the
# manifest (no host access, CSP connect-src 'self', no test hooks, recipes packaged) and writes
# target/store/rex-extension-<version>.zip with its sha256.
# Requires what scripts/build-extension.sh requires.
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/store
rm -rf "$out" && mkdir -p "$out"

CARGO_INCREMENTAL=0 cargo build -q -p rex-wasm --target wasm32-unknown-unknown --profile wasm
wasm-bindgen --target web --out-dir extension/wasm target/wasm32-unknown-unknown/wasm/rex_wasm.wasm
cargo build -q --release -p rex-cli
target/release/rex bundle embed --out "$out/builtin"
(cd extension && pnpm install --silent --frozen-lockfile && pnpm -s run typecheck)
REX_CONFIG="$PWD/extension/config.example.json" REX_BUILTIN="$PWD/$out/builtin" REX_DIST="$PWD/$out/dist" \
  pnpm -s -C extension run build

node - "$out/dist" <<'JS'
const fs = require("fs");
const dist = process.argv[2];
const m = JSON.parse(fs.readFileSync(`${dist}/manifest.json`, "utf8"));
const csp = m.content_security_policy.extension_pages;
const problems = [
  m.host_permissions && "host_permissions",
  m.optional_host_permissions && "optional_host_permissions",
  !/connect-src .self.(;|$)/.test(csp) && `CSP connect-src is not self only: ${csp}`,
  fs.readFileSync(`${dist}/background.js`, "utf8").includes("rexDebug") && "e2e hooks",
  !fs.existsSync(`${dist}/recipes/recipes.json`) && "no packaged recipes",
].filter(Boolean);
if (problems.length) { console.error("store build:", problems.join("; ")); process.exit(1); }
console.log(`manifest ok: v${m.version}, permissions ${m.permissions.join(",")}`);
JS

version=$(node -p 'require("./extension/package.json").version')
zip="$out/rex-extension-$version.zip"
python3 - "$out/dist" "$zip" <<'PY'
import os, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
    for d, dirs, files in os.walk(src):
        dirs.sort()
        for f in sorted(files):
            p = os.path.join(d, f)
            z.write(p, os.path.relpath(p, src))
PY
echo "$zip: $(wc -c < "$zip") bytes, sha256 $(sha256sum "$zip" | cut -d' ' -f1)"
