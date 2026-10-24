"""Batch recipe harvesting (decisions.md U31): turn surveyed sites into gated recipes.

For each site (article schema for now):
  1. capture the homepage (the negative page: an article recipe must not match it);
  2. find article URLs (RSS items first, then homepage links that look like articles) and capture 2-3;
  3. thin recipe: when every article page states a schema.org article type (or og:type=article) that the
     homepage does not, `extends: generic/article.v1` plus host + page condition, checked with `rex gate`;
  4. otherwise `rex draft` with the other pages as `--also`, the homepage as `--negative`, and 2 repair rounds.
Captures stay in the private work directory; accepted recipes are written to OUT/recipes/<id>.yaml with the
pages that passed the gate listed in OUT/results/<domain>.json (fixtures are redacted later, at import).

Resumable: a site with a result file is skipped. The LLM endpoint comes from $REX_LLM_ENDPOINT.
Run: uv run --with httpx --with selectolax --with defusedxml --with pyyaml tools/harvest/harvest.py \
       --sites tools/harvest/sites.csv --work ~/projects/scratch/rex-harvest --limit 50 --tier P1
"""

import argparse
import asyncio
import csv
import json
import os
import re
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from urllib.parse import urljoin, urlparse

import httpx
import yaml
from collections import Counter
from defusedxml import ElementTree
from selectolax.lexbor import LexborHTMLParser

REPO = Path(__file__).resolve().parents[2]
REX = REPO / "target" / "release" / "rex"
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0 Safari/537.36"
ARTICLE_TYPES = {"NewsArticle", "Article", "BlogPosting", "ReportageNewsArticle", "TechArticle", "AnalysisNewsArticle", "OpinionNewsArticle"}
ARTICLE_PATH = re.compile(
    r"/(19|20)\d{2}[/-]\d{1,2}([/-]\d{1,2})?/|/(article|articles|news|story|stories|post|posts|p|blog|a|detail|content|archives?)/[^/?#]{4,}|\d{6,}\.s?html?$|/[a-z0-9]+(-[a-z0-9]+){3,}/?$",
    re.I,
)
SCHEMAS = {"article": "rex.schema.article.v1.Article"}


@dataclass
class Result:
    domain: str
    id: str = ""
    outcome: str = ""  # thin | thin-url | draft | failed | skipped | existing
    reason: str = ""
    pages: list[str] = field(default_factory=list)
    negative: str = ""
    problems: list[str] = field(default_factory=list)
    seconds: float = 0.0
    log: list[str] = field(default_factory=list)


async def run(*args: str, timeout: float = 600) -> tuple[int, str]:
    proc = await asyncio.create_subprocess_exec(*args, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT)
    try:
        out, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    except TimeoutError:
        proc.kill()
        return 124, "timeout"
    return proc.returncode or 0, out.decode("utf-8", errors="replace")


def recipe_host(url: str) -> str:
    host = (urlparse(url).hostname or "").lower()
    return host[4:] if host.startswith("www.") else host


DATE_SELECTORS = 'meta[property="article:published_time"], meta[itemprop="datePublished"], [itemprop="datePublished"], time[datetime]'


# A full date with a time of day in the visible text ("2026年10月13日 08:30", "2026-10-13 08:30").
TEXT_DATETIME = re.compile(r"20\d{2}\s*[-/.年]\s*\d{1,2}\s*[-/.月]\s*\d{1,2}\s*日?\s+\d{1,2}:\d{2}")


def has_publish_date(html: str) -> bool:
    """A publish date in metadata or markup, or a date and time in the text: articles have one, product and
    listing pages rarely do."""
    if '"datePublished"' in html:
        return True
    tree = LexborHTMLParser(html)
    if tree.css_first(DATE_SELECTORS) is not None:
        return True
    body = tree.body
    return bool(body and TEXT_DATETIME.search(body.text(separator=" ")[:20000]))


