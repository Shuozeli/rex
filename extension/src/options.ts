// Options page (FR-EX-10): copy format, developer mode with an unsigned local recipe, and what the packaged
// recipes are (decisions.md U45: they ship with the extension; there is nothing to configure or refresh).

import { localize, t } from "./i18n";
import { validateSettings } from "./settings";
import type { OutputFormat, Settings, SettingsResponse, Status, ValidateResponse } from "./types";

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (el === null) throw new Error(`missing #${id}`);
  return el as T;
};

let draft: Settings | null = null;

function current(): Settings {
  if (draft === null) throw new Error("settings not loaded");
  return draft;
}

function renderRecipes(status: Status | null): void {
  const r = status?.recipes;
  const box = $("recipes");
  if (r === null || r === undefined) {
    box.textContent = t("errNoRecipes");
    box.classList.add("err");
    return;
  }
  const lines = [r.generatedAt ? t("recipesBuilt", String(r.count), new Date(r.generatedAt).toLocaleDateString()) : t("recipesInfo", String(r.count))];
  if (r.skipped.length > 0) lines.push(`${t("skippedN", String(r.skipped.length))}: ${r.skipped.map((x) => `${x.id} (${x.reason})`).join("; ")}`);
  box.textContent = lines.join("\n");
  box.classList.remove("err");
}

function render(settings: Settings, status: Status | null): void {
  draft = structuredClone(settings);
  renderRecipes(status);
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
  const r = (await chrome.runtime.sendMessage({ type: "saveSettings", settings })) as SettingsResponse;
  if (!r.ok) {
    $("result").textContent = "";
    showErrors([r.error]);
    return;
  }
  render(r.settings, r.status);
  $("result").textContent = t("saved");
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

void load();
