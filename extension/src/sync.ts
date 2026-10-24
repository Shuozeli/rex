// Bundle sync (design.md §9.3): conditional GET of index.json, Ed25519 verification against pinned
// keys, client-version gate, anti-rollback, sha256-verified objects, compile check, then one atomic
// storage write. Any failure keeps the previous bundle and records the error.
//
// Stored state is bound to the source identity (base URL, channel, pinned keys): editing any of them
// starts from scratch instead of trusting a bundle verified under different settings.

import { features, sha256, validateBundle, verify } from "./engine";
import { keysFingerprint, originPattern } from "./settings";
import type { BundleIndex, IndexEntry, SkippedEntry, SourceConfig, SourceSetting, SourceStatus, StoredSource } from "./types";

const PREFIX = "rex.source.";
/** Recipe objects fetched at once on a sync. */
const FETCH_CONCURRENCY = 8;
const storageKey = (id: string): string => `${PREFIX}${id}`;
const errorKey = (id: string): string => `${storageKey(id)}.error`;

/** The stored bundle for `src`, or null when none was verified for this exact identity. */
export async function loadSource(src: SourceConfig): Promise<StoredSource | null> {
  const key = storageKey(src.id);
  const got = await chrome.storage.local.get(key);
  const value = got[key] as StoredSource | undefined;
  if (
    value === undefined ||
    value.baseUrl !== src.baseUrl ||
    value.channel !== src.channel ||
    value.keysFingerprint !== keysFingerprint(src.keys)
  ) {
    return null;
  }
  return value;
}

// The anti-rollback floor: the highest sequence ever accepted from a base URL + channel. It is kept apart from
// the stored bundle so that editing the pinned keys (a rotation) or re-adding the source cannot reset it, and
// source pruning never removes it.
const floorKey = (src: SourceConfig): string => `rex.floor.${src.channel}|${src.baseUrl}`;

async function loadFloor(src: SourceConfig): Promise<bigint | null> {
  const got = await chrome.storage.local.get(floorKey(src));
  const v = got[floorKey(src)] as string | undefined;
  return v === undefined ? null : BigInt(v);
}

async function raiseFloor(src: SourceConfig, sequence: string): Promise<void> {
  const floor = await loadFloor(src);
  if (floor === null || BigInt(sequence) > floor) {
    await chrome.storage.local.set({ [floorKey(src)]: sequence });
  }
}

async function saveSource(id: string, value: StoredSource): Promise<void> {
  await chrome.storage.local.set({ [storageKey(id)]: value });
}

async function recordError(src: SourceConfig, prev: StoredSource | null, error: string): Promise<void> {
  if (prev !== null) {
    await saveSource(src.id, { ...prev, lastError: error, lastSync: Date.now() });
  } else {
    await chrome.storage.local.set({ [errorKey(src.id)]: { error, at: Date.now(), baseUrl: src.baseUrl } });
  }
}

async function bootstrapError(src: SourceConfig): Promise<string | null> {
  const got = await chrome.storage.local.get(errorKey(src.id));
  const value = got[errorKey(src.id)] as { error: string; baseUrl: string } | undefined;
  return value !== undefined && value.baseUrl === src.baseUrl ? value.error : null;
}

/** Drop stored bundles of sources that no longer exist. */
export async function pruneSources(keep: string[]): Promise<void> {
  const all = await chrome.storage.local.get(null);
  const live = new Set(keep.flatMap((id) => [storageKey(id), errorKey(id)]));
  const stale = Object.keys(all).filter((k) => k.startsWith(PREFIX) && !live.has(k));
  if (stale.length > 0) {
    await chrome.storage.local.remove(stale);
  }
}

async function fetchBytes(url: string): Promise<Uint8Array> {
  const res = await fetch(url, { cache: "no-cache" });
  if (!res.ok) {
    throw new Error(`GET ${url} -> HTTP ${res.status}`);
  }
  return new Uint8Array(await res.arrayBuffer());
}

function toBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