def page_signals(html: str) -> tuple[set[str], str]:
    tree = LexborHTMLParser(html)
    types: set[str] = set()

    def walk(v: object) -> None:
        if isinstance(v, dict):
            t = v.get("@type")
            for x in t if isinstance(t, list) else [t]:
                if isinstance(x, str):
                    types.add(x.rsplit("/", 1)[-1])
            for k in ("@graph", "mainEntity"):
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
    og = tree.css_first('meta[property="og:type"]')
    return types, ((og.attributes.get("content") or "").strip().lower() if og else "")


async def feed_items(client: httpx.AsyncClient, feed: str) -> list[str]:
    try:
        r = await client.get(feed)
        root = ElementTree.fromstring(r.content)
    except (httpx.HTTPError, ElementTree.ParseError, ValueError):
        return []
    out: list[str] = []
    for el in root.iter():
        tag = el.tag.rsplit("}", 1)[-1]
        if tag == "link":
            href = el.attrib.get("href") or (el.text or "").strip()
            if href.startswith("http") and el.attrib.get("rel", "alternate") == "alternate":
                out.append(href)
    return out


async def sitemap_urls(client: httpx.AsyncClient, base: str, limit: int = 300) -> list[str]:
    """Article URLs from the sitemaps named in robots.txt (one level of sitemap index, newest first)."""
    try:
        robots = (await client.get(urljoin(base, "/robots.txt"))).text
    except httpx.HTTPError:
        return []
    maps = [l.split(":", 1)[1].strip() for l in robots.splitlines() if l.lower().startswith("sitemap:")][:3]
    out: list[tuple[str, str]] = []
    for sm in maps:
        try:
            root = ElementTree.fromstring((await client.get(sm)).content)
        except (httpx.HTTPError, ElementTree.ParseError, ValueError):
            continue
        tag = root.tag.rsplit("}", 1)[-1]
        if tag == "sitemapindex":
            children = [
                (c.findtext("{*}lastmod") or "", c.findtext("{*}loc") or "")
                for c in root
                if c.tag.rsplit("}", 1)[-1] == "sitemap"
            ]
            children.sort(reverse=True)
            # Prefer news/post/article sitemaps, newest first.
            children.sort(key=lambda c: not re.search(r"news|post|article|blog|story", c[1], re.I))
            for _, loc in children[:2]:
                try:
                    sub = ElementTree.fromstring((await client.get(loc)).content)
                except (httpx.HTTPError, ElementTree.ParseError, ValueError):
                    continue
                out += [(u.findtext("{*}lastmod") or "", u.findtext("{*}loc") or "") for u in sub]
        else:
            out += [(u.findtext("{*}lastmod") or "", u.findtext("{*}loc") or "") for u in root]
        if len(out) > limit:
            break
    out.sort(reverse=True)
    return [loc.strip() for _, loc in out if loc.strip().startswith("http") and ARTICLE_PATH.search(urlparse(loc.strip()).path)][:limit]


def path_pattern(urls: list[str]) -> str | None:
    """An include regex for the common shape of article paths, or None when there is no constraining segment.

    Equal segments stay literal (except numbers: dates and ids vary), numeric segments become \\d+, the rest
    [^/]+ (keeping a shared file extension). At least one literal or numeric segment is required, so the
    pattern cannot be "any path of this depth"."""
    segs = [urlparse(u).path.strip("/").split("/") for u in urls]
    depth = Counter(len(x) for x in segs).most_common(1)[0][0]
    group = [x for x in segs if len(x) == depth]
    if len(group) < 2 or depth == 0:
        return None
    parts: list[str] = []
    constrained = False
    for i in range(depth):
        vals = {x[i] for x in group}
        if all(v.isdigit() for v in vals):
            parts.append(r"\d+")
            constrained = True
        elif len(vals) == 1:
            parts.append(re.escape(vals.pop()))
            constrained = True
        else:
            exts = {os.path.splitext(v)[1] for v in vals}
            ext = exts.pop() if len(exts) == 1 else ""
            parts.append(r"[^/]+" + (re.escape(ext) if ext else ""))
    return "^/" + "/".join(parts) + "/?$" if constrained else None


