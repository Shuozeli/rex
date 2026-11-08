#!/usr/bin/env bash
# Run the CI gates (.github/workflows/ci.yml) on a workstation, in the same order.
#   scripts/ci-local.sh            all gates
#   GIT_BASE=<ref> scripts/ci-local.sh   version gate against <ref> (default HEAD)
# Requires: rustup target wasm32-unknown-unknown, wasm-bindgen CLI 0.2.123, protoc, node 22, pnpm, and a
# Chromium for the e2e (REX_CHROME, default: the Playwright cache).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
# Warnings fail clippy (`-D warnings`, which covers every rustc lint). A global RUSTFLAGS would split the
# build cache from normal builds, so it is not set here; CARGO_BUILD_JOBS limits memory on busy hosts.
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
budget=1150000
step() { printf '\n== %s\n' "$*"; }

step "fmt";                cargo fmt --all -- --check
step "clippy";             cargo clippy -q --workspace --all-targets --release -- -D warnings
step "tests"
if ! cargo test -q --workspace --release > "$root/target/ci-local-tests.log" 2>&1; then
  grep -E "FAILED|panicked|error" "$root/target/ci-local-tests.log" | head -40; exit 1
fi
grep -E "^test result" "$root/target/ci-local-tests.log" | awk '{p+=$4; f+=$6} END {print p " passed, " f " failed"}' 
step "rex";                cargo build -q --release -p rex-cli
rex=target/release/rex
# Harvested-recipe fixtures (decisions.md U31): a sibling checkout of Shuozeli/rex-fixtures.
fx="${REX_FIXTURES:-$root/../rex-fixtures}"
test -d "$fx" || { echo "missing $fx (clone Shuozeli/rex-fixtures next to rex, or set REX_FIXTURES)"; exit 1; }
step "rex check";          $rex check recipes --fixtures fixtures --fixtures "$fx" --liveness liveness.yaml --liveness liveness-auto.yaml --git-base "${GIT_BASE:-HEAD}" | grep -v '^ok' || true
                           $rex check recipes --fixtures fixtures --fixtures "$fx" --liveness liveness.yaml --liveness liveness-auto.yaml --git-base "${GIT_BASE:-HEAD}" >/dev/null
step "rex test";           $rex test --fixtures fixtures --fixtures conformance --fixtures "$fx" | tail -1
step "fixture lint";       $rex fixture lint fixtures "$fx" | tail -1

step "wasm build";         cargo build -q -p rex-wasm --target wasm32-unknown-unknown --profile wasm
wasm=target/wasm32-unknown-unknown/wasm/rex_wasm.wasm
wasm-bindgen --target web --out-dir extension/wasm "$wasm"
size=$(gzip -9c extension/wasm/rex_wasm_bg.wasm | wc -c)
echo "shipped rex_wasm_bg.wasm: $size bytes gzipped (budget $budget)"
test "$size" -le "$budget"
step "wasm conformance"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
wasm-bindgen --target nodejs --out-dir target/wasm-node "$wasm"
$rex bundle keygen --key-id ci-local --out "$tmp/key.json" >/dev/null
rm -rf target/conformance-bundle
$rex bundle build --key "$tmp/key.json" --out target/conformance-bundle >/dev/null
node crates/rex-wasm/tests/conformance.mjs target/wasm-node target/conformance-bundle | tail -1

step "embed recipes";       rm -rf extension/builtin; $rex bundle embed --out extension/builtin

cd extension
step "extension typecheck"; pnpm install --silent --frozen-lockfile; pnpm -s run typecheck
step "extension unit tests"; node --test tests/format.test.mjs 2>&1 | grep -E "^# (pass|fail)"
step "store build";         pnpm -s run build
node -e '
  const fs = require("fs");
  const m = JSON.parse(fs.readFileSync("dist/manifest.json", "utf8"));
  const csp = m.content_security_policy.extension_pages;
  const problems = [
    m.host_permissions && "host_permissions",
    m.optional_host_permissions && "optional_host_permissions",
    !/connect-src .self.(;|$)/.test(csp) && `CSP connect-src is not self only: ${csp}`,
    fs.readFileSync("dist/background.js", "utf8").includes("rexDebug") && "e2e hooks",
    !fs.existsSync("dist/recipes/recipes.json") && "no packaged recipes",
  ].filter(Boolean);
  if (problems.length) { console.error("store build:", problems.join("; ")); process.exit(1); }
  console.log("manifest ok:", m.permissions.join(","), "| recipes:", JSON.parse(fs.readFileSync("dist/recipes/index.json", "utf8")).entries.length);'
if command -v zip >/dev/null; then
  (cd dist && zip -qr "$tmp/rex-extension.zip" .)
else
  uv run --quiet python -c 'import os, sys, zipfile
with zipfile.ZipFile(sys.argv[2], "w", zipfile.ZIP_DEFLATED) as z:
    for d, _, fs in sorted(os.walk(sys.argv[1])):
        for f in sorted(fs):
            p = os.path.join(d, f); z.write(p, os.path.relpath(p, sys.argv[1]))' dist "$tmp/rex-extension.zip"
fi
echo "packed: $(wc -c < "$tmp/rex-extension.zip") bytes"
step "e2e build";           pnpm -s run build:e2e
step "e2e";                 node tests/e2e.mjs | tail -14
if [ "${REX_DOCKER_E2E:-0}" = "1" ]; then
  step "docker e2e (Google Chrome)"; (cd .. && scripts/docker-e2e.sh --no-video) | tail -9
fi
echo; echo "CI-LOCAL PASSED"
