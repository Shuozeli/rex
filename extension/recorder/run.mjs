// Records the extension on real pages in the recorder desktop (docker/), as a tutorial video and an e2e test.
//
// Each scene is the flow a user goes through, with real input: type the URL into the address bar, scroll
// the page, move the pointer to the rex icon and click it, wait for the popup's result, scroll the popup.
// CDP only observes (load state, the popup's result, the extraction behind it). ffmpeg records the whole
// screen at 50 fps (a frame every 20 ms) per scene; captions are burned in from the logged events, and the
// scenes are joined between an intro and a results card.
//
// Run inside the container:
//   docker compose -f extension/recorder/docker/compose.yaml exec desktop \
//     node /repo/extension/recorder/run.mjs /repo/extension/recorder/scenarios/tutorial.json --out /out/tutorial
// Several scenario files run one after the other, each into <out>/<scenario name>/.
// Options: --only a,b (scene names), --no-video (no captions or joined video for any scenario).
// Scenario keys: "flow": "tutorial" (default; scroll the page and the popup, captions paced for viewing) or
// "quick" (open, click, read, close: for many scenes); "video": false (checks only; the raw clips of passing
// scenes are deleted, failing ones are kept). Exit code 1 when a scene does not meet its `expect`.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { attachWorker, connect, evaluate, sleep } from "./lib/cdp.mjs";
import * as desk from "./lib/desktop.mjs";
import { selfCheck, startAudit, violations } from "./lib/netaudit.mjs";
import { loadScenario } from "./lib/scenario.mjs";
import { assemble, cardAss, sceneAss } from "./lib/video.mjs";

const args = process.argv.slice(2);
const opt = (n, d) => (args.indexOf(n) < 0 ? d : args[args.indexOf(n) + 1]);
const scenarioFiles = args.filter((a) => a.endsWith(".json"));
if (scenarioFiles.length === 0) throw new Error("usage: run.mjs <scenario.json>... [--out dir] [--only a,b] [--no-video]");
const outBase = opt("--out", "/out/recording");
const only = opt("--only", "").split(",").filter(Boolean);
const noVideo = args.includes("--no-video");
const CDP = process.env.REX_CDP ?? "http://127.0.0.1:9222";
const EXT = readFileSync("/tmp/rex-ext-id", "utf8").trim();
const REPO = process.env.REX_REPO ?? "/repo";
const ICON = "/ext/icons/icon-16.png";

async function waitFor(fn, ms, every = 100) {
  const started = Date.now();
  while (Date.now() - started < ms) {
    const v = await fn().catch(() => null);
    if (v) return v;
    await sleep(every);
  }
  return null;
}

/** Restarts Chrome (the container's loop starts it again) and waits for CDP and the extension worker. */
async function freshChrome() {
  // SIGTERM first: Chrome shuts down cleanly and does not offer to restore pages next time.
  try {
    execFileSync("pkill", ["-TERM", "-f", "/opt/google/chrome/chrome"]);
  } catch {}
  await waitFor(async () => {
    try {
      execFileSync("pgrep", ["-f", "/opt/google/chrome/chrome --user-data-dir"]);
      return false;
    } catch {
      return true;
    }
  }, 10000, 200);
  await sleep(1500);
  const up = await waitFor(async () => (await fetch(`${CDP}/json/version`)).ok, 30000, 300);
  if (!up) throw new Error("Chrome did not come back");
  await sleep(1500);
}

const isPage = (t) => t.type === "page" && !t.url.startsWith("chrome-extension://") && !t.url.startsWith("devtools://");
const isPopup = (t) => t.type === "page" && t.url.startsWith(`chrome-extension://${EXT}/popup.html`);

function check(expect, r) {
  const problems = [];
  if (!expect) return problems;
  if (expect.recipe && !(r.matched ?? "").startsWith(expect.recipe)) problems.push(`recipe ${r.matched} (want ${expect.recipe})`);
  if (r.mismatch) problems.push(r.mismatch);
  const status = expect.status ?? "OK";
  if (r.status !== status) problems.push(`status ${r.status} (want ${status})`);
  if (expect.minFields && r.fields < expect.minFields) problems.push(`${r.fields} fields (want >= ${expect.minFields})`);
  if (expect.fields !== undefined && r.fields !== expect.fields) problems.push(`${r.fields} fields (want ${expect.fields})`);
  return problems;
}

