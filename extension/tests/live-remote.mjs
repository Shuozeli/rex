// Live-page test in a real, separately running Chrome with the e2e build of the extension loaded (decisions.md
// U38). Unlike tests/e2e.mjs (hermetic fixtures, local headless Chrome), this opens the real pages over the
// browser's own network, lets them render, and extracts through the extension (serialize in the tab -> wasm in
// the service worker) with the recipes packaged in that build. Values change on live pages, so each case checks
// the matched recipe, the status, and which fields of the fixture's expected record the live record lacks.
//
// It speaks raw CDP to the extension's service worker only. The worker opens each page itself (chrome.tabs),
// waits for it to load, extracts and closes it; no debugger attaches to the pages. (Playwright attached to every
// page, stalled on pages that stream updates, and left the browser refusing a second connection.)
//
// Usage: node tests/live-remote.mjs --ext-id <id> [--cdp http://127.0.0.1:9530] [--only <substr>[,<substr>...]]
//                                    [--out test-results/live] [--save-html] [--cases <file>]
// --ext-id: the unpacked extension's id in that browser (it follows from the install path).
// The browser: Chrome for Testing started with --load-extension=<e2e dist> (see docs/operations.md).

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i < 0 ? fallback : args[i + 1];
};
const repo = resolve(import.meta.dirname, "../..");
const cdp = opt("--cdp", "http://127.0.0.1:9530");
const only = opt("--only", "");
const extId = opt("--ext-id", "");
const saveHtml = args.includes("--save-html");
const out = resolve(opt("--out", join(repo, "extension/test-results/live")));
const casesFile = resolve(opt("--cases", join(repo, "extension/tests/live-cases.json")));
mkdirSync(out, { recursive: true });
if (!extId) throw new Error("--ext-id is required");

const { cases } = JSON.parse(readFileSync(casesFile, "utf8"));
const wanted = only.split(",").filter(Boolean);
const selected = cases.filter((c) => wanted.length === 0 || wanted.some((w) => c.recipe.includes(w)));

/** A minimal CDP client over the browser websocket (flattened sessions). */
async function connect(httpBase) {
  const { webSocketDebuggerUrl } = await (await fetch(`${httpBase}/json/version`)).json();
  const ws = new WebSocket(webSocketDebuggerUrl);
  await new Promise((ok, fail) => {
    ws.onopen = ok;
    ws.onerror = () => fail(new Error(`cannot open ${webSocketDebuggerUrl}`));
  });
  let next = 1;
  const pending = new Map();
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    const p = msg.id && pending.get(msg.id);
    if (!p) return;
    pending.delete(msg.id);
    msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result);
  };
  const send = (method, params = {}, sessionId) =>
    new Promise((resolveP, reject) => {
      const id = next++;
      pending.set(id, { resolve: resolveP, reject });
      ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  return { send, close: () => ws.close() };
}

/** Evaluates an async expression in the service worker and returns its value. */
async function inWorker(conn, sessionId, expression, timeoutMs) {
  const res = await Promise.race([
    conn.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId),
    new Promise((_, reject) => setTimeout(() => reject(new Error(`worker call timed out after ${timeoutMs / 1000}s`)), timeoutMs)),
  ]);
  if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description?.split("\n")[0] ?? res.exceptionDetails.text);
  return res.result.value;
}

/** Field paths present in a record: top-level keys, and "a.b" for keys of objects (and of objects in lists). */
function fieldSet(record) {
  const out = new Set();
  for (const [k, v] of Object.entries(record ?? {})) {
    if (k === "@type") continue;
    out.add(k);
    const objs = Array.isArray(v) ? v.filter((x) => x && typeof x === "object") : v && typeof v === "object" ? [v] : [];
    for (const o of objs) for (const kk of Object.keys(o)) out.add(`${k}.${kk}`);
  }
  return out;
}

const conn = await connect(cdp);
const workerTarget = async () =>
  (await conn.send("Target.getTargets")).targetInfos.find((t) => t.type === "service_worker" && t.url.startsWith(`chrome-extension://${extId}/`));
let target = await workerTarget();
if (!target) {
  // An idle MV3 worker is stopped; opening an extension page starts it again.
  const { targetId } = await conn.send("Target.createTarget", { url: `chrome-extension://${extId}/options.html` });
  for (let i = 0; i < 60 && !target; i++) {
    await new Promise((r) => setTimeout(r, 500));
    target = await workerTarget();
  }
  await conn.send("Target.closeTarget", { targetId });
}
if (!target) throw new Error(`no service worker for extension ${extId}: is the e2e build loaded?`);
let sessionId = (await conn.send("Target.attachToTarget", { targetId: target.targetId, flatten: true })).sessionId;
/** Chrome may stop and restart the MV3 worker (after a long call, or when idle); attach to the current one. */
async function reattach() {
  let t = await workerTarget();
  if (!t) {
    const { targetId } = await conn.send("Target.createTarget", { url: `chrome-extension://${extId}/options.html` });
    for (let i = 0; i < 60 && !t; i++) {
      await new Promise((r) => setTimeout(r, 500));
      t = await workerTarget();
    }
    await conn.send("Target.closeTarget", { targetId });
  }
  if (!t) throw new Error("the extension's service worker did not come back");
  sessionId = (await conn.send("Target.attachToTarget", { targetId: t.targetId, flatten: true })).sessionId;
}
const worker = (expr, ms) => inWorker(conn, sessionId, expr, ms);

