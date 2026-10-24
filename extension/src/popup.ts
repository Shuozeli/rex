// Popup: extract the active tab and render the typed record (design.md §9.2, §9.5).
// When opened as a normal tab (e.g. by tests), `?tab=<id>` selects the tab to extract.

import { localize, t } from "./i18n";
import type {
  EngineOutput,
  ExtractResponse,
  Extraction,
  FieldSource,
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

const snake = (camel: string): string => camel.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
const label = (key: string): string => snake(key).replace(/_/g, " ");

function statusBadge(status: string | null): void {
  const b = $("status");
  const s = (status ?? "").replace("EXTRACTION_STATUS_", "");
  b.textContent = s === "" ? "—" : s;
  b.className = "badge " + (s === "OK" ? "ok" : s === "FAILED_EXPECTATION" ? "warn" : s === "" ? "" : "bad");
}

function showMessage(text: string, kind: "info" | "error" | "warn" = "info"): void {
  const m = $("message");
  m.hidden = false;
  m.className = `message ${kind === "info" ? "" : kind}`;
  m.textContent = text;
}

function originChip(origins: Map<string, FieldSource>, key: string): HTMLElement | null {
  const src = origins.get(snake(key));
  if (!src) return null;
  const o = src.origin.replace("FIELD_ORIGIN_", "");
  const chip = el("span", `origin o-${o}`, o);
  chip.title = `${src.recipe}${src.candidate !== undefined ? ` · candidate ${src.candidate}` : ""}`;
  return chip;
}

function textOf(v: unknown): string {
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    for (const k of ["name", "title", "url", "id"]) {
      if (typeof o[k] === "string") return o[k] as string;
    }
  }
  return JSON.stringify(v);
}

function renderValue(key: string, value: unknown): HTMLElement {
  const lk = key.toLowerCase();
  if (typeof value === "string") {
    if (lk === "title") return el("div", "v title", value);
    if (lk === "bodytext" || lk === "bodyhtml" || value.length > 280) {
      return el("div", "v body", lk === "bodyhtml" ? `${value.length} chars of sanitized HTML` : value);
    }
    if (lk === "image" && /^https?:/.test(value)) {
      const wrap = el("div", "v");
      const img = el("img");
      img.src = value;
      img.alt = "";
      img.referrerPolicy = "no-referrer";
      img.addEventListener("error", () => img.remove());
      const link = el("a", "", value);
      link.href = value;
      link.target = "_blank";
      wrap.append(img, link);
      return wrap;
    }
    if (/^https?:\/\//.test(value)) {
      const a = el("a", "v", value);
      a.href = value;
      a.target = "_blank";
      return a;
    }
    return el("div", "v", value);
  }
  if (Array.isArray(value)) {
    if (value.every((x) => typeof x === "string")) {
      const chips = el("div", "v chips");
      for (const s of value as string[]) chips.appendChild(el("span", "chip", s));
      return chips;
    }
    const list = el("ol", "v items");
    for (const item of value.slice(0, 50)) {
      const li = el("li");
      const o = (item ?? {}) as Record<string, unknown>;
      const title = textOf(item);
      if (typeof o["url"] === "string" && title !== o["url"]) {
        const a = el("a", "", title);
        a.href = o["url"] as string;
        a.target = "_blank";
        li.appendChild(a);
      } else {
        li.appendChild(document.createTextNode(title));
      }
      const extras = Object.entries(o)
        .filter(([k]) => !["title", "name", "url", "id"].includes(k))
        .map(([k, v]) => `${label(k)}: ${textOf(v)}`);
      if (extras.length > 0) {
        li.appendChild(el("div", "sub", extras.join(" · ")));
      }
      list.appendChild(li);
    }
    if (value.length > 50) list.appendChild(el("li", "sub", `… ${value.length - 50} more`));
    return list;
  }
  return el("div", "v", textOf(value));
}

function renderRecord(ext: Extraction): void {
  const box = $("record");
  box.replaceChildren();
  const origins = new Map((ext.sources ?? []).map((s) => [s.path, s] as const));
  for (const [key, value] of Object.entries(ext.record)) {
    if (key === "@type") continue;
    const f = el("div", "field");
    const k = el("div", "k", label(key));
    const chip = originChip(origins, key);
    if (chip) k.appendChild(chip);
    if (Array.isArray(value)) k.appendChild(el("span", "", `(${value.length})`));
    f.append(k, renderValue(key, value));
    box.appendChild(f);
  }
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
  renderRecord(ext);
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

$("refresh").addEventListener("click", (e) => {
  e.preventDefault();
  $("bundle").textContent = t("refreshing");
  void (chrome.runtime.sendMessage({ type: "refresh" }) as Promise<StatusResponse>).then((r) => {
    renderFooter(r.status);
    if (!r.ok) showMessage(r.error, "error");
    else void run();
  });
});

void run().catch((e: unknown) => showMessage(e instanceof Error ? e.message : String(e), "error"));
