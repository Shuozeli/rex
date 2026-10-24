"""Label every probed domain with an LLM (Gemini generateContent, endpoint from $REX_LLM_ENDPOINT).

Each batch sends compact signals (domain, Chinaz name/intro, title, description, language, schema.org
types, og:type, article-link count) and asks for a strict JSON array. Labels are validated against the
enums below; invalid batches are retried, then split. Resumable via work/labels.jsonl.
Run: REX_LLM_ENDPOINT=... uv run --with httpx scripts/classify.py
"""

import asyncio
import json
import os
import re
from pathlib import Path

import httpx

# Survey data (raw rankings, work files, outputs) lives outside the repo.
ROOT = Path(os.environ.get("REX_SURVEY_DIR", "~/projects/scratch/rex-site-survey")).expanduser()
OUT = ROOT / "work" / "labels.jsonl"
BATCH = 60
CONCURRENCY = 6

CATEGORIES = [
    "news", "finance_news", "tech_news", "magazine", "blog_platform", "blog", "forum", "social", "qa",
    "video", "audio", "reference", "dev_docs", "academic", "government", "education", "ecommerce",
    "classifieds", "travel", "jobs", "real_estate", "auto", "health", "sports", "entertainment", "gaming",
    "finance_service", "search_portal", "saas_tool", "corporate", "adult", "gambling", "piracy", "infra", "other",
]
SCHEMAS = ["article", "feed", "thread", "product", "video", "reference", "none"]
VALUES = ["high", "medium", "low"]
ACCESS = ["open", "login", "paywall", "app_only", "unknown"]

SYSTEM = f"""You classify websites for rex, a tool that turns a web page into one typed JSON record.
For each input site return one object with exactly these keys:
- "domain": copied from the input
- "category": one of {CATEGORIES}
- "schema": the rex record a typical content page of this site maps to, one of {SCHEMAS}
  (article = news/blog/magazine story; feed = a list page such as a front page, ranking or timeline;
  thread = forum/Q&A post with replies; product = shop item; video = video page; reference = wiki/docs
  entry; none = no page worth turning into a record: tools, logins, infrastructure, corporate homepages)
- "value": how useful saving its pages as structured records would be for readers or researchers, one of {VALUES}
- "access": one of {ACCESS} (login = most content needs an account; paywall = metered or hard paywall)
- "lang": main content language as an ISO 639-1 code
- "note": at most 12 words, e.g. what the site is
Use the signals given and your own knowledge of well-known sites. Return only a JSON array, one object per
input site, in the same order."""


def compact(p: dict[str, object], pool: dict[str, object]) -> dict[str, object]:
    return {
        "domain": p["domain"],
        "cn_name": pool.get("name") or None,
        "cn_intro": str(pool.get("intro") or "")[:80] or None,
        "title": str(p.get("title") or "")[:90] or None,
        "desc": str(p.get("description") or "")[:120] or None,
        "lang": p.get("lang") or None,
        "ld": (p.get("ld_types") or [])[:6] or None,
        "og": p.get("og_type") or None,
        "article_links": p.get("article_links") or 0,
        "reachable": bool(p.get("ok")),
        "bot_wall": bool(p.get("challenge")),
    }


def validate(items: list[dict[str, object]], got: object) -> list[dict[str, object]]:
    if not isinstance(got, list) or len(got) != len(items):
        raise ValueError(f"expected {len(items)} objects, got {type(got).__name__} of {len(got) if isinstance(got, list) else '?'}")
    out = []
    for want, g in zip(items, got):
        if not isinstance(g, dict) or g.get("domain") != want["domain"]:
            raise ValueError(f"order/domain mismatch at {want['domain']}")
        for key, allowed in (("category", CATEGORIES), ("schema", SCHEMAS), ("value", VALUES), ("access", ACCESS)):
            if g.get(key) not in allowed:
                raise ValueError(f"{want['domain']}: bad {key} {g.get(key)!r}")
        out.append({k: g.get(k) for k in ("domain", "category", "schema", "value", "access", "lang", "note")})
    return out


