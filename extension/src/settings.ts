// User settings (FR-EX-10): defaults, validation, and storage. Shared by the options page and the service worker.

import type { OutputFormat, Settings } from "./types";

const KEY = "rex.settings";

export function defaultSettings(): Settings {
  return {
    outputFormat: "envelope",
    devMode: false,
    localRecipe: "",
    localRecipeEnabled: false,
  };
}

/** Stored settings over the defaults; keys of older versions (recipe sources, refresh interval) are dropped. */
export async function loadSettings(): Promise<Settings> {
  const got = await chrome.storage.local.get(KEY);
  const stored = (got[KEY] ?? {}) as Partial<Settings>;
  const d = defaultSettings();
  return {
    outputFormat: stored.outputFormat ?? d.outputFormat,
    devMode: stored.devMode ?? d.devMode,
    localRecipe: stored.localRecipe ?? d.localRecipe,
    localRecipeEnabled: stored.localRecipeEnabled ?? d.localRecipeEnabled,
  };
}

export async function storeSettings(settings: Settings): Promise<void> {
  await chrome.storage.local.set({ [KEY]: settings });
}

const FORMATS: OutputFormat[] = ["envelope", "record"];

/** Every problem found (empty when valid). */
export function validateSettings(s: Settings): { settings: Settings; errors: string[] } {
  const errors: string[] = [];
  if (!FORMATS.includes(s.outputFormat)) errors.push(`unknown output format "${s.outputFormat}"`);
  return { settings: { ...s }, errors };
}
