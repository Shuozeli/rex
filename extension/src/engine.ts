// Lazy wrapper around the rex wasm engine. Service workers may not use top-level await, so the
// module is initialized on first use; engines are cached per recipe set (the packaged one, plus the
// developer-mode local recipe).

import init, {
  RexEngine,
  engineVersion,
  lintHtml,
  recipeId,
  redactHtml,
  sha256Hex,
  supportedFeatures,
} from "../wasm/rex_wasm.js";
import type { EngineOutput, UrlHit } from "./types";

let ready: Promise<void> | null = null;

export function wasmReady(): Promise<void> {
  if (ready === null) {
    ready = init({ module_or_path: chrome.runtime.getURL("wasm/rex_wasm_bg.wasm") }).then(() => undefined);
  }
  return ready;
}

export async function sha256(bytes: Uint8Array): Promise<string> {
  await wasmReady();
  return sha256Hex(bytes);
}

export async function version(): Promise<string> {
  await wasmReady();
  return engineVersion();
}

export async function features(): Promise<string[]> {
  await wasmReady();
  return JSON.parse(supportedFeatures()) as string[];
}

/** Redact a page for a fixture; throws when the lint still finds publisher prose. */
export async function redactForFixture(html: string): Promise<string> {
  await wasmReady();
  const redacted = redactHtml(html);
  const violations = JSON.parse(lintHtml(redacted)) as string[];
  if (violations.length > 0) {
    throw new Error(`redaction left prose behind: ${violations.slice(0, 3).join("; ")}`);
  }
  return redacted;
}

/** The id of a recipe (protojson); throws with the parse error. */
export async function idOfRecipe(text: string): Promise<string> {
  await wasmReady();
  return recipeId(text);
}

interface Entry {
  engine: RexEngine;
  /** Extractions holding the engine; it is freed only when none do. */
  users: number;
  retired: boolean;
}

const cache = new Map<string, Entry>();
const leased = new Map<RexEngine, Entry>();

/**
 * Engine for a recipe set; `key` identifies its content (the packaged set, plus a local recipe's hash). The caller holds it until `releaseEngine`, so a concurrent `retainEngines` cannot free it mid-use.
 */
export async function engineFor(key: string, fds: Uint8Array, recipes: string[]): Promise<RexEngine> {
  await wasmReady();
  let entry = cache.get(key);
  if (entry === undefined) {
    entry = { engine: new RexEngine(fds, recipes), users: 0, retired: false };
    cache.set(key, entry);
    leased.set(entry.engine, entry);
  }
  entry.users++;
  return entry.engine;
}

function freeIfIdle(entry: Entry): void {
  if (entry.retired && entry.users === 0) {
    leased.delete(entry.engine);
    entry.engine.free();
  }
}

export function releaseEngine(engine: RexEngine): void {
  const entry = leased.get(engine);
  if (entry === undefined) return;
  entry.users = Math.max(0, entry.users - 1);
  freeIfIdle(entry);
}

/** Drop engines whose bundles are no longer current; each is freed once no extraction holds it. */
export function retainEngines(keys: Set<string>): void {
  for (const [key, entry] of cache) {
    if (!keys.has(key)) {
      cache.delete(key);
      entry.retired = true;
      freeIfIdle(entry);
    }
  }
}

export function urlHits(engine: RexEngine, url: string): UrlHit[] {
  return JSON.parse(engine.matchUrl(url)) as UrlHit[];
}

export function runExtract(
  engine: RexEngine,
  url: string,
  html: string,
  recipeId: string | undefined,
  hints: Record<string, string>,
  nowMs: number = Date.now(),
): EngineOutput {
  const raw = engine.extract(url, html, recipeId, nowMs, JSON.stringify(hints), `ext/${__REX_VERSION__}`);
  return JSON.parse(raw) as EngineOutput;
}
