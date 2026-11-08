// The recipes ship inside the package (decisions.md U45): `rex bundle embed` writes recipes/index.json,
// recipes/schemas.fds and recipes/recipes.json at build time, and they are read from the extension itself.
// Nothing is fetched from the network; new recipes arrive with a new extension version.

import { features } from "./engine";
import type { BundleIndex, IndexEntry, SkippedEntry } from "./types";

export interface Builtin {
  index: BundleIndex;
  fds: Uint8Array;
  /** Canonical recipe protojson of the entries this engine can run, in index order. */
  recipes: string[];
  /** Entries left out, with the reason (FR-EX-5). */
  skipped: SkippedEntry[];
  /** Index entries of `recipes`, in the same order. */
  entries: IndexEntry[];
}

let loaded: Promise<Builtin> | null = null;

async function packaged(path: string): Promise<Response> {
  const res = await fetch(chrome.runtime.getURL(`recipes/${path}`));
  if (!res.ok) {
    throw new Error(`recipes/${path} is missing from the package (HTTP ${res.status})`);
  }
  return res;
}

/** The packaged recipes, read once per service worker. */
export function builtin(): Promise<Builtin> {
  if (loaded === null) {
    loaded = (async () => {
      const [index, fds, all] = await Promise.all([
        packaged("index.json").then((r) => r.json() as Promise<BundleIndex>),
        packaged("schemas.fds").then(async (r) => new Uint8Array(await r.arrayBuffer())),
        packaged("recipes.json").then((r) => r.json() as Promise<string[]>),
      ]);
      if (all.length !== index.entries.length) {
        throw new Error(`recipes.json has ${all.length} recipes for ${index.entries.length} index entries`);
      }
      const { keep, skipped } = selectEntries(index.entries, await features());
      const kept = new Set(keep);
      const recipes = all.filter((_, i) => kept.has(index.entries[i]!));
      return { index, fds, recipes, skipped, entries: keep };
    })();
    loaded.catch(() => (loaded = null));
  }
  return loaded;
}

/**
 * FR-EX-5: entries needing features this engine lacks are skipped, and so is everything that extends a
 * skipped (or absent) recipe. The rest still applies.
 */
export function selectEntries(entries: IndexEntry[], supported: string[]): { keep: IndexEntry[]; skipped: SkippedEntry[] } {
  const reasons = new Map<string, string>();
  for (const e of entries) {
    const missing = (e.requires ?? []).filter((f) => !supported.includes(f));
    if (missing.length > 0) {
      reasons.set(e.ref.id, `requires unsupported feature(s): ${missing.join(", ")}`);
    }
  }
  const ids = new Set(entries.map((e) => e.ref.id));
  let changed = true;
  while (changed) {
    changed = false;
    for (const e of entries) {
      const parent = e.extends ?? "";
      if (parent === "" || reasons.has(e.ref.id)) continue;
      if (!ids.has(parent) || reasons.has(parent)) {
        reasons.set(e.ref.id, `extends ${parent}, which is ${ids.has(parent) ? "skipped" : "not in the bundle"}`);
        changed = true;
      }
    }
  }
  return {
    keep: entries.filter((e) => !reasons.has(e.ref.id)),
    skipped: [...reasons].map(([id, reason]) => ({ id, reason })),
  };
}
