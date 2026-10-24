// Options page (FR-EX-9, FR-EX-10): edit sources (order, enable, pinned keys, host permission), refresh
// interval, copy format, and developer mode with an unsigned local recipe.

import { localize, t } from "./i18n";
import { formatKeys, originPattern, parseKeys, validateSettings } from "./settings";
import type { OutputFormat, Settings, SettingsResponse, SourceStatus, Status, StatusResponse, ValidateResponse } from "./types";

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (el === null) throw new Error(`missing #${id}`);
  return el as T;
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

let draft: Settings | null = null;
let statuses: SourceStatus[] = [];

function current(): Settings {
  if (draft === null) throw new Error("settings not loaded");
  return draft;
}

function statusText(s: SourceStatus | undefined): { text: string; error: boolean } {
  if (s === undefined) return { text: "", error: false };
  const lines: string[] = [];
  if (!s.hasPermission) lines.push(t("statusNoPermission"));
  lines.push(s.sequence ? t("bundleShort", s.sequence, String(s.recipeCount)) : t("statusNever"));
  if (s.skipped.length > 0) lines.push(`${t("skippedN", String(s.skipped.length))}: ${s.skipped.map((x) => `${x.id} (${x.reason})`).join("; ")}`);
  if (s.lastSync) lines.push(new Date(s.lastSync).toLocaleString());
  lines.push(s.lastError ? s.lastError : t("statusOk"));
  return { text: lines.join("\n"), error: s.lastError !== null || !s.hasPermission };
}

function renderSources(): void {
  const settings = current();
  const body = document.querySelector("#sources tbody");
  if (body === null) throw new Error("missing #sources tbody");
  body.replaceChildren();
  settings.sources.forEach((src, i) => {
    const tr = el("tr");
    tr.dataset["index"] = String(i);

    const on = el("input");
    on.type = "checkbox";
    on.checked = src.enabled;
    on.className = "src-enabled";
    on.addEventListener("change", () => (src.enabled = on.checked));
    const tdOn = el("td");
    tdOn.appendChild(on);

    const id = el("input", "src-id");
    id.type = "text";
    id.value = src.id;
    id.addEventListener("input", () => (src.id = id.value.trim()));
    const url = el("input", "src-url");
    url.type = "url";
    url.value = src.baseUrl;
    url.addEventListener("input", () => (src.baseUrl = url.value.trim()));
    const channel = el("select", "src-channel");
    for (const c of new Set(["stable", "dev", src.channel])) {
      const o = el("option", "", c);
      o.value = c;
      channel.appendChild(o);
    }
    channel.value = src.channel;
    channel.addEventListener("change", () => (src.channel = channel.value));
    const tdSrc = el("td");
    tdSrc.append(id, url, channel);

    const keys = el("textarea", "src-keys");
    keys.rows = 2;
    keys.spellcheck = false;
    keys.value = formatKeys(src.keys);
    keys.addEventListener("input", () => (src.keys = parseKeys(keys.value)));
    const tdKeys = el("td");
    tdKeys.appendChild(keys);

    const st = statusText(statuses.find((s) => s.id === src.id));
    const tdSt = el("td");
    tdSt.appendChild(el("div", `st ${st.error ? "err" : ""}`, st.text));
    const actions = el("div", "row-actions");
    const move = (delta: number) => {
      const j = i + delta;
      if (j < 0 || j >= settings.sources.length) return;
      const [moved] = settings.sources.splice(i, 1);
      if (moved !== undefined) settings.sources.splice(j, 0, moved);
      renderSources();
    };
    const up = el("button", "", "↑");
    up.title = t("moveUp");
    up.disabled = i === 0;
    up.addEventListener("click", () => move(-1));
    const down = el("button", "", "↓");
    down.title = t("moveDown");
    down.disabled = i === settings.sources.length - 1;
    down.addEventListener("click", () => move(1));
    const rm = el("button", "", t("remove"));
    rm.addEventListener("click", () => {
      settings.sources.splice(i, 1);
      renderSources();
    });
    actions.append(up, down, rm);
    tdSt.appendChild(actions);

    tr.append(tdOn, tdSrc, tdKeys, tdSt);
    body.appendChild(tr);
  });
}

