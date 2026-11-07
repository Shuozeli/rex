// Popup: extract the active tab and render the typed record (design.md §9.2, §9.5).
// When opened as a normal tab (e.g. by tests), `?tab=<id>` selects the tab to extract.

import { langOf } from "./format";
import { localize, t } from "./i18n";
import { renderJson, renderView, type View } from "./render";
import type {
  EngineOutput,
  ExtractResponse,
  Extraction,
  FixtureResponse,
  OutputFormat,
  Status,
  StatusResponse,
} from "./types";

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (el === null) {
    throw new Error(`missing #${id}`);
  }
  return el as T;
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function toast(text: string): void {
  const n = el("div", "toast", text);
  document.body.appendChild(n);
  setTimeout(() => n.remove(), 1600);
}

const lang = langOf(chrome.i18n.getUILanguage());
document.body.dataset.lang = lang;

/** The status as words; `data-status` keeps the engine's name for tests ("OK", "FAILED_EXPECTATION", …). */
function statusBadge(status: string | null): void {
  const b = $("status");
  const s = (status ?? "").replace("EXTRACTION_STATUS_", "");
  const [text, cls] =
    s === "OK"
      ? [t("statusComplete"), "ok"]
      : s === "FAILED_EXPECTATION"
        ? [t("statusCheck"), "warn"]
        : s === "EMPTY"
          ? [t("statusEmpty"), "bad"]
          : s === ""
            ? [t("statusNone"), ""]
            : [t("statusIncomplete"), "bad"];
  b.textContent = text;
  b.className = `state ${cls}`;
  b.dataset.status = s || "NONE";
  b.title = s;
}

function showMessage(text: string, kind: "info" | "error" | "warn" = "info"): void {
  const m = $("message");
  m.hidden = false;
  m.className = `message ${kind === "info" ? "" : kind}`;
  m.textContent = text;
}

// The view (reader or table) is the user's choice, kept across popups.
const VIEW_KEY = "rex.popupView";
let view: View = "reader";
let shown: { ext: Extraction; record: Record<string, unknown> } | null = null;

function showView(next: View): void {
  view = next;
  for (const b of document.querySelectorAll<HTMLButtonElement>("#views button")) {
    b.setAttribute("aria-selected", String(b.dataset["view"] === next));
  }
  if (shown) renderView($("record"), view, shown.ext, shown.record, lang);
}

for (const b of document.querySelectorAll<HTMLButtonElement>("#views button")) {
  b.addEventListener("click", () => {
    const next = b.dataset["view"] === "table" ? "table" : "reader";
    showView(next);
    void chrome.storage.local.set({ [VIEW_KEY]: next }).catch(() => {});
  });
}

let fuel = 0;

function renderTrace(out: EngineOutput): void {
  const box = $("trace");
  box.replaceChildren();
  if (out.plan) {
    const p = out.plan;
    box.appendChild(
      el(
        "div",
        "trace-field",
        `plan · url hits [${p.urlHits.map((h) => h.id).join(", ")}] · confirmed [${p.confirmed.join(", ")}] · types [${p.detectedTypes.join(", ")}] · fallback ${p.fallback ?? "—"}`,
      ),
    );
  }
  for (const tr of out.trace) {
    const d = el("div", "trace-field");
    d.appendChild(el("div", "p", `${tr.path} = ${tr.value.length ? tr.value.join(" | ").slice(0, 120) : "(missing)"}`));
    for (const a of tr.attempts) {
      const line = el("div", `a ${a.outcome === "won" ? "won" : ""}`, `[${a.candidate}] ${a.source} → ${a.outcome}`);
      if (a.raw.length > 0) line.title = a.raw.join("\n").slice(0, 2000);
      d.appendChild(line);
    }
    box.appendChild(d);
  }
  for (const dg of out.diagnostics) {
    box.appendChild(el("div", "trace-field", `${dg.level} ${dg.code} ${dg.path} — ${dg.detail}`));
  }
  fuel = out.fuelUsed;
}

