"""Join pool + probe + labels into the candidate list and suggest a priority tier.

Kept: every labeled site whose pages map to a rex record (schema != none), except adult, gambling, piracy
and infrastructure. Tier (a suggestion only; the decision is made in the review page):
  P1  high value, open access, article/feed/thread/reference, homepage reachable or only bot-walled
  P2  everything else with medium/high value
  P3  low value
Outputs: out/candidates.csv (all columns) and out/sites.json (compact, for the review page).
Run: uv run scripts/select.py
"""

import csv
import json
import os
from pathlib import Path

# Survey data (raw rankings, work files, outputs) lives outside the repo.
ROOT = Path(os.environ.get("REX_SURVEY_DIR", "~/projects/scratch/rex-site-survey")).expanduser()
EXCLUDED = {"adult", "gambling", "piracy", "infra"}
CORE_SCHEMAS = {"article", "feed", "thread", "reference"}
ARTICLE_TYPES = {"NewsArticle", "Article", "BlogPosting", "ReportageNewsArticle", "TechArticle", "NewsMediaOrganization", "DiscussionForumPosting", "QAPage", "Product", "VideoObject"}


def load(name: str) -> dict[str, dict[str, object]]:
    return {r["domain"]: r for l in (ROOT / "work" / name).open() if (r := json.loads(l))}


def best_rank(p: dict[str, object]) -> int:
    ranks = [int(x) for x in (p.get("tranco"), p.get("majestic")) if x]
    # Chinaz ranks a Chinese-only list; scale it so its top 1200 interleave with the global top ~10k.
    if p.get("chinaz_rank"):
        ranks.append(int(p["chinaz_rank"]) * 8)
    elif p.get("chinaz_lists"):
        ranks.append(12000)
    return min(ranks) if ranks else 99999


def tier(lab: dict[str, object], probe: dict[str, object]) -> str:
    if lab["value"] == "low":
        return "P3"
    reachable = bool(probe.get("ok")) or bool(probe.get("challenge"))
    if lab["value"] == "high" and lab["access"] == "open" and lab["schema"] in CORE_SCHEMAS and reachable:
        return "P1"
    return "P2"


def main() -> None:
    pool, probes, labels = load("pool.jsonl"), load("probe.jsonl"), load("labels.jsonl")
    rows: list[dict[str, object]] = []
    for d, lab in labels.items():
        if lab["schema"] == "none" or lab["category"] in EXCLUDED or str(lab.get("note", "")).startswith("UNLABELED"):
            continue
        p, pr = pool[d], probes.get(d, {})
        # Some homepages misdeclare their encoding; drop names that decoded to replacement characters.
        name = next((n for n in (str(x or "") for x in (p.get("name"), pr.get("site_name"), pr.get("title"))) if n and "\ufffd" not in n), "")[:60]
        rows.append(
            {
                "domain": d,
                "name": name,
                "rank": best_rank(p),
                "tranco": p.get("tranco") or "",
                "majestic": p.get("majestic") or "",
                "chinaz": p.get("chinaz_rank") or "",
                "chinaz_lists": ",".join(p.get("chinaz_lists") or []),
                "category": lab["category"],
                "schema": lab["schema"],
                "value": lab["value"],
                "access": lab["access"],
                "lang": (str(lab.get("lang") or pr.get("lang") or "")[:2]).lower(),
                "note": str(lab.get("note") or "").replace("\ufffd", ""),
                "reachable": bool(pr.get("ok")),
                "bot_wall": bool(pr.get("challenge")),
                "ld_types": ",".join(t for t in (pr.get("ld_types") or []) if t in ARTICLE_TYPES),
                "rss": bool(pr.get("feeds")),
                "sample_url": "" if "\ufffd" in str(pr.get("sample_article") or "") else (pr.get("sample_article") or ""),
                "tier": tier(lab, pr),
                "final_url": pr.get("final_url") or "",
                "feeds": json.dumps(pr.get("feeds") or []),
            }
        )
    rows.sort(key=lambda r: (int(r["rank"]), str(r["domain"])))
    out = ROOT / "out"
    out.mkdir(exist_ok=True)
    with (out / "candidates.csv").open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    compact = [
        [r["domain"], r["name"], r["rank"], r["category"], r["schema"], r["value"], r["access"], r["lang"], r["note"],
         int(bool(r["reachable"])) + 2 * int(bool(r["bot_wall"])), r["ld_types"], int(bool(r["rss"])), r["sample_url"], r["tier"],
         r["chinaz_lists"]]
        for r in rows
    ]
    meta = {"generated": "2026-10-13", "sources": {"tranco": "64VKX top 5000", "majestic": "top 5000", "chinaz": "2400"}, "pool": len(pool), "probed": len(probes), "labeled": len(labels)}
    (out / "sites.json").write_text(json.dumps({"meta": meta, "cols": ["domain", "name", "rank", "category", "schema", "value", "access", "lang", "note", "reach", "ld", "rss", "sample", "tier", "cn"], "rows": compact}, ensure_ascii=False, separators=(",", ":")))
    by_tier = {t: sum(1 for r in rows if r["tier"] == t) for t in ("P1", "P2", "P3")}
    print(f"{len(rows)} candidates {by_tier}; labeled {len(labels)} of {len(probes)} probed")


if __name__ == "__main__":
    main()
