// rex-readability vs Mozilla Readability.js on the private corpus: token-set Jaccard of the article
// text (same tokenizer as the engine's agreement metric: lowercase alphanumeric words + CJK chars).
// Usage: pnpm install && node bench.mjs [path/to/rex]
import { Readability } from "@mozilla/readability";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { JSDOM, VirtualConsole } from "jsdom";
import { join, resolve } from "node:path";

const repo = resolve(new URL("../..", import.meta.url).pathname);
const rex = process.argv[2] ?? join(repo, "target/release/rex");
const dir = join(repo, "fixtures-private", "corpus");
const corpus = JSON.parse(readFileSync(join(dir, "corpus.json"), "utf8"));
const isCjk = (c) => /[぀-ヿ㐀-䶿一-鿿豈-﫿가-힯]/.test(c);

function tokens(s) {
  const out = new Set();
  let word = "";
  for (const c of s) {
    if (isCjk(c)) {
      if (word) out.add(word), (word = "");
      out.add(c);
    } else if (/[\p{L}\p{N}]/u.test(c)) {
      word += c.toLowerCase();
    } else if (word) {
      out.add(word);
      word = "";
    }
  }
  if (word) out.add(word);
  return out;
}

function jaccard(a, b) {
  const ta = tokens(a), tb = tokens(b);
  let inter = 0;
  for (const t of ta) if (tb.has(t)) inter++;
  const union = ta.size + tb.size - inter;
  return union === 0 ? null : inter / union;
}

const rows = [];
for (const { file, url } of corpus) {
  const html = readFileSync(join(dir, file), "utf8");
  const dom = new JSDOM(html, { url, virtualConsole: new VirtualConsole() });
  const moz = new Readability(dom.window.document).parse();
  const ours = JSON.parse(execFileSync(rex, ["readability", "--html", join(dir, file), "--url", url], { encoding: "utf8", maxBuffer: 64 << 20 }));
  const mozText = moz?.textContent ?? "";
  const j = jaccard(ours.contentText, mozText);
  rows.push({ file, host: new URL(url).hostname, j, ours: ours.contentText.length, moz: mozText.trim().length });
}
rows.sort((a, b) => (a.j ?? -1) - (b.j ?? -1));
for (const r of rows) {
  console.log(`${r.j === null ? "  n/a" : r.j.toFixed(3)}  ${r.file}  ${r.host.padEnd(28)} rex ${String(r.ours).padStart(7)} chars  mozilla ${String(r.moz).padStart(7)} chars`);
}
const js = rows.map((r) => r.j).filter((x) => x !== null).sort((a, b) => a - b);
const median = js.length ? js[Math.floor(js.length / 2)] : 0;
console.log(`\n${rows.length} pages, median token-set Jaccard ${median.toFixed(3)} (target >= 0.9), p25 ${js[Math.floor(js.length / 4)]?.toFixed(3)}`);
process.exit(median >= 0.9 ? 0 : 1);
