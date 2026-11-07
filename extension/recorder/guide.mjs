// Screenshots for the user guide (docs/user-guide.md), taken in the recorder desktop with the real flow: type
// the URL, click the rex icon, wait for the popup. Each shot is the full screen plus crops of the popup and the
// toolbar, so the guide shows what a user sees.
//
// Run inside the live desktop (real pages; the extension as built in extension/dist, preferably the store build):
//   docker compose -f extension/recorder/docker/compose.yaml exec desktop \
//     node /repo/extension/recorder/guide.mjs --out /out/guide
// Then copy the PNGs to docs/user-guide/.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { attachWorker, connect, evaluate, sleep } from "./lib/cdp.mjs";
import * as desk from "./lib/desktop.mjs";

const args = process.argv.slice(2);
const out = args.includes("--out") ? args[args.indexOf("--out") + 1] : "/out/guide";
const CDP = "http://127.0.0.1:9222";
const EXT = readFileSync("/tmp/rex-ext-id", "utf8").trim();
mkdirSync(out, { recursive: true });

// `table`: also shoot the Table view; `json`: also the JSON panel at the bottom.
const PAGES = [
  { name: "wikipedia", url: "https://en.wikipedia.org/wiki/Web_scraping", scroll: true, table: true, json: true },
  { name: "hn", url: "https://news.ycombinator.com/", table: true },
  { name: "bilibili", url: "https://www.bilibili.com/video/BV149WF6zEw2/" },
  { name: "xueqiu", url: "https://www.xueqiu.com/S/SH600519", table: true },
  { name: "recipe", url: "https://www.allrecipes.com/recipe/10813/best-chocolate-chip-cookies/" },
  { name: "nomatch", url: "https://example.com/" },
];

/** Crops and scales a screenshot with PIL: box = [left, top, right, bottom], width = output width. */
function crop(src, dst, box, width) {
  execFileSync("python3", [
    "-c",
    `import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
box = [int(v) for v in sys.argv[3].split(",")]
if box[2] > box[0]: im = im.crop(box)
w = int(sys.argv[4])
if w and im.width != w: im = im.resize((w, round(im.height * w / im.width)), Image.LANCZOS)
im.save(sys.argv[2], optimize=True)`,
    src,
    dst,
    box.join(","),
    String(width),
  ]);
}

/**
 * Where the popup is: the region that changed between a shot without it and one with it, searched below the
 * toolbar near the icon, then sized to the popup's own viewport (w x h, plus its 1 px frame).
 */
function popupBox(before, after, iconX, w, h) {
  const out = execFileSync("python3", [
    "-c",
    `import sys
from PIL import Image, ImageChops
a = Image.open(sys.argv[1]).convert("L"); b = Image.open(sys.argv[2]).convert("L")
x0 = max(0, int(sys.argv[3]) - int(sys.argv[4]) - 80)
region = (x0, 68, a.width, a.height)
d = ImageChops.difference(a.crop(region), b.crop(region)).point(lambda v: 255 if v > 24 else 0)
bb = d.getbbox()
print(bb[0] + x0, bb[1] + 68, bb[2] + x0, bb[3] + 68)`,
    before,
    after,
    String(iconX),
    String(w),
  ], { encoding: "utf8" });
  const [l, t] = out.trim().split(/\s+/).map(Number);
  // Left and top edges from the change; the right side can include unrelated changes (the page's scrollbar),
  // so the width and height come from the popup itself.
  return [l, t, l + w + 2, t + h + 2];
}

async function waitFor(fn, ms, every = 150) {
  const t = Date.now();
  while (Date.now() - t < ms) {
    const v = await fn().catch(() => null);
    if (v) return v;
    await sleep(every);
  }
  return null;
}

const conn = await connect(CDP);
const sw = await attachWorker(conn, EXT);
const worker = (expr, ms) => evaluate(conn, sw, expr, ms);
const isPage = (t) => t.type === "page" && !t.url.startsWith("chrome-extension://") && !t.url.startsWith("devtools://");
const isPopup = (t) => t.type === "page" && t.url.startsWith(`chrome-extension://${EXT}/popup.html`);
// The browser's tab, whatever it shows (a page, or the options page left from a previous run).
const tab = (await conn.send("Target.getTargets")).targetInfos.find((t) => isPage(t) || (t.type === "page" && !isPopup(t)));
const page = (await conn.send("Target.attachToTarget", { targetId: tab.targetId, flatten: true })).sessionId;
const shots = [];
let anchor = null;

async function open(url) {
  desk.key("ctrl+l");
  await sleep(250);
  desk.type(url, 20);
  desk.key("Return");
  await waitFor(async () => (await evaluate(conn, page, "location.href", 3000)) !== "about:blank" && (await evaluate(conn, page, "document.readyState", 3000)) === "complete", 30000);
  await sleep(3500);
}

