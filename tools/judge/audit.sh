#!/usr/bin/env bash
# Periodic recipe audit (decisions.md U44, layer 3). Runs on a host that reaches the LLM endpoint (the internal
# proxy is not reachable from GitHub runners): fetches every liveness page, then has `rex judge` review those
# live records and every captured fixture. Verdicts are cached, so a rerun only asks about pages or records
# that changed.
#
#   REX_LLM_ENDPOINT=... REX_LLM_INLINE_SYSTEM=1 tools/judge/audit.sh [out dir]
#
# Default out dir: ~/.cache/rex/audit/<UTC date>. judge.txt is the readable report, judge.json the full one.
# Exit 1 when any record has errors.
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${REX_LLM_ENDPOINT:?set REX_LLM_ENDPOINT}"
out=${1:-$HOME/.cache/rex/audit/$(date -u +%F)}
fx=${REX_FIXTURES:-../rex-fixtures}
mkdir -p "$out"
cargo build -q --release -p rex-cli
rex=./target/release/rex

rm -rf "$out/live"
for f in liveness liveness-auto; do
  $rex liveness --file "$f.yaml" --save "$out/live/$f" --report "$out/$f.json" >"$out/$f.txt" || true
  echo "$f: $(tail -1 "$out/$f.txt")"
done

roots=(--fixtures "$out/live" --fixtures fixtures)
[ -d "$fx" ] && roots+=(--fixtures "$fx")
status=0
$rex judge "${roots[@]}" --captured-only --cache "$HOME/.cache/rex/judge" --report "$out/judge.json" \
  >"$out/judge.txt" || status=$?
echo "judge: $(tail -1 "$out/judge.txt")"
echo "report: $out/judge.txt"
exit $status