async function runScene(scenario, out, scene, index) {
  const quick = scenario.flow === "quick";
  const r = { name: scene.name, label: scene.label, url: scene.url };
  if (scenario.restartBetweenScenes !== false || index === 0) await freshChrome();
  const conn = await connect(CDP);
  const events = [];
  let rec = null;
  let audit = null;
  try {
    const sw = await attachWorker(conn, EXT);
    const worker = (expr, ms) => evaluate(conn, sw, expr, ms);
    // The extension may not make a single network request from here on (decisions.md U45).
    audit = await startAudit(conn, EXT, sw);
    // The e2e build exposes rexDebug; the store build does not, and is driven through its UI only.
    r.build = (await worker("typeof globalThis.rexDebug")) === "object" ? "e2e" : "store";
    // The recipes come with the package.
    r.bundle = await worker(
      `(async () => (await (await fetch(chrome.runtime.getURL("recipes/index.json"))).json()).entries.length + " packaged recipes")()`,
    );

    const pageTarget = (await conn.send("Target.getTargets")).targetInfos.find(isPage);
    if (!pageTarget) throw new Error("no browser tab");
    await conn.send("Target.activateTarget", { targetId: pageTarget.targetId });
    const page = (await conn.send("Target.attachToTarget", { targetId: pageTarget.targetId, flatten: true })).sessionId;
    const pageEval = (expr) => evaluate(conn, page, expr, 5000);

    rec = desk.startRecording(join(out, `${scene.name}.raw.mp4`));
    await rec.ready;
    const ev = (kind, data = {}) => events.push({ t: Date.now() - rec.t0, kind, ...data });
    await desk.moveTo(960, 600, 200);
    await sleep(600);

    // 1. Open the page from the address bar.
    ev("caption", { text: `第 1 步：在地址栏输入网址，打开${scene.label}` });
    desk.key("ctrl+l");
    await sleep(300);
    desk.type(scene.url);
    await sleep(300);
    desk.key("Return");
    await waitFor(async () => (await pageEval("location.href")) !== "about:blank" && (await pageEval("document.readyState")) === "complete", 30000, 250);
    await sleep(scene.settleMs ?? (quick ? 400 : 2500));

    // 2. Look around the page.
    if (!quick) {
      ev("caption", { text: "浏览页面：rex 只读取你打开的这一页，不会另外请求网站" });
      await desk.moveTo(960, 620, 500);
      await desk.wheel(8, 1600);
      await sleep(800);
      await desk.wheel(-8, 1200);
      await sleep(600);
    }

    // 3. Click the rex icon on the toolbar (real pointer, real click).
    ev("caption", { text: "第 2 步：点击工具栏上的 rex 图标" });
    const icon = desk.locateIcon(ICON);
    r.icon = icon;
    if (icon.score > 25) throw new Error(`rex icon not found on the toolbar (best match ${JSON.stringify(icon)})`);
    const before = new Set((await conn.send("Target.getTargets")).targetInfos.filter(isPopup).map((t) => t.targetId));
    const clicked = Date.now();
    await desk.click(icon.x, icon.y, quick ? 300 : 900);
    ev("click", { x: icon.x, y: icon.y });
    const popupTarget = await waitFor(async () => (await conn.send("Target.getTargets")).targetInfos.find((t) => isPopup(t) && !before.has(t.targetId)), 8000);
    if (!popupTarget) throw new Error("the popup did not open");
    const popup = (await conn.send("Target.attachToTarget", { targetId: popupTarget.targetId, flatten: true })).sessionId;
    ev("caption", { text: "第 3 步：rex 用这个网站的 recipe 把页面变成结构化数据" });
    // The result as the popup shows it: #status[data-status], #meta[data-recipe] ("<id>@<version>"), one
    // [data-field] element per top-level field (both views mark them).
    const shown = await waitFor(
      () =>
        evaluate(
          conn,
          popup,
          `(() => {
            const msg = document.querySelector("#message:not([hidden])");
            const fields = document.querySelectorAll("#record [data-field]").length;
            if (!fields && !msg) return null;
            const recipe = document.getElementById("meta")?.dataset.recipe ?? null;
            return { status: document.getElementById("status").dataset.status ?? "", recipe,
                     fields, message: msg?.innerText ?? null, text: document.body.innerText.slice(0, 600) };
          })()`,
          5000,
        ),
      20000,
    );
    r.popupMs = Date.now() - clicked;
    r.popupText = shown?.text ?? null;
    r.matched = shown?.recipe ?? null;
    r.status = !shown ? "ERROR" : shown.recipe ? shown.status : "NO_MATCH";
    r.fields = shown?.fields ?? 0;
    if (!shown) r.error = "the popup showed no result";
    else if (shown.message && !shown.recipe) r.message = shown.message;

    // e2e build: the extraction behind the popup must agree with what the popup shows.
    if (r.build === "e2e" && shown) {
      const tabId = await worker(`(async () => (await chrome.tabs.query({ active: true, lastFocusedWindow: true }))[0]?.id)()`);
      const x = await worker(`globalThis.rexDebug.extractTab(${tabId})`, 60000).catch((e) => ({ ok: false, error: String(e) }));
      const ext = x.output?.extraction;
      const behind = ext?.recipe ? `${ext.recipe.id}@${ext.recipe.version}` : null;
      if (behind !== r.matched) r.mismatch = `popup shows ${r.matched}, the extraction is ${behind}`;
    }
    // Once per scenario: prove the audit sees the worker and the popup, and the CSP blocks fetches and images.
    if (index === 0) {
      r.auditSelfCheck = await selfCheck(audit, worker, (expr) => evaluate(conn, popup, expr, 5000));
    }
    r.finalUrl = await pageEval("location.href").catch(() => null);
    ev("caption", { text: r.matched ? `结果：recipe ${r.matched} · 状态 ${r.status} · ${r.fields} 个字段` : `结果：没有可用的 recipe（${r.status}）` });
    await sleep(quick ? 500 : 2200);

    // 4. Scroll through the fields in the popup.
    if (!quick) {
      await desk.moveTo(icon.x - 200, 420, 500);
      await desk.wheel(6, 2000);
      await sleep(900);
      await desk.wheel(6, 1600);
      await sleep(1500);
    }
    desk.key("Escape");
    await sleep(600);
    ev("end");
  } catch (e) {
    r.status = r.status ?? "ERROR";
    r.error = String(e.message ?? e);
    if (rec?.t0) events.push({ t: Date.now() - rec.t0, kind: "caption", text: `出错：${r.error.slice(0, 50)}` });
    await sleep(2000);
    if (rec?.t0) events.push({ t: Date.now() - rec.t0, kind: "end" });
  } finally {
    if (rec) await rec.stop();
    await sleep(300); // requests still in flight when the scene ends
    conn.close();
  }
  if (audit) {
    r.network = { requests: audit.requests.map(({ url, type, blocked }) => `${type} ${url}${blocked ? ` (blocked: ${blocked})` : ""}`) };
    r.networkViolations = violations(audit);
  }
  r.problems = [
    ...(r.error ? [r.error] : []),
    ...check(scene.expect, r),
    ...(r.networkViolations ?? []).map((v) => `network: ${v}`),
    ...(r.auditSelfCheck ?? []),
  ];
  r.pass = r.problems.length === 0;
  writeFileSync(join(out, `${scene.name}.events.json`), JSON.stringify(events, null, 1));
  const video = !noVideo && scenario.video !== false;
  if (video && rec) writeFileSync(join(out, `${scene.name}.ass`), sceneAss(events));
  // Checks only: keep the clip of a failing scene for debugging, drop the rest.
  if (!video && rec && r.pass && scenario.video === false) rmSync(rec.file, { force: true });
  return r;
}

