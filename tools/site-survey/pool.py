"""Merge the rankings into one candidate pool keyed by registrable domain.

Inputs (raw/): top-1m.csv (Tranco), majestic.csv (Majestic Million), chinaz.jsonl (Chinaz).
Output: work/pool.jsonl with {domain, tranco, majestic, chinaz_rank, chinaz_lists, name, intro, infra}.
`infra` marks domains that are obviously not browsable sites (CDNs, APIs, ads, tracking, DNS, mail);
they are kept in the pool but skipped by the probe.
Run: uv run --with tldextract scripts/pool.py
"""

import csv
import json
import os
import re
from pathlib import Path

import tldextract

# Survey data (raw rankings, work files, outputs) lives outside the repo.
ROOT = Path(os.environ.get("REX_SURVEY_DIR", "~/projects/scratch/rex-site-survey")).expanduser()
TOP_TRANCO = 5000
TOP_MAJESTIC = 5000
# Offline: use the bundled public-suffix snapshot.
EXTRACT = tldextract.TLDExtract(suffix_list_urls=())

INFRA = re.compile(
    r"(cdn|akamai|edgekey|edgesuite|cloudfront|fastly|gstatic|googleapis|googleusercontent|googlevideo|"
    r"googlesyndication|doubleclick|googletagmanager|google-analytics|googleadservices|gvt[0-9]|ytimg|"
    r"fbcdn|fbsbx|cdninstagram|twimg|tiktokcdn|byteimg|bytedance|alicdn|aliyuncs|qpic|gtimg|qlogo|"
    r"amazonaws|azureedge|azurewebsites|windows\.net|msedge|trafficmanager|office\.net|msftconnecttest|"
    r"cloudflare-dns|1e100|dns|ntp|ocsp|crl|pki|digicert|letsencrypt|sectigo|globalsign|"
    r"analytics|metrics|telemetry|tracking|tracker|adsrvr|adnxs|criteo|taboola|outbrain|pubmatic|rubicon|"
    r"openx|casalemedia|scorecardresearch|quantserve|demdex|omtrdc|hotjar|segment\.io|branch\.io|"
    r"appsflyer|adjust\.com|onesignal|sentry\.io|newrelic|nr-data|datadoghq|"
    r"\bapi\b|static|assets|img|images|media-amazon|ssl-images|push|update|download\.windows|"
    r"mcafee|avast|kaspersky|symantec|norton|"
    r"in-addr|local$|lan$|arpa$)",
    re.I,
)


def registrable(host: str) -> str | None:
    e = EXTRACT(host.strip().lower().rstrip("."))
    if not e.domain or not e.suffix:
        return None
    return f"{e.domain}.{e.suffix}"


def main() -> None:
    pool: dict[str, dict[str, object]] = {}

    def entry(d: str) -> dict[str, object]:
        return pool.setdefault(
            d, {"domain": d, "tranco": None, "majestic": None, "chinaz_rank": None, "chinaz_lists": [], "name": "", "intro": ""}
        )

    with (ROOT / "raw" / "top-1m.csv").open() as f:
        for rank, host in csv.reader(f):
            if int(rank) > TOP_TRANCO:
                break
            d = registrable(host)
            if d and entry(d)["tranco"] is None:
                entry(d)["tranco"] = int(rank)
    with (ROOT / "raw" / "majestic.csv").open() as f:
        for row in csv.DictReader(f):
            if int(row["GlobalRank"]) > TOP_MAJESTIC:
                break
            d = registrable(row["Domain"])
            if d and entry(d)["majestic"] is None:
                entry(d)["majestic"] = int(row["GlobalRank"])
    with (ROOT / "raw" / "chinaz.jsonl").open() as f:
        for line in f:
            r = json.loads(line)
            d = registrable(str(r["domain"]))
            if not d:
                continue
            e = entry(d)
            lists = e["chinaz_lists"]
            assert isinstance(lists, list)
            if r["list"] not in lists:
                lists.append(r["list"])
            if r["list"] == "all" and e["chinaz_rank"] is None:
                e["chinaz_rank"] = r["rank"]
            if not e["name"]:
                e["name"], e["intro"] = r["name"], r["intro"]
    out = ROOT / "work"
    out.mkdir(exist_ok=True)
    infra = 0
    with (out / "pool.jsonl").open("w") as f:
        for d, e in sorted(pool.items()):
            e["infra"] = bool(INFRA.search(d)) and not e["chinaz_lists"]
            infra += bool(e["infra"])
            f.write(json.dumps(e, ensure_ascii=False) + "\n")
    print(f"pool: {len(pool)} registrable domains, {infra} marked infra, {len(pool) - infra} to probe")


if __name__ == "__main__":
    main()