for (const p of PAGES) {
  await open(p.url);
  await desk.moveTo(900, 700, 200);
  const full = join(out, `${p.name}-page.raw.png`);
  desk.screenshot(full);
  const icon = desk.locateIcon("/ext/icons/icon-16.png");
  // The toolbar around the rex icon: the green dot says this site has a recipe of its own.
  crop(full, join(out, `${p.name}-toolbar.png`), [icon.x - 260, 0, icon.x + 110, 90], 740);

  const before = new Set((await conn.send("Target.getTargets")).targetInfos.filter(isPopup).map((t) => t.targetId));
  await desk.click(icon.x, icon.y, 500);
  const popupTarget = await waitFor(async () => (await conn.send("Target.getTargets")).targetInfos.find((t) => isPopup(t) && !before.has(t.targetId)), 8000);
  const popup = (await conn.send("Target.attachToTarget", { targetId: popupTarget.targetId, flatten: true })).sessionId;
  const shown = await waitFor(
    () => evaluate(conn, popup, `document.querySelector("#record [data-field], #message:not([hidden])") ? { w: innerWidth, h: innerHeight, status: document.getElementById("status").dataset.status, meta: document.getElementById("meta").innerText } : null`, 3000),
    20000,
  );
  // The pointer off the popup: hovering a link there shows a tooltip in the shot.
  await desk.moveTo(300, 700, 300);
  await sleep(900);
  const withPopup = join(out, `${p.name}-popup.raw.png`);
  desk.screenshot(withPopup);
  // The popup hangs from the icon at a fixed offset: measure it on the first page (a static one), reuse it
  // on pages whose ads or carousels would disturb the comparison.
  anchor ??= (() => {
    const [l, t] = popupBox(full, withPopup, icon.x, shown.w, shown.h);
    return { dx: l - icon.x, top: t };
  })();
  const box = [icon.x + anchor.dx, anchor.top, icon.x + anchor.dx + shown.w + 2, anchor.top + shown.h + 2];
  crop(withPopup, join(out, `${p.name}-popup.png`), box, 0);
  crop(withPopup, join(out, `${p.name}-screen.png`), [0, 0, 1920, 1080], 1280);
  shots.push({ name: p.name, url: p.url, status: shown.status, meta: shown.meta.split("\n")[0], popup: { w: shown.w, h: shown.h } });

  const shoot = async (label, js) => {
    await evaluate(conn, popup, js, 3000);
    await sleep(700);
    const raw = join(out, `${p.name}-${label}.raw.png`);
    desk.screenshot(raw);
    crop(raw, join(out, `${p.name}-${label}.png`), box, 0);
  };
  if (p.json) await shoot("json", `(() => { document.getElementById("json").scrollIntoView({ block: "end" }); return 1; })()`);
  if (p.table) {
    await shoot("table", `(() => { document.getElementById("viewTable").click(); scrollTo(0, 0); return 1; })()`);
    // The view is remembered: back to Reader for the next page.
    await evaluate(conn, popup, `(() => { document.getElementById("viewReader").click(); scrollTo(0, 0); return 1; })()`, 3000);
    await sleep(400);
  }
  if (p.scroll) {
    // Scroll with the pointer over the popup, then move it away for the shot.
    await desk.moveTo(box[0] + 200, box[1] + 300, 300);
    await desk.wheel(8, 1200);
    await desk.moveTo(300, 700, 200);
    await sleep(900);
    const scrolled = join(out, `${p.name}-popup-scrolled.raw.png`);
    desk.screenshot(scrolled);
    crop(scrolled, join(out, `${p.name}-popup-scrolled.png`), box, 0);
    // The trace: why each field has its value (open the "Trace & diagnostics" section).
    await evaluate(conn, popup, `(() => { const d = document.getElementById("details"); d.open = true; d.scrollIntoView(); return 1; })()`, 3000);
    await sleep(700);
    const trace = join(out, `${p.name}-trace.raw.png`);
    desk.screenshot(trace);
    crop(trace, join(out, `${p.name}-trace.png`), box, 0);
  }
  desk.key("Escape");
  await sleep(700);
  console.log(`${p.name}: ${shown.status} ${shown.meta.split("\n")[0]}`);
}

// The options page, in a tab of its own.
await open(`chrome-extension://${EXT}/options.html`);
const opts = join(out, "options.raw.png");
desk.screenshot(opts);
crop(opts, join(out, "options.png"), [0, 0, 1920, 1080], 1280);
console.log("options: done");

writeFileSync(join(out, "shots.json"), JSON.stringify({ at: new Date().toISOString(), version: await worker("chrome.runtime.getManifest().version"), shots }, null, 2));
conn.close();
