"""Promote auto-harvested recipes from the dev channel to stable (decisions.md U31).

A recipe in recipes/auto/ is promoted when it passed liveness in at least --days daily reports (the
`liveness-auto-report.json` artifacts of the Liveness workflow, downloaded into --reports, e.g. with
`gh run download --name liveness-report`) and failed in none of them. Promotion moves the YAML to
recipes/sites/ (stable) and its liveness entry from liveness-auto.yaml to liveness.yaml. Fixtures stay in
rex-fixtures (`rex check` counts every root).

Run: uv run --with pyyaml tools/harvest/promote.py --reports ~/rex-liveness-reports [--days 7] [--dry-run]
"""

import argparse
import json
import subprocess
from collections import defaultdict
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
HEADER_AUTO = "# Live sample pages for auto-harvested recipes (tools/harvest/import_results.py), one per recipe.\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--reports", type=Path, required=True, help="directory tree holding liveness-auto-report.json files")
    ap.add_argument("--days", type=int, default=7)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    passes: dict[str, int] = defaultdict(int)
    failures: dict[str, int] = defaultdict(int)
    reports = sorted(args.reports.expanduser().rglob("liveness-auto-report.json"))
    for report in reports:
        for r in json.loads(report.read_text()):
            (passes if r["ok"] else failures)[r["recipe"]] += 1
    print(f"{len(reports)} daily report(s)")

    auto_live_path, live_path = REPO / "liveness-auto.yaml", REPO / "liveness.yaml"
    auto_live = yaml.safe_load(auto_live_path.read_text())
    live = yaml.safe_load(live_path.read_text())
    promoted: list[str] = []
    for f in sorted((REPO / "recipes" / "auto").rglob("*.yaml")):
        rid = str(f.relative_to(REPO / "recipes" / "auto").with_suffix(""))
        if passes[rid] < args.days or failures[rid] > 0:
            continue
        promoted.append(rid)
        if args.dry_run:
            continue
        dest = REPO / "recipes" / "sites" / f"{rid}.yaml"
        dest.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "mv", str(f), str(dest)], cwd=REPO, check=True)
        entry = next((c for c in auto_live["checks"] if c["recipe"] == rid), None)
        if entry is not None:
            auto_live["checks"].remove(entry)
            live["checks"].append(entry)
    print(f"{'would promote' if args.dry_run else 'promoted'} {len(promoted)} recipe(s)")
    for rid in promoted[:50]:
        print(" ", rid)
    if promoted and not args.dry_run:
        auto_live_path.write_text(HEADER_AUTO + yaml.safe_dump(auto_live, sort_keys=False, allow_unicode=True, width=200))
        live_path.write_text(yaml.safe_dump(live, sort_keys=False, allow_unicode=True, width=200))
        print("next: rex check recipes --fixtures fixtures --fixtures ../rex-fixtures, then commit")


if __name__ == "__main__":
    main()
