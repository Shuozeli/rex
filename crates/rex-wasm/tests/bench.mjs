// wasm timing for the same ~1.27 MB page as the native criterion bench (engine spec §11).
// Usage: node crates/rex-wasm/tests/bench.mjs target/wasm-node target/conformance-bundle
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const [wasmDir, bundleDir] = process.argv.slice(2).map((p) => resolve(p));
const repo = resolve(new URL("../../..", import.meta.url).pathname);
const { RexEngine } = createRequire(import.meta.url)(join(wasmDir, "rex_wasm.js"));
const index = JSON.parse(Buffer.from(JSON.parse(readFileSync(join(bundleDir, "index.json"), "utf8")).payload, "base64").toString("utf8"));
const fds = readFileSync(join(bundleDir, "s", `${index.schemas.sha256}.fds`));
const recipes = index.entries.map((e) => readFileSync(join(bundleDir, "r", `${e.ref.sha256}.json`), "utf8"));
const engine = new RexEngine(fds, recipes);
const para = '<p>Federal Reserve officials held interest rates steady on Wednesday, signaling patience as price growth slowed for a third straight month, analysts said.</p>\n<div class="ad">Advertisement</div>\n';
const src = readFileSync(join(repo, "fixtures/wsj.com/main/article/page.html"), "utf8");
const html = src.replace("<p>Markets had largely", para.repeat(6500) + "<p>Markets had largely");
const url = "https://www.wsj.com/articles/fed-holds-rates-steady-4f2a";
const runs = [];
for (let i = 0; i < 7; i++) {
  const t0 = performance.now();
  const out = JSON.parse(engine.extract(url, html, undefined, 1791727200000, "{}", "bench"));
  runs.push(performance.now() - t0);
  if (out.extraction?.status !== "EXTRACTION_STATUS_OK") throw new Error("unexpected status");
}
runs.sort((a, b) => a - b);
console.log(`wasm parse+match+extract ${(html.length / 1e6).toFixed(2)} MB: median ${runs[3].toFixed(0)} ms (min ${runs[0].toFixed(0)}, max ${runs[6].toFixed(0)})`);
