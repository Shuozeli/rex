// Capture a private readability corpus (publisher content: never commit it).
// Writes <repo>/fixtures-private/corpus/<n>.html plus corpus.json [{file, url}].
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const repo = resolve(new URL("../..", import.meta.url).pathname);
const out = join(repo, "fixtures-private", "corpus");
mkdirSync(out, { recursive: true });
const { urls, feeds } = JSON.parse(readFileSync(new URL("sources.json", import.meta.url), "utf8"));
const UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0 Safari/537.36";

async function get(url) {
  const res = await fetch(url, { headers: { "User-Agent": UA, "Accept-Language": "en,zh;q=0.8" }, redirect: "follow", signal: AbortSignal.timeout(30000) });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return { url: res.url, text: await res.text() };
}

const targets = [...urls];
for (const f of feeds) {
  try {
    const { text } = await get(f.url);
    const links = [...text.matchAll(/<item[\s\S]*?<link>([^<]+)<\/link>|<entry[\s\S]*?<link[^>]*href="([^"]+)"/g)]
      .map((m) => (m[1] ?? m[2]).trim().replace(/^<!\[CDATA\[|\]\]>$/g, ""))
      .filter((u) => u.startsWith("http"));
    targets.push(...links.slice(0, f.take));
  } catch (e) {
    console.log(`feed ${f.url}: ${e.message}`);
  }
}

const corpus = [];
let n = 0;
for (const url of targets) {
  try {
    const page = await get(url);
    const file = `${String(++n).padStart(3, "0")}.html`;
    writeFileSync(join(out, file), page.text);
    corpus.push({ file, url: page.url });
    console.log(`ok  ${file} ${page.url} (${page.text.length} bytes)`);
  } catch (e) {
    console.log(`ERR ${url}: ${e.message}`);
  }
}
writeFileSync(join(out, "corpus.json"), JSON.stringify(corpus, null, 2));
console.log(`\n${corpus.length} pages -> ${out}`);