const status = await worker("globalThis.rexDebug.status()", 60000).catch(async (e) => {
  if (!/Session with given id not found|No target with given id/i.test(String(e))) throw e;
  await reattach();
  return worker("globalThis.rexDebug.status()", 60000);
});
const { product } = await conn.send("Browser.getVersion");
console.log(`${product} · ${status.recipes?.count ?? 0} packaged recipes (built ${status.recipes?.generatedAt})`);

// Runs in the worker: open the page as a user would, wait for it to load and settle, extract, close it.
const visit = (url, keepHtml) => `(async () => {
  const timed = (p, ms, what) => Promise.race([p, new Promise((_, no) => setTimeout(() => no(new Error(what + " took over " + ms / 1000 + "s")), ms))]);
  const tab = await chrome.tabs.create({ url: ${JSON.stringify(url)}, active: true });
  try {
    await new Promise((done) => {
      const timer = setTimeout(done, 45000);
      const onUpdated = (id, info) => {
        if (id === tab.id && info.status === "complete") { clearTimeout(timer); chrome.tabs.onUpdated.removeListener(onUpdated); done(); }
      };
      chrome.tabs.onUpdated.addListener(onUpdated);
    });
    await new Promise((r) => setTimeout(r, 5000));   // client-rendered content and lazy blocks
    const info = await chrome.tabs.get(tab.id);
    // A page that replaces its document after load (redirect, prerender activation) drops the first frame;
    // read it again once.
    const read = (f) => f().catch(async (e) => {
      if (!/Frame with ID \d+ was removed|No frame with id/.test(String(e))) throw e;
      await new Promise((r) => setTimeout(r, 3000));
      return f();
    });
    const [{ result: page }] = await read(() => timed(chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: (keep) => ({ visibility: document.visibilityState, html: keep ? (document.doctype ? "<!DOCTYPE html>" : "") + document.documentElement.outerHTML : null }),
      args: [${keepHtml ? "true" : "false"}],
    }), 30000, "reading the page"));
    const t0 = Date.now();
    let res = await timed(globalThis.rexDebug.extractTab(tab.id), 40000, "extraction");
    if (!res.ok && /Frame with ID \d+ was removed/.test(res.error)) {
      await new Promise((r) => setTimeout(r, 3000));
      res = await globalThis.rexDebug.extractTab(tab.id);
    }
    return { finalUrl: info.url, title: info.title, visibility: page.visibility, html: page.html, ms: Date.now() - t0, res };
  } finally {
    await chrome.tabs.remove(tab.id).catch(() => {});
  }
})()`;

const results = [];
for (const c of selected) {
  const expected = JSON.parse(readFileSync(join(repo, c.fixture, "expected.json"), "utf8"));
  const r = { recipe: c.recipe, url: c.url, note: c.note };
  const name = (c.name ?? c.recipe).replaceAll("/", "__");
  try {
    const v = await worker(visit(c.url, saveHtml), 120000);
    Object.assign(r, { title: (v.title ?? "").slice(0, 80), visibility: v.visibility, extractMs: v.ms });
    if (!v.res.ok) throw new Error(v.res.error);
    const ex = v.res.output.extraction;
    r.matched = ex?.recipe?.id ?? null;
    r.status = ex?.status?.replace("EXTRACTION_STATUS_", "") ?? null;
    r.detected = v.res.output.plan?.detectedTypes ?? [];
    const want = fieldSet(expected.record);
    const got = fieldSet(ex?.record);
    r.missing = [...want].filter((f) => !got.has(f));
    r.fields = `${[...want].filter((f) => got.has(f)).length}/${want.size}`;
    r.verdict = r.matched !== c.recipe ? "WRONG-RECIPE" : r.status !== "OK" ? "BAD-STATUS" : r.missing.length > 0 ? "PARTIAL" : "OK";
    writeFileSync(join(out, `${name}.json`), JSON.stringify({ url: c.url, finalUrl: v.finalUrl, title: r.title, plan: v.res.output.plan, extraction: ex }, null, 2));
    if (v.html) writeFileSync(join(out, `${name}.html`), v.html);
  } catch (e) {
    r.verdict = "ERROR";
    r.error = String(e.message ?? e).split("\n")[0].slice(0, 160);
    // A stuck call, or a worker Chrome stopped and replaced (its session is gone): attach to the current one.
    if (/timed out|Session with given id not found|No target with given id|Target closed/i.test(r.error)) {
      await reattach().catch((err) => console.log(`reattach failed: ${err.message}`));
    }
  }
  results.push(r);
  const detail =
    r.verdict === "ERROR"
      ? r.error
      : `${r.matched} ${r.status} fields ${r.fields} (${r.extractMs} ms)${r.missing?.length ? ` missing ${r.missing.slice(0, 6).join(",")}` : ""}`;
  const why = r.verdict === "OK" || r.verdict === "ERROR" ? "" : ` [page: "${r.title}"; ${r.visibility}; types ${JSON.stringify(r.detected)}]`;
  console.log(`${r.verdict.padEnd(12)} ${c.recipe.padEnd(28)} ${detail}${why}${c.note ? ` (${c.note})` : ""}`);
}
writeFileSync(join(out, "summary.json"), JSON.stringify(results, null, 2));
const tally = results.reduce((m, r) => ((m[r.verdict] = (m[r.verdict] ?? 0) + 1), m), {});
console.log(`\n${results.length} live pages: ${Object.entries(tally).map(([k, v]) => `${v} ${k}`).join(", ")}`);
conn.close();