export function fromBase64(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

/** -1, 0 or 1 for dotted numeric versions ("0.2.0" vs "0.10.1"); missing parts count as 0. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(".").map((x) => Number.parseInt(x, 10) || 0);
  const pb = b.split(".").map((x) => Number.parseInt(x, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d < 0 ? -1 : 1;
  }
  return 0;
}

/**
 * FR-EX-5: entries needing features this engine lacks are skipped, and so is everything that extends a
 * skipped (or absent) recipe. The rest of the bundle still applies.
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

export type SyncResult = "updated" | "not-modified" | "unchanged";

export async function syncSource(src: SourceConfig): Promise<SyncResult> {
  const prev = await loadSource(src);
  try {
    const engineFeatures = await features();
    // A bundle selected by an engine with other features is re-selected even when the index is unchanged.
    const current = prev !== null && (prev.features ?? "") === engineFeatures.join(",");
    const origin = originPattern(src.baseUrl);
    if (!(await chrome.permissions.contains({ origins: [origin] }))) {
      throw new Error(`no host permission for ${origin}; grant it in Options`);
    }
    const headers: Record<string, string> = {};
    if (current && prev?.etag) {
      headers["If-None-Match"] = prev.etag;
    }
    const res = await fetch(new URL("index.json", src.baseUrl), { cache: "no-cache", headers });
    if (res.status === 304 && prev !== null && current) {
      await saveSource(src.id, { ...prev, lastSync: Date.now(), lastError: null });
      return "not-modified";
    }
    if (!res.ok) {
      throw new Error(`GET index.json -> HTTP ${res.status}`);
    }
    const indexBytes = new Uint8Array(await res.arrayBuffer());
    const index = JSON.parse(await verify(indexBytes, src.keys)) as BundleIndex;
    if (index.channel !== src.channel) {
      throw new Error(`index channel "${index.channel}" does not match configured "${src.channel}"`);
    }
    const minVersion = index.minClientVersion ?? "";
    if (minVersion !== "" && compareVersions(__REX_VERSION__, minVersion) < 0) {
      throw new Error(`bundle #${index.sequence} needs extension ${minVersion} or newer (this is ${__REX_VERSION__}); update the extension`);
    }
    const next = BigInt(index.sequence);
    const floor = await loadFloor(src);
    if (floor !== null && next < floor) {
      throw new Error(`rollback rejected: sequence ${next} < ${floor}`);
    }
    if (prev !== null) {
      const last = BigInt(prev.sequence);
      if (next < last) {
        throw new Error(`rollback rejected: sequence ${next} < ${last}`);
      }
      if (next === last && current) {
        await saveSource(src.id, { ...prev, lastSync: Date.now(), lastError: null });
        return "unchanged";
      }
    }

    let fdsBase64 = prev?.fdsSha === index.schemas.sha256 ? prev.fdsBase64 : "";
    if (fdsBase64 === "") {
      const fds = await fetchBytes(new URL(`s/${index.schemas.sha256}.fds`, src.baseUrl).toString());
      const got = await sha256(fds);
      if (got !== index.schemas.sha256) {
        throw new Error(`schema set hash mismatch (${got})`);
      }
      fdsBase64 = toBase64(fds);
    }

    const { keep, skipped } = selectEntries(index.entries, engineFeatures);
    const fetched: Record<string, string> = {};
    const missing = keep.filter((e) => prev?.recipes[e.ref.sha256] === undefined);
    // A first sync fetches every recipe object; a few at a time instead of one after another.
    let nextMissing = 0;
    const fetcher = async (): Promise<void> => {
      while (nextMissing < missing.length) {
        const entry = missing[nextMissing++]!;
        const sha = entry.ref.sha256;
        const bytes = await fetchBytes(new URL(`r/${sha}.json`, src.baseUrl).toString());
        const got = await sha256(bytes);
        if (got !== sha) {
          throw new Error(`recipe ${entry.ref.id} hash mismatch (${got})`);
        }
        fetched[sha] = new TextDecoder().decode(bytes);
      }
    };
    await Promise.all(Array.from({ length: Math.min(FETCH_CONCURRENCY, missing.length) }, fetcher));
    // In index order, whatever order the fetches finished in.
    const recipes: Record<string, string> = {};
    for (const entry of keep) {
      const sha = entry.ref.sha256;
      recipes[sha] = prev?.recipes[sha] ?? fetched[sha]!;
    }

    await validateBundle(fromBase64(fdsBase64), Object.values(recipes));
    await saveSource(src.id, {
      baseUrl: src.baseUrl,
      channel: src.channel,
      keysFingerprint: keysFingerprint(src.keys),
      etag: res.headers.get("ETag"),
      sequence: index.sequence,
      index,
      fdsSha: index.schemas.sha256,
      fdsBase64,
      recipes,
      skipped,
      features: engineFeatures.join(","),
      lastSync: Date.now(),
      lastError: null,
    });
    await raiseFloor(src, index.sequence);
    await chrome.storage.local.remove(errorKey(src.id));
    return "updated";
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    await recordError(src, prev, message);
    throw new Error(`source ${src.id}: ${message}`);
  }
}

export async function sourceStatus(src: SourceSetting): Promise<SourceStatus> {
  const stored = await loadSource(src);
  const hasPermission = await chrome.permissions.contains({ origins: [originPattern(src.baseUrl)] });
  return {
    id: src.id,
    baseUrl: src.baseUrl,
    channel: src.channel,
    enabled: src.enabled,
    hasPermission,
    sequence: stored?.sequence ?? null,
    generatedAt: stored?.index.generatedAt ?? null,
    recipeCount: stored === null ? 0 : Object.keys(stored.recipes).length,
    skipped: stored?.skipped ?? [],
    lastSync: stored?.lastSync ?? null,
    lastError: stored?.lastError ?? (await bootstrapError(src)),
  };
}
