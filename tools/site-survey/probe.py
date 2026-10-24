"""Fetch each candidate's homepage once and record what a recipe author needs to know.

Per domain: reachability, final URL, title, description, language, og:site_name/og:type, JSON-LD and
microdata types, RSS/Atom feeds, <article> count, a sample article-like link, and whether the page is a
bot challenge or a JavaScript shell. Resumable: domains already in work/probe.jsonl are skipped.
Run: uv run --with httpx --with selectolax scripts/probe.py
"""

import asyncio
import json
import os
import re
from pathlib import Path
from urllib.parse import urljoin, urlparse

import httpx
from selectolax.lexbor import LexborHTMLParser

# Survey data (raw rankings, work files, outputs) lives outside the repo.
ROOT = Path(os.environ.get("REX_SURVEY_DIR", "~/projects/scratch/rex-site-survey")).expanduser()
OUT = ROOT / "work" / "probe.jsonl"
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0 Safari/537.36"
CONCURRENCY = 32
MAX_BYTES = 1_500_000
CHALLENGE = re.compile(r"just a moment|attention required|cf-chl|captcha|are you a robot|access denied|验证码|安全验证|滑动验证", re.I)
ARTICLE_PATH = re.compile(
    r"/(19|20)\d{2}[/-]\d{1,2}([/-]\d{1,2})?/|/(article|articles|news|story|stories|post|posts|p|blog|a|detail|content|doc|item|archives?)/[^/?#]{4,}|\d{6,}\.s?html?$|/[a-z0-9]+(-[a-z0-9]+){3,}/?$",
    re.I,
)


def ld_types(tree: LexborHTMLParser) -> list[str]:
    types: set[str] = set()

    def walk(v: object) -> None:
        if isinstance(v, dict):
            t = v.get("@type")
            for x in t if isinstance(t, list) else [t]:
                if isinstance(x, str):
                    types.add(x.rsplit("/", 1)[-1])
            for k in ("@graph", "mainEntity", "itemListElement"):
                if k in v:
                    walk(v[k])
        elif isinstance(v, list):
            for x in v:
                walk(x)

    for s in tree.css('script[type="application/ld+json"]'):
        try:
            walk(json.loads(s.text() or "null"))
        except (json.JSONDecodeError, RecursionError):
            continue
    for n in tree.css("[itemtype]"):
        t = n.attributes.get("itemtype") or ""
        types.add(t.rstrip("/").rsplit("/", 1)[-1])
    return sorted(t for t in types if t)


def meta(tree: LexborHTMLParser, *keys: str) -> str:
    for k in keys:
        n = tree.css_first(f'meta[property="{k}"]') or tree.css_first(f'meta[name="{k}"]')
        if n is not None and (c := (n.attributes.get("content") or "").strip()):
            return c[:300]
    return ""


def analyze(domain: str, final_url: str, status: int, html: str) -> dict[str, object]:
    tree = LexborHTMLParser(html)
    title_node = tree.css_first("title")
    title = (title_node.text(strip=True) if title_node else "")[:200]
    html_node = tree.css_first("html")
    lang = (html_node.attributes.get("lang") or "") if html_node else ""
    body = tree.body
    text_len = len(body.text(separator=" ", strip=True)) if body else 0
    feeds = [
        urljoin(final_url, n.attributes.get("href") or "")
        for n in tree.css('link[rel="alternate"]')
        if "rss" in (n.attributes.get("type") or "") or "atom" in (n.attributes.get("type") or "")
    ][:3]
    host = urlparse(final_url).hostname or ""
    sample = ""
    article_links = 0
    for a in tree.css("a[href]"):
        href = urljoin(final_url, a.attributes.get("href") or "")
        u = urlparse(href)
        if u.scheme not in ("http", "https") or not (u.hostname or "").endswith(domain):
            continue
        if ARTICLE_PATH.search(u.path):
            article_links += 1
            if not sample:
                sample = href.split("#")[0]
    return {
        "domain": domain,
        "ok": 200 <= status < 300,
        "status": status,
        "final_url": final_url,
        "final_host": host,
        "offsite": not host.endswith(domain),
        "title": title,
        "description": meta(tree, "og:description", "description"),
        "site_name": meta(tree, "og:site_name", "application-name"),
        "og_type": meta(tree, "og:type"),
        "lang": lang[:10],
        "ld_types": ld_types(tree),
        "feeds": feeds,
        "articles": len(tree.css("article")),
        "article_links": article_links,
        "sample_article": sample,
        "text_len": text_len,
        "scripts": len(tree.css("script")),
        "challenge": bool(CHALLENGE.search(title)) or (status in (403, 429, 503) and text_len < 2000),
        "bytes": len(html),
    }


async def fetch(client: httpx.AsyncClient, domain: str) -> dict[str, object]:
    last_error = ""
    for url in (f"https://{domain}/", f"https://www.{domain}/", f"http://www.{domain}/"):
        try:
            async with client.stream("GET", url) as resp:
                chunks: list[bytes] = []
                size = 0
                async for chunk in resp.aiter_bytes():
                    chunks.append(chunk)
                    size += len(chunk)
                    if size > MAX_BYTES:
                        break
                ctype = resp.headers.get("content-type", "")
                raw = b"".join(chunks)
                if "html" not in ctype and not raw.lstrip()[:15].lower().startswith((b"<!doctype", b"<html")):
                    return {"domain": domain, "ok": False, "status": resp.status_code, "error": f"not html: {ctype[:60]}"}
                html = raw.decode(resp.encoding or "utf-8", errors="replace")
                return analyze(domain, str(resp.url), resp.status_code, html)
        except (httpx.HTTPError, UnicodeError, ValueError) as e:
            last_error = f"{type(e).__name__}: {str(e)[:120]}"
    return {"domain": domain, "ok": False, "status": 0, "error": last_error}


async def main() -> None:
    done: set[str] = set()
    if OUT.exists():
        done = {json.loads(l)["domain"] for l in OUT.open()}
    todo = [e["domain"] for l in (ROOT / "work" / "pool.jsonl").open() if not (e := json.loads(l))["infra"] and e["domain"] not in done]
    print(f"{len(done)} done, {len(todo)} to probe")
    sem = asyncio.Semaphore(CONCURRENCY)
    limits = httpx.Limits(max_connections=CONCURRENCY * 2, max_keepalive_connections=0)
    async with httpx.AsyncClient(
        headers={"User-Agent": UA, "Accept-Language": "en,zh-CN;q=0.8"},
        timeout=httpx.Timeout(12.0),
        follow_redirects=True,
        limits=limits,
    ) as client:
        with OUT.open("a") as f:
            count = 0

            async def one(d: str) -> None:
                nonlocal count
                async with sem:
                    try:
                        r = await asyncio.wait_for(fetch(client, d), timeout=45)
                    except TimeoutError:
                        r = {"domain": d, "ok": False, "status": 0, "error": "overall timeout"}
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
                f.flush()
                count += 1
                if count % 250 == 0:
                    print(f"{count}/{len(todo)}", flush=True)

            await asyncio.gather(*(one(d) for d in todo))


if __name__ == "__main__":
    asyncio.run(main())
