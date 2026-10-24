"""Scrape Chinaz (站长之家) rankings: the overall Chinese top list and selected industry lists.

Writes raw/chinaz.jsonl: one {list, rank, name, domain, intro} per site. Polite: one request at a time,
~1 s apart.
Run: uv run --with httpx --with selectolax scripts/chinaz.py
"""

import asyncio
import json
import os
from pathlib import Path

import httpx
from selectolax.lexbor import LexborHTMLParser as HTMLParser

# Survey data (raw rankings, work files, outputs) lives outside the repo.
ROOT = Path(os.environ.get("REX_SURVEY_DIR", "~/projects/scratch/rex-site-survey")).expanduser()
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0 Safari/537.36"
LISTS: dict[str, tuple[str, int]] = {
    "all": ("https://top.chinaz.com/all/index{}.html", 40),
    "news": ("https://top.chinaz.com/hangye/index_news{}.html", 20),
    "wangluo": ("https://top.chinaz.com/hangye/index_wangluo{}.html", 10),
    "jiaoyu": ("https://top.chinaz.com/hangye/index_jiaoyu{}.html", 5),
    "yule": ("https://top.chinaz.com/hangye/index_yule{}.html", 5),
}


def parse(html: str, list_name: str, offset: int) -> list[dict[str, object]]:
    out: list[dict[str, object]] = []
    for i, li in enumerate(HTMLParser(html).css("ul.listCentent > li")):
        name = li.css_first("h3 a")
        domain = li.css_first("h3 span.col-gray")
        intro = li.css_first("p.RtCInfo")
        if name is None or domain is None:
            continue
        out.append(
            {
                "list": list_name,
                "rank": offset + i + 1,
                "name": name.text(strip=True),
                "domain": domain.text(strip=True).lower(),
                "intro": (intro.text(strip=True).removeprefix("网站简介：") if intro else ""),
            }
        )
    return out


async def main() -> None:
    rows: list[dict[str, object]] = []
    async with httpx.AsyncClient(headers={"User-Agent": UA}, timeout=30, follow_redirects=True) as client:
        for name, (pattern, pages) in LISTS.items():
            for page in range(1, pages + 1):
                url = pattern.format("" if page == 1 else f"_{page}")
                resp = await client.get(url)
                resp.raise_for_status()
                got = parse(resp.text, name, (page - 1) * 30)
                if not got:
                    raise RuntimeError(f"no sites parsed from {url}")
                rows.extend(got)
                await asyncio.sleep(1.0)
            print(f"{name}: {sum(1 for r in rows if r['list'] == name)} sites")
    with (ROOT / "raw" / "chinaz.jsonl").open("w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    asyncio.run(main())
