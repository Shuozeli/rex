// User settings (FR-EX-9): defaults from the build config, validation, and storage. Shared by the
// options page (validation before asking for host permissions) and the service worker.

import type { PinnedKey, Settings, SourceSetting } from "./types";

const KEY = "rex.settings";
export const MIN_REFRESH_MINUTES = 60;
export const MAX_REFRESH_MINUTES = 7 * 24 * 60;

export function defaultSettings(): Settings {
  const config = __REX_CONFIG__;
  return {
    sources: config.sources.map((s) => ({ ...s, enabled: true })),
    refreshMinutes: config.refreshMinutes,
    outputFormat: "envelope",
    devMode: false,
    localRecipe: "",
    localRecipeEnabled: false,
  };
}

export async function loadSettings(): Promise<Settings> {
  const got = await chrome.storage.local.get(KEY);
  const stored = got[KEY] as Partial<Settings> | undefined;
  return { ...defaultSettings(), ...(stored ?? {}) };
}

export async function storeSettings(settings: Settings): Promise<void> {
  await chrome.storage.local.set({ [KEY]: settings });
}

/** `https://host/*` for a source base URL (what host permissions are granted for). */
export function originPattern(baseUrl: string): string {
  const u = new URL(baseUrl);
  return `${u.protocol}//${u.hostname}/*`;
}

/** Stable identity of a key set; a stored bundle verified under other keys is not reused. */
export function keysFingerprint(keys: PinnedKey[]): string {
  return keys
    .map((k) => `${k.keyId}:${k.public.toLowerCase()}`)
    .sort()
    .join(",");
}

/** `keyId:hex` per line (the options page format). */
export function parseKeys(text: string): PinnedKey[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "")
    .map((l) => {
      const i = l.lastIndexOf(":");
      // A line without "id:" keeps an empty id, which validation reports (rather than cutting the hex).
      return i < 0 ? { keyId: "", public: l } : { keyId: l.slice(0, i).trim(), public: l.slice(i + 1).trim() };
    });
}

export function formatKeys(keys: PinnedKey[]): string {
  return keys.map((k) => `${k.keyId}:${k.public}`).join("\n");
}

/** Normalizes in place-free fashion and returns every problem found (empty when valid). */
export function validateSettings(s: Settings): { settings: Settings; errors: string[] } {
  const errors: string[] = [];
  const ids = new Set<string>();
  const sources: SourceSetting[] = s.sources.map((src, i) => {
    const where = `source ${i + 1}${src.id ? ` (${src.id})` : ""}`;
    if (!/^[A-Za-z0-9_-]{1,40}$/.test(src.id)) errors.push(`${where}: id must be 1-40 of [A-Za-z0-9_-]`);
    if (ids.has(src.id)) errors.push(`${where}: duplicate id`);
    ids.add(src.id);
    let baseUrl = src.baseUrl.trim();
    try {
      const u = new URL(baseUrl);
      if (u.protocol !== "https:" && u.protocol !== "http:") errors.push(`${where}: base URL must be http(s)`);
      if (u.search !== "" || u.hash !== "") errors.push(`${where}: base URL must not have a query or fragment`);
      if (!baseUrl.endsWith("/")) baseUrl += "/";
    } catch {
      errors.push(`${where}: base URL is not a URL`);
    }
    if (!/^[A-Za-z0-9_-]+$/.test(src.channel)) errors.push(`${where}: channel must be [A-Za-z0-9_-]+`);
    if (src.keys.length === 0) errors.push(`${where}: at least one pinned key is required`);
    for (const k of src.keys) {
      if (k.keyId === "") errors.push(`${where}: a key has no id (use keyId:hex)`);
      if (!/^[0-9a-fA-F]{64}$/.test(k.public)) errors.push(`${where}: key ${k.keyId || "?"} must be 64 hex chars`);
    }
    return { ...src, baseUrl };
  });
  if (!sources.some((src) => src.enabled)) errors.push("enable at least one source");
  if (!Number.isInteger(s.refreshMinutes) || s.refreshMinutes < MIN_REFRESH_MINUTES || s.refreshMinutes > MAX_REFRESH_MINUTES) {
    errors.push(`refresh interval must be between 1 hour and 7 days`);
  }
  return { settings: { ...s, sources }, errors };
}