def on_site(url: str, domain: str) -> bool:
    h = recipe_host(url)
    return h == domain or h.endswith("." + domain)


def homepage_links(html: str, base: str, domain: str) -> list[str]:
    """Article-looking links on the site or any of its subdomains (blog.csdn.net, news.un.org, ...)."""
    out: list[str] = []
    for a in LexborHTMLParser(html).css("a[href]"):
        try:
            href = urljoin(base, a.attributes.get("href") or "").split("#")[0]
            u = urlparse(href)
        except ValueError:
            continue  # malformed href (e.g. a broken IPv6 literal)
        if u.scheme in ("http", "https") and on_site(href, domain) and ARTICLE_PATH.search(u.path):
            out.append(href)
    return out


def existing_hosts() -> set[str]:
    """Hosts already covered by a hand-written or previously harvested recipe."""
    hosts: set[str] = set()
    for f in (REPO / "recipes").rglob("*.yaml"):
        doc = yaml.safe_load(f.read_text()) or {}
        for h in ((doc.get("match") or {}).get("urls") or {}).get("hosts") or []:
            hosts.add(str(h).lower().removeprefix("*."))
    return hosts


def shape(url: str) -> tuple[int, str]:
    """URL shape for clustering: path depth and the first segment (numbers collapsed)."""
    segs = urlparse(url).path.strip("/").split("/")
    if len(segs) == 1:
        return 1, "*"  # /<slug>: the only segment is the article itself
    first = "#" if segs[0].isdigit() else segs[0]
    return len(segs), first


def clusters(urls: list[str]) -> list[list[str]]:
    """Links grouped by URL shape, largest group first, each in original order (RSS items first)."""
    groups: dict[tuple[int, str], list[str]] = {}
    for u in urls:
        groups.setdefault(shape(u), []).append(u)
    return sorted(groups.values(), key=len, reverse=True)


def pick(urls: list[str], n: int) -> list[str]:
    """Distinct URLs, preferring different first path segments (different sections)."""
    seen: set[str] = set()
    firsts: set[str] = set()
    chosen: list[str] = []
    for pass_ in (0, 1):
        for u in urls:
            if u in seen or len(chosen) >= n:
                continue
            first = urlparse(u).path.strip("/").split("/")[0]
            if pass_ == 0 and first in firsts:
                continue
            seen.add(u)
            firsts.add(first)
            chosen.append(u)
    return chosen


async def capture(url: str, rid: str, case: str, root: Path) -> Path | None:
    code, out = await run(str(REX), "fixture", "capture", url, "--id", rid, "--case", case, "--root", str(root), timeout=60)
    d = root / rid / case
    return d if code == 0 and (d / "page.html").exists() else None


def url_thin_recipe(rid: str, host: str, pages: list[Path]) -> str | None:
    urls = [json.loads((p / "meta.json").read_text())["url"] for p in pages]
    pattern = path_pattern(urls)
    if pattern is None:
        return None
    return (
        f"# Thin recipe generated by tools/harvest (decisions.md U31): the generic article recipe on this site's\n"
        f"# article URL shape.\n"
        f"id: {rid}\nversion: 1\nschema: rex.schema.article.v1.Article\nextends: generic/article.v1\n"
        f"match:\n  urls:\n    hosts: [\"{host}\"]\n    include: [{json.dumps(pattern)}]\n"
        f"fields:\n  - path: title\n    expect: true\n    candidates: [ {{ json_ld: {{}} }}, {{ meta: {{ key: \"og:title\" }} }}, {{ css: {{ selector: \"h1\" }} }} ]\n"
    )


FULL_CHARS = 600  # the gate's stub threshold


async def readable_chars(d: Path) -> int:
    meta = json.loads((d / "meta.json").read_text())
    code, out = await run(str(REX), "readability", "--html", str(d / "page.html"), "--url", meta["url"], timeout=60)
    if code != 0:
        return 0
    try:
        return len(json.loads(out).get("contentText", ""))
    except json.JSONDecodeError:
        return 0


