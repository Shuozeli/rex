#!/bin/bash
# Hermetic e2e of the store build in Google Chrome stable, in Docker (decisions.md U40, operations.md):
#   1. a throwaway Ed25519 bundle key and a config whose only source is https://bundles.rex.test/v1/dev/;
#   2. the store build with that config (extension/recorder/docker/out/hermetic/ext; dist/ is untouched);
#   3. `rex bundle publish` of recipes/ signed with that key;
#   4. the e2e container (no network): it installs the CRX by policy, serves the fixture pages and the bundle,
#      and runs a scenario with real input; each scene must match its fixture's expected.json.
# Exit code: the runner's (1 if a scene fails). Extra arguments go to run.mjs (e.g. --no-video, --only hn).
# Env: REX_E2E_SCENARIO (default fixtures.json in extension/recorder/scenarios/); REX_E2E_SKIP_BUILD=1 uses the
#      rex-recorder-desktop:local image as it is (CI builds it with a layer cache first).
# Requires: docker, pnpm, target/release/rex. The CRX signing key is $REX_EXT_KEY, else
# ~/.config/rex/ext-e2e-key.pem, else a throwaway RSA key made here (any key works; it only fixes the id).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=extension/recorder/docker/out/hermetic
COMPOSE=(docker compose -f extension/recorder/docker/compose.yaml --profile e2e)
SCENARIO=${REX_E2E_SCENARIO:-fixtures.json}
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

node --input-type=module - "$OUT" <<'JS'
import { createPrivateKey, createPublicKey } from "node:crypto";
import { writeFileSync } from "node:fs";
const out = process.argv[2];
const seed = Buffer.alloc(32, 7);
const der = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]);
const pub = createPublicKey(createPrivateKey({ key: der, format: "der", type: "pkcs8" })).export({ format: "der", type: "spki" }).subarray(-32).toString("hex");
const keyId = "e2e-docker";
writeFileSync(`${out}/key.json`, JSON.stringify({ key_id: keyId, seed: seed.toString("hex"), public: pub }));
writeFileSync(`${out}/config.json`, JSON.stringify({
  sources: [{ id: "e2e", baseUrl: "https://bundles.rex.test/v1/dev/", channel: "dev", keys: [{ keyId, public: pub }] }],
  refreshMinutes: 360,
  reportRepo: "Shuozeli/rex",
}, null, 2));
JS

REX_CONFIG="$PWD/$OUT/config.json" REX_DIST="$PWD/$OUT/ext" pnpm -s -C extension run build
target/release/rex bundle publish --recipes recipes --key "$OUT/key.json" --channel dev --to "$OUT/bundle" | tail -1
if [ "${REX_E2E_SKIP_BUILD:-0}" != "1" ]; then
  "${COMPOSE[@]}" build -q e2e
fi
"${COMPOSE[@]}" run --rm e2e node /repo/extension/recorder/run.mjs "/repo/extension/recorder/scenarios/$SCENARIO" \
  --out /out/hermetic/run "$@"
