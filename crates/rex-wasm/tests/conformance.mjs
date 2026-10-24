// Runs every fixture (fixtures/) and conformance case (conformance/) through the wasm engine in Node and
// requires the full Extraction to equal the native expected.json (engine spec §14: byte-identical output).
//
// Usage (from the repo root):
//   cargo build -p rex-wasm --target wasm32-unknown-unknown --profile wasm
//   wasm-bindgen --target nodejs --out-dir target/wasm-node target/wasm32-unknown-unknown/wasm/rex_wasm.wasm
//   rex bundle build --key <key> --out target/conformance-bundle
//   node crates/rex-wasm/tests/conformance.mjs target/wasm-node target/conformance-bundle
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { gunzipSync } from "node:zlib";

const [wasmDir, bundleDir] = process.argv.slice(2).map((p) => resolve(p));
const repo = resolve(new URL("../../..", import.meta.url).pathname);
const rex = join(repo, "target/release/rex");
const { RexEngine } = createRequire(import.meta.url)(join(wasmDir, "rex_wasm.js"));

const index = JSON.parse(Buffer.from(JSON.parse(readFileSync(join(bundleDir, "index.json"), "utf8")).payload, "base64").toString("utf8"));
const fds = readFileSync(join(bundleDir, "s", `${index.schemas.sha256}.fds`));
const bundleRecipes = index.entries.map((e) => readFileSync(join(bundleDir, "r", `${e.ref.sha256}.json`), "utf8"));
const shared = new RexEngine(fds, bundleRecipes);

function cases(root) {
  const out = [];
  const walk = (d) => {
    if (existsSync(join(d, "meta.json"))) out.push(d);
    for (const e of readdirSync(d)) {
      const p = join(d, e);
      if (statSync(p).isDirectory()) walk(p);
    }
  };
  if (existsSync(root)) walk(root);
  return out.sort();
}

let pass = 0;
let fail = 0;
let totalMs = 0;
for (const dir of [...cases(join(repo, "fixtures")), ...cases(join(repo, "conformance"))]) {
  const meta = JSON.parse(readFileSync(join(dir, "meta.json"), "utf8"));
  // Large pages are stored gzip-compressed (page.html.gz), like the rex-fixtures repo.
  const html = existsSync(join(dir, "page.html"))
    ? readFileSync(join(dir, "page.html"), "utf8")
    : gunzipSync(readFileSync(join(dir, "page.html.gz"))).toString("utf8");
  let engine = shared;
  let recipeId = meta.recipe;
  if (existsSync(join(dir, "recipe.yaml"))) {
    const local = execFileSync(rex, ["fmt", "--json", join(dir, "recipe.yaml")], { encoding: "utf8" }).trim();
    recipeId = recipeId ?? JSON.parse(local).id;
    engine = new RexEngine(fds, [...bundleRecipes, local]);
  }
  const t0 = performance.now();
  const out = JSON.parse(engine.extract(meta.url, html, recipeId, Date.parse(meta.fetch_time), JSON.stringify(meta.hints ?? {}), "test"));
  totalMs += performance.now() - t0;
  if (engine !== shared) engine.free();
  const expected = JSON.stringify(JSON.parse(readFileSync(join(dir, "expected.json"), "utf8")));
  const got = JSON.stringify(out.extraction);
  const name = dir.slice(repo.length + 1);
  if (got === expected) {
    pass++;
    console.log(`ok   ${name}`);
  } else {
    fail++;
    let i = 0;
    while (i < got.length && got[i] === expected[i]) i++;
    console.log(`FAIL ${name}\n  wasm:   …${got.slice(Math.max(0, i - 60), i + 80)}\n  native: …${expected.slice(Math.max(0, i - 60), i + 80)}`);
  }
}
console.log(`\n${pass} identical, ${fail} different (wasm extract total ${totalMs.toFixed(0)} ms)`);
process.exit(fail === 0 ? 0 : 1);
