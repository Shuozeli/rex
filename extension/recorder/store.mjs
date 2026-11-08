// Chrome Web Store screenshots (store/README.md): 1280x800, full bleed, the real flow on real pages. The desktop
// runs at 1280x800, so each shot is the whole screen: the page with the rex popup open under its toolbar icon.
//
// Run inside the live desktop with the store build in extension/dist, once per UI language:
//   REX_RECORDER_SCREEN=1280x800 REX_RECORDER_CHROME_ARGS=--lang=en-US \
//     docker compose -f extension/recorder/docker/compose.yaml up -d --force-recreate desktop
//   docker compose -f extension/recorder/docker/compose.yaml exec desktop \
//     node /repo/extension/recorder/store.mjs --locale en --out /out/store/en
// (zh_CN: --lang=zh-CN and --locale zh_CN.) Then copy the PNGs to store/assets/<locale>/.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { attachWorker, connect, evaluate, sleep } from "./lib/cdp.mjs";
import * as desk from "./lib/desktop.mjs";

const args = process.argv.slice(2);
const opt = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback);
const locale = opt("--locale", "en");
const out = opt("--out", `/out/store/${locale}`);
const CDP = "http://127.0.0.1:9222";
const EXT = readFileSync("/tmp/rex-ext-id", "utf8").trim();
mkdirSync(out, { recursive: true });

// Five shots per locale (the store shows up to five): articles, lists, a quote, the Table view, and the JSON that
// Copy JSON copies. Only pages without third-party ads (YouTube plays one first; recipe sites run banners).
// `view`: "table" switches to the Table view, "json" scrolls to the JSON panel; `settle`: ms to let the page render;
// `dismiss`: the site's own close button for a prompt over the page.
const PAGES = {
  en: [
    { url: "https://en.wikipedia.org/wiki/Web_scraping" },
    { url: "https://news.ycombinator.com/" },
    { url: "https://github.com/trending", view: "table" },
    { url: "https://finance.yahoo.com/quote/AAPL/" },
    { url: "https://en.wikipedia.org/wiki/JSON", view: "json" },
  ],
  // zh.wikipedia asks which script variant to remember; its own close button dismisses that. No Xueqiu (a login
  // dialog covers the page), no video pages (other people's footage), no sspai (its header art renders late), no
// Eastmoney (ad banners).
  zh_CN: [
    { url: "https://zh.wikipedia.org/zh-cn/%E7%BD%91%E9%A1%B5%E6%8A%93%E5%8F%96", dismiss: ".va-variant-prompt__close" },
    { url: "https://www.ruanyifeng.com/blog/2026/10/weekly-issue-414.html" },
    { url: "https://github.com/trending", view: "table" },
    { url: "https://news.ycombinator.com/" },
    { url: "https://zh.wikipedia.org/zh-cn/JSON", view: "json", dismiss: ".va-variant-prompt__close" },
  ],
}[locale];
if (!PAGES) throw new Error(`unknown --locale ${locale} (en, zh_CN)`);

async function waitFor(fn, ms, every = 150) {
  const t = Date.now();
  while (Date.now() - t < ms) {
    const v = await fn().catch(() => null);
    if (v) return v;
    await sleep(every);
  }
  return null;
}

/** The store wants 24-bit PNG (no alpha) at exactly 1280x800. */
function save(raw, dst) {
  execFileSync("python3", [
    "-c",
    `import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
assert im.size == (1280, 800), f"screen is {im.size}, not 1280x800 (REX_RECORDER_SCREEN=1280x800)"
im.save(sys.argv[2], optimize=True)`,
    raw,
    dst,
  ]);
}

const conn = await connect(CDP);
await attachWorker(conn, EXT);
const isPage = (t) => t.type === "page" && !t.url.startsWith("devtools://");
const isPopup = (t) => t.type === "page" && t.url.startsWith(`chrome-extension://${EXT}/popup.html`);
const tab = (await conn.send("Target.getTargets")).targetInfos.find((t) => isPage(t) && !isPopup(t));
const page = (await conn.send("Target.attachToTarget", { targetId: tab.targetId, flatten: true })).sessionId;
const shots = [];

for (const [i, p] of PAGES.entries()) {
  desk.key("ctrl+l");
  await sleep(250);
  desk.type(p.url, 20);
  desk.key("Return");
  await waitFor(async () => (await evaluate(conn, page, "document.readyState", 3000)) === "complete" && (await evaluate(conn, page, "location.href", 3000)) !== "about:blank", 30000);
  await sleep(p.settle ?? 4000);
  if (p.dismiss) {
    await evaluate(conn, page, `(() => { document.querySelector(${JSON.stringify(p.dismiss)})?.click(); return 1; })()`, 3000);
    await sleep(500);
  }
  const icon = desk.locateIcon("/ext/icons/icon-16.png");
  const before = new Set((await conn.send("Target.getTargets")).targetInfos.filter(isPopup).map((t) => t.targetId));
  await desk.click(icon.x, icon.y, 500);
  const target = await waitFor(async () => (await conn.send("Target.getTargets")).targetInfos.find((t) => isPopup(t) && !before.has(t.targetId)), 8000);
  const popup = (await conn.send("Target.attachToTarget", { targetId: target.targetId, flatten: true })).sessionId;
  const shown = await waitFor(
    () => evaluate(conn, popup, `document.querySelector("#record [data-field], #message:not([hidden])") ? { status: document.getElementById("status").dataset.status, meta: document.getElementById("meta").innerText } : null`, 3000),
    20000,
  );
  if (p.view === "table") await evaluate(conn, popup, `(() => { document.getElementById("viewTable").click(); scrollTo(0, 0); return 1; })()`, 3000);
  if (p.view === "json") await evaluate(conn, popup, `(() => { document.getElementById("json").scrollIntoView({ block: "end" }); return 1; })()`, 3000);
  // The pointer on the empty tab strip: over the popup or the page it would hover a link and show its URL.
  await desk.moveTo(760, 18, 300);
  await sleep(1000);
  const raw = join(out, `screenshot-${i + 1}.raw.png`);
  desk.screenshot(raw);
  save(raw, join(out, `screenshot-${i + 1}.png`));
  // The view is remembered: back to Reader for the next page.
  if (p.view === "table") await evaluate(conn, popup, `(() => { document.getElementById("viewReader").click(); return 1; })()`, 3000);
  desk.key("Escape");
  await sleep(700);
  shots.push({ file: `screenshot-${i + 1}.png`, url: p.url, view: p.view ?? "reader", status: shown?.status, meta: shown?.meta.split("\n")[0] });
  console.log(`screenshot-${i + 1}: ${shown?.status} ${shown?.meta.split("\n")[0]} (${p.url})`);
}

writeFileSync(join(out, "shots.json"), JSON.stringify({ at: new Date().toISOString(), locale, shots }, null, 2));
conn.close();