def thin_recipe(rid: str, host: str, pages: list[Path], home: Path) -> str | None:
    sig = [page_signals((p / "page.html").read_text(errors="replace")) for p in pages]
    home_types, home_og = page_signals((home / "page.html").read_text(errors="replace"))
    common = set.intersection(*(t & ARTICLE_TYPES for t, _ in sig))
    if common and not (home_types & common):
        cond = f"json_ld_type: {{ types: [{', '.join(sorted(common))}] }}"
    elif all(og == "article" for _, og in sig) and home_og != "article":
        cond = 'meta: { key: "og:type", value: "article" }'
    else:
        return None
    return (
        f"# Thin recipe generated by tools/harvest (decisions.md U31): the generic article recipe on this site,\n"
        f"# confirmed by the page's own schema.org/Open Graph type.\n"
        f"id: {rid}\nversion: 1\nschema: rex.schema.article.v1.Article\nextends: generic/article.v1\n"
        f"match:\n  urls:\n    hosts: [\"{host}\"]\n  page:\n    any:\n      - {cond}\n"
        f"fields:\n  - path: title\n    expect: true\n    candidates: [ {{ json_ld: {{}} }}, {{ meta: {{ key: \"og:title\" }} }} ]\n"
    )


async def harvest_site(row: dict[str, str], args: argparse.Namespace, client: httpx.AsyncClient, llm: asyncio.Semaphore) -> Result:
    start = time.monotonic()
    domain = row["domain"]
    res = Result(domain=domain)
    home_url = row.get("final_url") or f"https://{domain}/"
    work: Path = args.work / "captures"
    tmp_id = f"{domain}/_"
    home = await capture(home_url, tmp_id, "home", work)
    if home is None:
        res.outcome, res.reason = "skipped", "homepage capture failed"
        return res
    meta = json.loads((home / "meta.json").read_text())
    res.negative = str(home)

    urls: list[str] = []
    for feed in json.loads(row.get("feeds") or "[]"):
        urls += [u for u in await feed_items(client, feed) if on_site(u, domain)]
    urls += homepage_links((home / "page.html").read_text(errors="replace"), meta["url"], domain)
    if len(urls) < 3:
        urls += [u for u in await sitemap_urls(client, meta["url"]) if on_site(u, domain)]
    urls = [u for u in urls if u.rstrip("/") != meta["url"].rstrip("/")]
    if not urls:
        res.outcome, res.reason = "skipped", "no article-like links in the RSS feeds or on the homepage"
        return res
    # Articles often live on one subdomain (blog.csdn.net): target the host with the most article links.
    host = Counter(recipe_host(u) for u in urls).most_common(1)[0][0]
    urls = [u for u in urls if recipe_host(u) == host]
    rid = f"{host}/article"
    res.id = rid
    if host in args.covered or domain in args.covered:
        res.outcome, res.reason = "existing", "a recipe already covers this host"
        return res
    # Try URL-shape clusters from the largest down (section and topic pages often form big clusters too) and
    # use the first that yields >= 2 article-like pages: an article type or a publish date (product and
    # listing pages rarely have one), preferring pages with enough readable text to judge.
    pages: list[Path] = []
    n = 0
    for cluster in clusters(urls)[:4]:
        ranked: list[tuple[int, Path]] = []
        for u in pick(cluster, 4):
            n += 1
            d = await capture(u, tmp_id, f"a{n}", work)
            if d is None:
                continue
            html = (d / "page.html").read_text(errors="replace")
            types, og = page_signals(html)
            if bool(types & ARTICLE_TYPES or og == "article") or has_publish_date(html):
                ranked.append((int(await readable_chars(d) >= FULL_CHARS), d))
            if sum(score for score, _ in ranked) == 3:
                break
        if len(ranked) >= 2:
            ranked.sort(key=lambda x: -x[0])
            pages = [d for _, d in ranked][:3]
            break
    if len(pages) < 2:
        res.outcome = "skipped"
        res.reason = f"no URL group with >= 2 article-like pages (article type or publish date) among {len(urls)} links"
        return res
    res.pages = [str(p) for p in pages]

    out_recipes: Path = args.out / "recipes"
    for kind, thin in (("thin", thin_recipe(rid, host, pages, home)), ("thin-url", url_thin_recipe(rid, host, pages))):
        if thin is None:
            continue
        path = out_recipes / f"{rid}.yaml"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(thin)
        gate_args = [str(REX), "gate", "--recipes", str(REPO / "recipes"), "--recipes", str(path), "--id", rid]
        for p in pages:
            gate_args += ["--page", str(p)]
        code, out = await run(*gate_args, "--negative", str(home))
        if code == 0:
            res.outcome = kind
            res.seconds = round(time.monotonic() - start, 1)
            return res
        path.unlink()
        res.log.append(f"{kind} recipe failed the gate: {out.strip()[:600]}")

    draft_args = [
        str(REX), "draft", "--from", str(pages[0]), "--id", rid, "--schema", SCHEMAS["article"],
        "--recipes", str(REPO / "recipes"), "--out", str(out_recipes), "--negative", str(home),
        "--repair-rounds", "2", "--inline-system", "--force",
    ]
    for p in pages[1:]:
        draft_args += ["--also", str(p)]
    pattern = path_pattern([json.loads((p / "meta.json").read_text())["url"] for p in pages])
    if pattern is not None:
        draft_args += ["--url-host", host, "--url-include", pattern]
    async with llm:
        code, out = await run(*draft_args, timeout=900)
    res.log.append(out.strip()[-1500:])
    if code == 0:
        res.outcome = "draft"
    else:
        res.outcome = "failed"
        res.problems = [l.strip() for l in out.splitlines() if l.startswith("  ")][:12]
        res.reason = "quality gate failed after repairs" if "quality gate FAILED" in out else (out.strip().splitlines() or ["?"])[-1][:300]
    res.seconds = round(time.monotonic() - start, 1)
    return res