function render(settings: Settings, status: Status | null): void {
  draft = structuredClone(settings);
  statuses = status?.sources ?? [];
  renderSources();
  $<HTMLInputElement>("refreshHours").value = String(Math.round(settings.refreshMinutes / 60));
  for (const r of document.querySelectorAll<HTMLInputElement>("input[name=format]")) {
    r.checked = r.value === settings.outputFormat;
  }
  $<HTMLInputElement>("devMode").checked = settings.devMode;
  $("devBox").hidden = !settings.devMode;
  $<HTMLTextAreaElement>("localRecipe").value = settings.localRecipe;
  $<HTMLInputElement>("localEnabled").checked = settings.localRecipeEnabled;
  const vr = $("validateResult");
  if (status?.localRecipeError) vr.textContent = status.localRecipeError;
  else if (status?.localRecipeId) vr.textContent = t("localValid", status.localRecipeId);
  else vr.textContent = "";
}

function collect(): Settings {
  const s = current();
  const format = document.querySelector<HTMLInputElement>("input[name=format]:checked")?.value ?? "envelope";
  return {
    ...s,
    refreshMinutes: Number($<HTMLInputElement>("refreshHours").value) * 60,
    outputFormat: format as OutputFormat,
    devMode: $<HTMLInputElement>("devMode").checked,
    localRecipe: $<HTMLTextAreaElement>("localRecipe").value,
    localRecipeEnabled: $<HTMLInputElement>("localEnabled").checked,
  };
}

function showErrors(errors: string[]): void {
  const box = $("errors");
  box.hidden = errors.length === 0;
  box.textContent = errors.join("\n");
}

async function save(): Promise<void> {
  const { settings, errors } = validateSettings(collect());
  if (errors.length > 0) {
    showErrors(errors);
    return;
  }
  showErrors([]);
  // Must run inside the click's user activation, before any other await.
  const origins = [...new Set(settings.sources.filter((s) => s.enabled).map((s) => originPattern(s.baseUrl)))];
  const granted = await chrome.permissions.request({ origins });
  if (!granted) {
    showErrors([t("permissionDenied", origins.join(", "))]);
  }
  $("result").textContent = t("refreshing");
  const r = (await chrome.runtime.sendMessage({ type: "saveSettings", settings })) as SettingsResponse;
  if (!r.ok) {
    $("result").textContent = "";
    showErrors([r.error]);
    return;
  }
  render(r.settings, r.status);
  const failed = r.status.sources.filter((s) => s.enabled && s.lastError !== null);
  $("result").textContent = failed.length === 0 ? t("saved") : failed.map((s) => `${s.id}: ${s.lastError ?? ""}`).join("; ");
}

async function load(): Promise<void> {
  const r = (await chrome.runtime.sendMessage({ type: "getSettings" })) as SettingsResponse;
  if (!r.ok) {
    showErrors([r.error]);
    return;
  }
  render(r.settings, r.status);
}

localize();

$("addSource").addEventListener("click", () => {
  const s = current();
  s.sources.push({ id: `source-${s.sources.length + 1}`, baseUrl: "https://", channel: "stable", keys: [], enabled: true });
  renderSources();
});

$("devMode").addEventListener("change", () => {
  $("devBox").hidden = !$<HTMLInputElement>("devMode").checked;
});

$("validate").addEventListener("click", () => {
  const text = $<HTMLTextAreaElement>("localRecipe").value;
  void (chrome.runtime.sendMessage({ type: "validateRecipe", text }) as Promise<ValidateResponse>).then((r) => {
    $("validateResult").textContent = r.ok ? t("localValid", r.id) : r.error;
  });
});

$("save").addEventListener("click", () => void save().catch((e: unknown) => showErrors([e instanceof Error ? e.message : String(e)])));

$("refresh").addEventListener("click", () => {
  $("result").textContent = t("refreshing");
  void (chrome.runtime.sendMessage({ type: "refresh" }) as Promise<StatusResponse>).then(async (r) => {
    $("result").textContent = r.ok ? t("upToDate") : r.error;
    statuses = r.status?.sources ?? statuses;
    renderSources();
  });
});

void load();
