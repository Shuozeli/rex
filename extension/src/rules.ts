// Toolbar icon state (FR-EX-11): declarativeContent rules generated from the synced indexes mark sites
// that have a site recipe with a green dot. Host-level only (stage 1's path filters are not mirrored);
// the generic fallback still works everywhere, so the icon is never disabled.

import { loadSource } from "./sync";
import type { Settings } from "./types";

const RULE_ID = "rex-site-recipes";

function matchersFor(host: string): chrome.declarativeContent.PageStateMatcher[] {
  const m = (pageUrl: chrome.events.UrlFilter) => new chrome.declarativeContent.PageStateMatcher({ pageUrl });
  if (host.startsWith("*.")) {
    const base = host.slice(2);
    return [m({ hostSuffix: `.${base}` }), m({ hostEquals: base })];
  }
  const bare = host.startsWith("www.") ? host.slice(4) : host;
  return [m({ hostEquals: bare }), m({ hostEquals: `www.${bare}` })];
}

/** Hosts with at least one usable (non-base, not skipped) site recipe across enabled sources. */
export async function recipeHosts(settings: Settings): Promise<string[]> {
  const hosts = new Set<string>();
  for (const src of settings.sources.filter((s) => s.enabled)) {
    const stored = await loadSource(src);
    if (stored === null) continue;
    for (const e of stored.index.entries) {
      if (e.base === true || stored.recipes[e.ref.sha256] === undefined) continue;
      for (const h of e.urls?.hosts ?? []) hosts.add(h.toLowerCase());
    }
  }
  return [...hosts].sort();
}

async function badgeIcons(): Promise<Record<number, ImageData>> {
  const out: Record<number, ImageData> = {};
  for (const size of [16, 32]) {
    const res = await fetch(chrome.runtime.getURL(`icons/icon-${size}.png`));
    const bitmap = await createImageBitmap(await res.blob());
    const canvas = new OffscreenCanvas(size, size);
    const g = canvas.getContext("2d");
    if (g === null) throw new Error("OffscreenCanvas 2d context unavailable");
    g.drawImage(bitmap, 0, 0);
    const r = size * 0.2;
    const c = size - r - size * 0.04;
    g.beginPath();
    g.arc(c, c, r + size * 0.06, 0, 2 * Math.PI);
    g.fillStyle = "#ffffff";
    g.fill();
    g.beginPath();
    g.arc(c, c, r, 0, 2 * Math.PI);
    g.fillStyle = "#2b8a3e";
    g.fill();
    out[size] = g.getImageData(0, 0, size, size);
  }
  return out;
}

function removeRules(): Promise<void> {
  return new Promise((resolve) => chrome.declarativeContent.onPageChanged.removeRules(undefined, () => resolve()));
}

function addRules(rules: chrome.events.Rule[]): Promise<void> {
  return new Promise((resolve, reject) =>
    chrome.declarativeContent.onPageChanged.addRules(rules, () => {
      const err = chrome.runtime.lastError;
      if (err) reject(new Error(err.message));
      else resolve();
    }),
  );
}

export function currentRules(): Promise<chrome.events.Rule[]> {
  return new Promise((resolve) => chrome.declarativeContent.onPageChanged.getRules(resolve));
}

/** Replace the icon rule; returns the number of hosts it covers. */
export async function updateRules(settings: Settings): Promise<number> {
  const hosts = await recipeHosts(settings);
  await removeRules();
  if (hosts.length === 0) return 0;
  const imageData = await badgeIcons();
  await addRules([
    {
      id: RULE_ID,
      conditions: hosts.flatMap(matchersFor),
      actions: [new chrome.declarativeContent.SetIcon({ imageData })],
    },
  ]);
  return hosts.length;
}