async def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sites", type=Path, required=True)
    ap.add_argument("--work", type=Path, required=True)
    ap.add_argument("--out", type=Path)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--tier", default="")
    ap.add_argument("--domains", default="", help="comma-separated; overrides --tier/--limit")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--llm-jobs", type=int, default=2)
    args = ap.parse_args()
    args.work = args.work.expanduser()
    args.out = (args.out or args.work / "out").expanduser()
    args.covered = existing_hosts()
    if "REX_LLM_ENDPOINT" not in os.environ:
        raise SystemExit("set REX_LLM_ENDPOINT")
    results = args.out / "results"
    results.mkdir(parents=True, exist_ok=True)

    rows = list(csv.DictReader(args.sites.open()))
    if args.domains:
        wanted = set(args.domains.split(","))
        rows = [r for r in rows if r["domain"] in wanted]
    else:
        rows = [r for r in rows if r["schema"] == "article" and r["bot_wall"] != "True" and (not args.tier or r["tier"] == args.tier)]
        if args.limit:
            rows = rows[: args.limit]
    todo = [r for r in rows if not (results / f"{r['domain']}.json").exists()]
    print(f"{len(rows)} sites selected, {len(todo)} to harvest", flush=True)

    sem = asyncio.Semaphore(args.jobs)
    llm = asyncio.Semaphore(args.llm_jobs)
    async with httpx.AsyncClient(headers={"User-Agent": UA}, timeout=20, follow_redirects=True) as client:

        async def one(row: dict[str, str]) -> None:
            async with sem:
                try:
                    res = await harvest_site(row, args, client, llm)
                except Exception as e:  # one broken site must not stop the batch; it is recorded, not hidden
                    res = Result(domain=row["domain"], outcome="error", reason=f"{type(e).__name__}: {e}"[:300])
            (results / f"{row['domain']}.json").write_text(json.dumps(asdict(res), ensure_ascii=False, indent=1))
            print(f"{res.outcome:8} {row['domain']:32} {res.id:40} {res.seconds:6.1f}s {res.reason[:90]}", flush=True)

        await asyncio.gather(*(one(r) for r in todo))


if __name__ == "__main__":
    asyncio.run(main())