async def ask(client: httpx.AsyncClient, endpoint: str, items: list[dict[str, object]]) -> list[dict[str, object]]:
    prompt = SYSTEM + "\n\n---\n\nSites:\n" + json.dumps(items, ensure_ascii=False)
    body = {
        "contents": [{"role": "user", "parts": [{"text": prompt}]}],
        "generationConfig": {"temperature": 0, "responseMimeType": "application/json"},
    }
    resp = await client.post(endpoint, json=body)
    resp.raise_for_status()
    text = "".join(p.get("text", "") for p in resp.json()["candidates"][0]["content"]["parts"])
    m = re.search(r"\[.*\]", text, re.S)
    if m is None:
        raise ValueError(f"no JSON array: {text[:120]!r}")
    return validate(items, json.loads(m.group(0)))


async def label(client: httpx.AsyncClient, endpoint: str, items: list[dict[str, object]]) -> list[dict[str, object]]:
    error = ""
    for _ in range(2):
        try:
            return await ask(client, endpoint, items)
        except (httpx.HTTPError, ValueError, KeyError, IndexError, json.JSONDecodeError) as e:
            error = str(e)[:160]
    if len(items) == 1:
        return [{"domain": items[0]["domain"], "category": "other", "schema": "none", "value": "low", "access": "unknown", "lang": "", "note": f"UNLABELED: {error}"}]
    mid = len(items) // 2
    return await label(client, endpoint, items[:mid]) + await label(client, endpoint, items[mid:])


async def main() -> None:
    endpoint = os.environ["REX_LLM_ENDPOINT"]
    pool = {e["domain"]: e for l in (ROOT / "work" / "pool.jsonl").open() if (e := json.loads(l))}
    # Rows that failed to label (UNLABELED) are dropped and asked again; the file is rewritten without them
    # (one row per domain, the last one winning) so a rerun does not leave both versions behind.
    lines = OUT.read_text().splitlines() if OUT.exists() else []
    rows = {r["domain"]: r for l in lines if (r := json.loads(l))}
    labeled = {d: r for d, r in rows.items() if not str(r.get("note", "")).startswith("UNLABELED")}
    if len(labeled) != len(lines):
        tmp = OUT.with_suffix(".tmp")
        tmp.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in labeled.values()))
        tmp.replace(OUT)
    done = set(labeled)
    probes = [json.loads(l) for l in (ROOT / "work" / "probe.jsonl").open()]
    def rank(d: str) -> int:
        e = pool[d]
        ranks = [int(x) for x in (e.get("tranco"), e.get("majestic")) if x]
        if e.get("chinaz_rank"):
            ranks.append(int(e["chinaz_rank"]) * 8)
        return min(ranks) if ranks else 12000

    # Highest-ranked first, so a partial run already covers the most important sites.
    probes.sort(key=lambda p: rank(p["domain"]))
    todo = [compact(p, pool[p["domain"]]) for p in probes if p["domain"] not in done]
    batches = [todo[i : i + BATCH] for i in range(0, len(todo), BATCH)]
    print(f"{len(done)} labeled, {len(todo)} to label in {len(batches)} batches", flush=True)
    sem = asyncio.Semaphore(CONCURRENCY)
    async with httpx.AsyncClient(timeout=300) as client:
        with OUT.open("a") as f:
            finished = 0

            async def run(batch: list[dict[str, object]]) -> None:
                nonlocal finished
                async with sem:
                    rows = await label(client, endpoint, batch)
                for r in rows:
                    f.write(json.dumps(r, ensure_ascii=False) + "\n")
                f.flush()
                finished += 1
                if finished % 10 == 0:
                    print(f"{finished}/{len(batches)} batches", flush=True)

            await asyncio.gather(*(run(b) for b in batches))


if __name__ == "__main__":
    asyncio.run(main())
