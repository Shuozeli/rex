"""Import accepted harvest results into the repos (decisions.md U31).

For every result with outcome thin / thin-url / draft:
  recipes/auto/<id>.yaml                      the recipe (dev channel only until promoted)
  <fixtures>/<id>/<case>/page.html.gz + meta.json   redacted sample pages (`rex fixture redact --gzip`)
  liveness-auto.yaml                          one live URL per recipe
then `rex test --update` writes expected.json for the imported cases, and `rex check` must pass.
Existing recipe files are left alone (re-harvest by deleting them first).

Run: uv run --with pyyaml tools/harvest/import_results.py --out ~/projects/scratch/rex-harvest/out \
       --fixtures ~/projects/shuozeli/_wip/rex-fixtures
"""

import argparse
import json
import re
import shutil
import subprocess
from datetime import date
from pathlib import Path
from urllib.parse import urlparse

import yaml

REPO = Path(__file__).resolve().parents[2]
REX = REPO / "target" / "release" / "rex"
ACCEPTED = {"thin", "thin-url", "draft"}


def case_name(url: str) -> str:
    """A fixture case name from a URL: the last path segment, slugged."""
    segs = [s for s in urlparse(url).path.split("/") if s]
    seg = re.sub(r"\.s?html?$", "", segs[-1] if segs else "page")
    slug = "-".join(p for p in re.split(r"[^a-z0-9]+", seg.lower()) if p)
    return (slug or "page")[:60]


def failing_recipe(line: str, fixtures: Path) -> str | None:
    """The recipe id of a FAIL/ERROR line of `rex test` ("ERROR   <case dir>: <message>", where the message
    may itself contain slashes), or None for other lines."""
    m = re.match(r"^(?:FAIL|ERROR)\s+(\S+?)(?::\s|$)", line)
    if m is None:
        return None
    try:
        return str(Path(m.group(1)).parent.relative_to(fixtures))
    except ValueError:
        return None


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True, help="harvest output directory (recipes/, results/)")
    ap.add_argument("--fixtures", type=Path, required=True, help="the rex-fixtures repo")
    args = ap.parse_args()
    out, fixtures = args.out.expanduser(), args.fixtures.expanduser()
    auto = REPO / "recipes" / "auto"
    live_path = REPO / "liveness-auto.yaml"
    live = yaml.safe_load(live_path.read_text()) if live_path.exists() else {"checks": []}
    known = {c["recipe"] for c in live["checks"]}
    imported: list[str] = []
    failed: list[str] = []
    for f in sorted((out / "results").glob("*.json")):
        r = json.loads(f.read_text())
        if r["outcome"] not in ACCEPTED:
            continue
        rid: str = r["id"]
        dest = auto / f"{rid}.yaml"
        if dest.exists() or (REPO / "recipes" / "sites" / f"{rid}.yaml").exists():
            continue
        # One recipe at a time, all or nothing: fixtures first, the recipe file last, and on any failure this
        # recipe's files are removed so a rerun imports it cleanly (an existing recipe file means "done").
        written: list[Path] = []
        try:
            urls = []
            # Two fixtures per recipe (the gate checked all sampled pages); fixtures dominate repo size.
            for i, page in enumerate(r["pages"][:2]):
                meta = json.loads((Path(page) / "meta.json").read_text())
                urls.append(meta["url"])
                case = case_name(meta["url"])
                target = fixtures / rid / case
                if target.exists():
                    target = fixtures / rid / f"{case}-{i + 1}"
                written.append(target)
                subprocess.run([str(REX), "fixture", "redact", page, str(target), "--gzip"], check=True, capture_output=True)
            if not urls:
                raise ValueError("no sample pages")
            src = (out / "recipes" / f"{rid}.yaml").read_text()
            header = f"# Auto-harvested {date.today().isoformat()} ({r['outcome']}); dev channel until promoted (decisions.md U31).\n"
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(header + src)
        except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as e:
            for t in written:
                shutil.rmtree(t, ignore_errors=True)
            dest.unlink(missing_ok=True)
            detail = e.stderr.decode(errors="replace").strip() if isinstance(e, subprocess.CalledProcessError) and e.stderr else str(e)
            print(f"skipped {rid}: {detail[:200]}")
            failed.append(rid)
            continue
        if rid not in known:
            live["checks"].append({"recipe": rid, "urls": [urls[0]]})
        imported.append(rid)
    live["checks"].sort(key=lambda c: c["recipe"])
    live_path.write_text(
        "# Live sample pages for auto-harvested recipes (tools/harvest/import_results.py), one per recipe.\n"
        + yaml.safe_dump(live, sort_keys=False, allow_unicode=True, width=200)
    )
    print(f"imported {len(imported)} recipe(s)" + (f", {len(failed)} skipped after errors" if failed else ""))
    if not imported:
        return
    # Expected outputs for the new cases. A recipe that does not match its own redacted fixtures depends on
    # publisher text (redaction replaces it, decisions.md U19): drop it with its fixtures and liveness entry.
    # Filters are substrings of the case path; "<id>/" keeps example.com/a from also selecting example.com/ab.
    update = subprocess.run(
        [str(REX), "test", "--recipes", str(REPO / "recipes"), "--fixtures", str(fixtures), "--update", *(f"{rid}/" for rid in imported)],
        capture_output=True,
        text=True,
    )
    if not re.search(r"^\d+ passed, \d+ failed, \d+ updated$", update.stdout, re.M):
        raise SystemExit(f"rex test --update did not run: {update.stderr.strip()[:500]}")
    broken = sorted({rid for line in update.stdout.splitlines() if (rid := failing_recipe(line, fixtures)) is not None})
    for rid in broken:
        (auto / f"{rid}.yaml").unlink(missing_ok=True)
        shutil.rmtree(fixtures / rid, ignore_errors=True)
        live["checks"] = [c for c in live["checks"] if c["recipe"] != rid]
        print(f"dropped {rid}: it does not match its own redacted fixtures")
    if broken:
        live_path.write_text(
            "# Live sample pages for auto-harvested recipes (tools/harvest/import_results.py), one per recipe.\n"
            + yaml.safe_dump(live, sort_keys=False, allow_unicode=True, width=200)
        )
    check = subprocess.run(
        [str(REX), "check", str(REPO / "recipes"), "--fixtures", str(REPO / "fixtures"), "--fixtures", str(fixtures)],
        capture_output=True,
        text=True,
    )
    bad = [l for l in check.stdout.splitlines() if l.startswith("ERR")]
    print(f"rex check: {len(bad)} error(s)")
    for l in bad[:20]:
        print(" ", l)


if __name__ == "__main__":
    main()