function renderFooter(status: Status | null): void {
  if (status === null) {
    $("bundle").textContent = "";
    return;
  }
  const synced = status.sources.filter((s) => s.enabled && s.sequence !== null);
  const parts = [`ext ${status.extensionVersion}`, `engine ${status.engineVersion ?? "?"}`];
  const s = synced[0] ?? status.sources.find((x) => x.enabled);
  if (s) {
    parts.unshift(
      s.sequence ? t("bundleInfo", s.id, s.sequence, String(s.recipeCount)) : `${t("notSynced", s.id)}${s.lastError ? ` — ${s.lastError}` : ""}`,
    );
  }
  if (synced.length > 1) parts.splice(1, 0, `+${synced.length - 1}`);
  if (fuel) parts.push(`fuel ${fuel}`);
  $("bundle").textContent = parts.join(" · ");
}

function download(name: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const a = el("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

const stamp = (): string => new Date().toISOString().replace(/[-:]/g, "").slice(0, 15);
const fileSafe = (s: string): string => s.replace(/[^\w.-]+/g, "_");

function reportUrl(ext: Extraction, includeUrl: boolean): string {
  const lines = [
    `Recipe: ${ext.recipe?.id ?? "?"}@${ext.recipe?.version ?? "?"}`,
    `Status: ${ext.status}`,
    `Missing required: ${(ext.missingRequired ?? []).join(", ") || "—"}`,
    `Expectations failed: ${(ext.expectationsFailed ?? []).join(", ") || "—"}`,
    `Versions: ${ext.extractorVersion}`,
    includeUrl ? `Page: ${ext.url}` : "Page: (not shared)",
  ];
  const q = new URLSearchParams({ title: `Broken recipe: ${ext.recipe?.id ?? "unknown"}`, body: lines.join("\n") });
  return `https://github.com/${__REX_CONFIG__.reportRepo}/issues/new?${q.toString()}`;
}

async function targetTab(): Promise<number> {
  const param = new URLSearchParams(location.search).get("tab");
  if (param !== null) {
    document.body.classList.add("tab");
    return Number(param);
  }
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab?.id === undefined) throw new Error("no active tab");
  return tab.id;
}

function copyButtons(format: OutputFormat, ext: Extraction, record: Record<string, unknown> | null): void {
  const envelope = JSON.stringify(ext, null, 2);
  const recordJson = JSON.stringify(record, null, 2);
  // The JSON panel shows exactly what Copy JSON copies.
  $("json").hidden = false;
  $("jsonNote").textContent = `· ${format === "record" ? t("jsonRecord") : t("jsonEnvelope")}`;
  renderJson($("jsonView"), format === "record" ? recordJson : envelope);
  const copyEnvelope = () => void navigator.clipboard.writeText(envelope).then(() => toast(t("copiedEnvelope")));
  const copyRecord = () => void navigator.clipboard.writeText(recordJson).then(() => toast(t("copiedRecord")));
  switch (format) {
    case "envelope":
      $("copy").onclick = copyEnvelope;
      $("copyOther").textContent = t("copyRecord");
      $("copyOther").onclick = copyRecord;
      break;
    case "record":
      $("copy").onclick = copyRecord;
      $("copyOther").textContent = t("copyEnvelope");
      $("copyOther").onclick = copyEnvelope;
      break;
    default: {
      const never: never = format;
      throw new Error(`unknown output format ${String(never)}`);
    }
  }
  $("download").onclick = () => {
    const text = format === "record" ? recordJson : envelope;
    download(`${fileSafe(ext.recipe?.id ?? "rex")}-${stamp()}.json`, text);
  };
}

async function saveFixture(tabId: number, recipeId: string | null): Promise<void> {
  const r = (await chrome.runtime.sendMessage({ type: "fixture", tabId, recipeId })) as FixtureResponse;
  if (!r.ok) {
    showMessage(r.error, "error");
    return;
  }
  download(`${fileSafe(recipeId ?? "page")}-${stamp()}.rexfixture.json`, JSON.stringify(r.fixture, null, 2));
  toast(t("fixtureSaved"));
}

async function run(recipeId?: string): Promise<void> {
  const tabId = await targetTab();
  $("message").hidden = true;
  const resp = (await chrome.runtime.sendMessage({ type: "extract", tabId, recipeId })) as ExtractResponse;
  renderFooter(resp.status);
  if (!resp.ok) {
    statusBadge(null);
    showMessage(resp.error, "error");
    return;
  }
  const out = resp.output;
  const plan = out.plan;
  const ext = out.extraction;
  const dev = resp.status.devMode;
  $("details").hidden = false;
  ($("details") as HTMLDetailsElement).open = dev;
  if (ext === null) {
    statusBadge(null);
    const types = plan?.detectedTypes.length ? plan.detectedTypes.join(", ") : t("none");
    showMessage(t("noMatch", types), "warn");
    renderTrace(out);
    return;
  }
  statusBadge(ext.status);
  const meta = $("meta");
  meta.replaceChildren();
  meta.dataset.recipe = `${ext.recipe?.id ?? "?"}@${ext.recipe?.version ?? "?"}`;
  const viaFallback = plan?.fallback !== null && plan?.fallback === ext.recipe?.id;
  meta.append(
    el("span", "", `${t("recipeLabel")} `),
    el("b", "", `${ext.recipe?.id ?? "?"}@${ext.recipe?.version ?? "?"}`),
    el("span", "", viaFallback ? ` ${t("genericDetected", plan?.detectedTypes[0] ?? "?")}` : ""),
    el("span", "", " → "),
    el("code", "", ext.schema.replace(/^rex\.schema\./, "")),
  );
  if (resp.local) meta.appendChild(el("span", "badge local", t("localUnsigned")));
  const alts = $("alts");
  alts.replaceChildren();
  const choices = [...new Set([...(plan?.confirmed ?? []), ...(plan?.fallback ? [plan.fallback] : [])])];
  alts.hidden = choices.length <= 1;
  if (choices.length > 1) {
    alts.appendChild(el("span", "", t("alsoMatches")));
    for (const id of choices) {
      const b = el("button", id === ext.recipe?.id ? "on" : "", id);
      b.addEventListener("click", () => void run(id));
      alts.appendChild(b);
    }
  }
  if (ext.status === "EXTRACTION_STATUS_FAILED_EXPECTATION") {
    showMessage(t("siteChanged", (ext.expectationsFailed ?? []).join(", ")), "warn");
  } else if (ext.status !== "EXTRACTION_STATUS_OK") {
    showMessage(t("incomplete", (ext.missingRequired ?? []).join(", ") || t("primaryContent")), "warn");
  }
  shown = { ext, record: (out.record ?? ext.record) as Record<string, unknown> };
  renderView($("record"), view, shown.ext, shown.record, lang);
  renderTrace(out);
  renderFooter(resp.status);
  $("actions").hidden = false;
  const broken = ext.status !== "EXTRACTION_STATUS_OK";
  $("report").hidden = !broken;
  $("reportUrlRow").hidden = !broken;
  copyButtons(resp.status.outputFormat, ext, out.record);
  $("fixture").hidden = !dev;
  $("fixture").onclick = () => void saveFixture(tabId, ext.recipe?.id ?? null);
  $("report").onclick = () => {
    const include = ($("reportUrl") as HTMLInputElement).checked;
    void chrome.tabs.create({ url: reportUrl(ext, include) });
  };
}

localize();
showView(view);

$("refresh").addEventListener("click", (e) => {
  e.preventDefault();
  $("bundle").textContent = t("refreshing");
  void (chrome.runtime.sendMessage({ type: "refresh" }) as Promise<StatusResponse>).then((r) => {
    renderFooter(r.status);
    if (!r.ok) showMessage(r.error, "error");
    else void run();
  });
});

void chrome.storage.local
  .get(VIEW_KEY)
  .then((got) => showView(got[VIEW_KEY] === "table" ? "table" : "reader"))
  .catch(() => {})
  .then(() => run())
  .catch((e: unknown) => showMessage(e instanceof Error ? e.message : String(e), "error"));