async function runScenario(file) {
  const scenario = loadScenario(file, REPO);
  const name = scenario.name ?? basename(file, ".json");
  const out = scenarioFiles.length > 1 ? join(outBase, name) : outBase;
  mkdirSync(out, { recursive: true });
  const scenes = scenario.scenes.filter((s) => only.length === 0 || only.includes(s.name));
  console.log(`== ${name}: ${scenes.length} scene(s)${scenario.flow === "quick" ? ", quick flow" : ""}`);

  const results = [];
  for (const [i, scene] of scenes.entries()) {
    const r = await runScene(scenario, out, scene, i);
    results.push(r);
    const net = r.network ? `, ${r.network.requests.length} extension request(s)` : "";
    console.log(`${r.pass ? "ok  " : "FAIL"} ${scene.name.padEnd(12)} ${r.matched ?? "-"} ${r.status} ${r.fields ?? 0} fields${r.popupMs ? `, popup ${r.popupMs} ms` : ""}${net}${r.pass ? "" : ` — ${r.problems.join("; ")}`}`);
  }
  writeFileSync(join(out, "results.json"), JSON.stringify({ scenario: scenario.title, at: new Date().toISOString(), results }, null, 2));
  if (!noVideo && scenario.video !== false) {
    writeFileSync(join(out, "intro.ass"), cardAss([{ text: scenario.title, size: 64, bold: true }, ...(scenario.subtitle ?? []).map((s) => ({ text: s, size: 32 }))]));
    const rows = results.map((r) => `${r.label}    ${r.matched ?? "（无匹配）"}    ${r.status}    ${r.fields ?? 0} 个字段`);
    writeFileSync(join(out, "outro.ass"), cardAss([{ text: "这次测试的结果", size: 56, bold: true }, ...rows.map((t, i) => ({ text: t, size: 32, ok: results[i].pass }))]));
    const final = join(out, `${name}.mp4`);
    assemble({ dir: out, scenes: results.map((r) => r.name), file: final });
    console.log(`video: ${final}`);
  }
  const failed = results.filter((r) => !r.pass);
  console.log(`${name}: ${results.length - failed.length}/${results.length} scenes passed`);
  return failed.length;
}

let failed = 0;
for (const f of scenarioFiles) failed += await runScenario(f);
process.exit(failed === 0 ? 0 : 1);
