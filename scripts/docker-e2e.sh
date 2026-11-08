#!/bin/bash
# Hermetic e2e of the store build in Google Chrome stable, in Docker (decisions.md U40, U45; operations.md):
#   1. the recipes embedded with `rex bundle embed` and the store build with them
#      (extension/recorder/docker/out/hermetic/ext; dist/ is untouched);
#   2. the e2e container (no network): it installs the CRX by policy, serves the fixture pages, and runs a
#      scenario with real input; each scene must match its fixture's expected.json, and the extension must not
#      make a single network request.
# Exit code: the runner's (1 if a scene fails). Extra arguments go to run.mjs (e.g. --no-video, --only hn).
# Env: REX_E2E_SCENARIOS (space-separated, in extension/recorder/scenarios/; default
#      "fixtures.json sites.json generic.json": the 7 tutorial scenes with a video, then one quick scene per site
#      recipe and one per generic recipe, checks only);
#      REX_E2E_SKIP_BUILD=1 uses the rex-recorder-desktop:local image as it is (CI builds it with a layer cache).
# Output: extension/recorder/docker/out/hermetic/run/<scenario name>/.
# Requires: docker, pnpm, target/release/rex. The CRX signing key is $REX_EXT_KEY, else
# ~/.config/rex/ext-e2e-key.pem, else a throwaway RSA key made here (any key works; it only fixes the id).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=extension/recorder/docker/out/hermetic
COMPOSE=(docker compose -f extension/recorder/docker/compose.yaml --profile e2e)
SCENARIOS=()
for s in ${REX_E2E_SCENARIOS:-fixtures.json sites.json generic.json}; do SCENARIOS+=("/repo/extension/recorder/scenarios/$s"); done
export REX_E2E_HERMETIC="${SCENARIOS[*]}"
rm -rf "$OUT" && mkdir -p "$OUT/run"
# The container runs as uid 1000; a CI runner may be another uid. It writes only to run/.
chmod a+rwx "$OUT/run"
if [ -z "${REX_EXT_KEY:-}" ]; then
  if [ -r ~/.config/rex/ext-e2e-key.pem ]; then
    export REX_EXT_KEY=~/.config/rex/ext-e2e-key.pem
  else
    # Throwaway, so readable by the container's user whatever this uid is.
    openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$OUT/crx-key.pem" 2>/dev/null
    chmod 644 "$OUT/crx-key.pem"
    export REX_EXT_KEY="$PWD/$OUT/crx-key.pem"
  fi
fi

target/release/rex bundle embed --out "$OUT/builtin"
REX_BUILTIN="$PWD/$OUT/builtin" REX_DIST="$PWD/$OUT/ext" pnpm -s -C extension run build
if [ "${REX_E2E_SKIP_BUILD:-0}" != "1" ]; then
  "${COMPOSE[@]}" build -q e2e
fi
"${COMPOSE[@]}" run --rm e2e node /repo/extension/recorder/run.mjs "${SCENARIOS[@]}" --out /out/hermetic/run "$@"
