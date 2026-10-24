// Service worker: owns settings, sync, the wasm engines, and extraction. Page content never leaves the
// browser.

import type { RexEngine } from "../wasm/rex_wasm.js";
import { engineFor, idOfRecipe, redactForFixture, releaseEngine, retainEngines, runExtract, sha256, urlHits, version } from "./engine";
import { t } from "./i18n";
import { currentRules, recipeHosts, updateRules } from "./rules";
import { loadSettings, storeSettings, validateSettings } from "./settings";
import { fromBase64, loadSource, pruneSources, sourceStatus, syncSource } from "./sync";
import type {
  ExtractResponse,
  FixtureResponse,
  Request,
  Settings,
  SettingsResponse,
  SourceSetting,
  Status,
  StatusResponse,
  ValidateResponse,
} from "./types";

const ALARM = "rex-sync";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

// Settings writes and syncs run one at a time, each reading the settings current when it starts: a
// startup sync must not interleave with an options save and write state for a source that changed.
let queue: Promise<unknown> = Promise.resolve();
function exclusive<T>(op: () => Promise<T>): Promise<T> {
  const run = queue.then(op, op);
  queue = run.catch(() => undefined);
  return run;
}

/** Sync every enabled source with the settings current at the time the sync runs. */
const syncNow = (): Promise<{ settings: Settings; errors: string[] }> =>
  exclusive(async () => {
    const settings = await loadSettings();
    return { settings, errors: await syncAll(settings) };
  });

/** The developer-mode local recipe, when active: its id, or the reason it cannot be used. */
async function localRecipe(settings: Settings): Promise<{ id: string | null; error: string | null }> {
  if (!settings.devMode || !settings.localRecipeEnabled || settings.localRecipe.trim() === "") {
    return { id: null, error: null };
  }
  try {
    const id = await idOfRecipe(settings.localRecipe);
    const compileError = localCompileErrors.get(await sha256(new TextEncoder().encode(settings.localRecipe)));
    return compileError === undefined ? { id, error: null } : { id: null, error: compileError };
  } catch (e) {
    return { id: null, error: errorText(e) };
  }
}

async function status(settings: Settings): Promise<Status> {
  const local = await localRecipe(settings);
  return {
    extensionVersion: __REX_VERSION__,
    engineVersion: await version(),
    sources: await Promise.all(settings.sources.map(sourceStatus)),
    outputFormat: settings.outputFormat,
    devMode: settings.devMode,
    localRecipeId: local.id,
    localRecipeError: local.error,
  };
}

/** Status for an error response; null when even that fails (the original error is still reported). */
async function statusOrNull(): Promise<Status | null> {
  try {
    return await status(await loadSettings());
  } catch {
    return null;
  }
}

async function syncAll(settings: Settings): Promise<string[]> {
  const errors: string[] = [];
  for (const src of settings.sources.filter((s) => s.enabled)) {
    try {
      await syncSource(src);
    } catch (e) {
      errors.push(errorText(e));
    }
  }
  await updateRules(settings);
  return errors;
}

/** Packaged serializer injected into the tab: returns the post-JavaScript DOM as HTML. */
function serializePage(): { url: string; title: string; html: string } {
  const doctype = document.doctype ? new XMLSerializer().serializeToString(document.doctype) : "";
  return { url: location.href, title: document.title, html: doctype + document.documentElement.outerHTML };
}

/** True for the error Chrome gives when the tab's document was replaced while the script was being injected. */
function frameReplaced(e: unknown): boolean {
  return /Frame with ID \d+ was removed|No frame with id/.test(String(e));
}

async function readTab(tabId: number): Promise<{ url: string; title: string; html: string }> {
  // A page that swaps its document after load (client redirect, prerender activation) can drop the frame the
  // script was sent to; the new document is read on a second try (found by the live-page test, U38).
  const inject = () => chrome.scripting.executeScript({ target: { tabId }, func: serializePage });
  const [injected] = await inject().catch(async (e: unknown) => {
    if (!frameReplaced(e)) throw e;
    await new Promise((r) => setTimeout(r, 1000));
    return inject();
  });
  const page = injected?.result;
  if (page === undefined) {
    throw new Error(t("errCannotRead"));
  }
  return page;
}

interface Candidate {
  src: SourceSetting;
  engine: RexEngine;
  /** Id of the local recipe merged into this engine (first synced source only). */
  localId: string | null;
}

// Compile errors of local recipes that parse but do not build into an engine, by recipe hash; the options page
// shows them, and extraction falls back to the bundle alone.
const localCompileErrors = new Map<string, string>();

/** One engine per enabled, synced source, in priority order. The caller releases them (`releaseAll`). */
async function engines(settings: Settings): Promise<Candidate[]> {
  const local = await localRecipe(settings);
  const out: Candidate[] = [];
  const keys = new Set<string>();
  try {
    for (const src of settings.sources.filter((s) => s.enabled)) {
      const stored = await loadSource(src);
      if (stored === null) continue;
      const fds = fromBase64(stored.fdsBase64);
      const recipes = Object.values(stored.recipes);
      // Everything that decides the engine's content: the source's identity (an edited base URL, channel or key
      // set must not reuse an engine built from another bundle), the bundle and the schema set.
      const key = [src.id, src.channel, src.baseUrl, stored.keysFingerprint, stored.sequence, stored.fdsSha].join("|");
      if (out.length === 0 && local.id !== null) {
        const localId = local.id;
        const localHash = await sha256(new TextEncoder().encode(settings.localRecipe));
        if (!localCompileErrors.has(localHash)) {
          const merged = recipes.filter((r) => (JSON.parse(r) as { id: string }).id !== localId).concat(settings.localRecipe);
          const localKey = `${key}+local:${localHash}`;
          try {
            const engine = await engineFor(localKey, fds, merged);
            keys.add(localKey);
            out.push({ src, engine, localId });
            continue;
          } catch (e) {
            localCompileErrors.clear();
            localCompileErrors.set(localHash, errorText(e));
          }
        }
      }
      const engine = await engineFor(key, fds, recipes);
      keys.add(key);
      out.push({ src, engine, localId: null });
    }
  } catch (e) {
    releaseAll(out);
    throw e;
  }
  retainEngines(keys);
  return out;
}

function releaseAll(cands: Candidate[]): void {
  for (const c of cands) releaseEngine(c.engine);
}

/** The first source with a stage-1 hit for the URL, else the first source (generic fallback). */
function choose(cands: Candidate[], url: string, recipeId: string | undefined): Candidate {
  const first = cands[0];
  if (first === undefined) throw new Error("no engines");
  if (recipeId !== undefined) {
    return cands.find((c) => c.engine.recipeIds().includes(recipeId)) ?? first;
  }
  return cands.find((c) => urlHits(c.engine, url).length > 0) ?? first;
}

// `nowMs` (e2e only) pins the clock that relative dates ("3 hours ago") resolve against.
async function extract(tabId: number, recipeId?: string, nowMs?: number): Promise<ExtractResponse> {
  try {
    const settings = await loadSettings();
    const cands = await engines(settings);
    try {
      if (cands.length === 0) {
        return { ok: false, error: t("errNoRecipes"), status: await statusOrNull() };
      }
      const page = await readTab(tabId);
      const c = choose(cands, page.url, recipeId);
      let output = runExtract(c.engine, page.url, page.html, recipeId, {}, nowMs);
      // Developer mode: a confirmed local recipe wins over the bundle's ranking.
      if (recipeId === undefined && c.localId !== null && output.plan?.confirmed.includes(c.localId) && output.plan.chosen !== c.localId) {
        output = runExtract(c.engine, page.url, page.html, c.localId, {}, nowMs);
      }
      const local = c.localId !== null && output.extraction?.recipe?.id === c.localId;
      return { ok: true, page: { url: page.url, title: page.title }, output, sourceId: c.src.id, local, status: await status(settings) };
    } finally {
      releaseAll(cands);
    }
  } catch (e) {
    return { ok: false, error: errorText(e), status: await statusOrNull() };
  }
}

async function fixture(tabId: number, recipeId: string | null): Promise<FixtureResponse> {
  const settings = await loadSettings();
  if (!settings.devMode) {
    return { ok: false, error: "developer mode is off" };
  }
  const page = await readTab(tabId);
  const pageHtml = await redactForFixture(page.html);
  return {
    ok: true,
    fixture: {
      format: "rex.fixture.v1",
      recipe: recipeId,
      url: page.url,
      fetchTime: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
      pageHtml,
    },
  };
}

function saveSettings(input: Settings): Promise<SettingsResponse> {
  return exclusive(() => saveSettingsNow(input));
}

async function saveSettingsNow(input: Settings): Promise<SettingsResponse> {
  const { settings, errors } = validateSettings(input);
  if (errors.length > 0) {
    return { ok: false, error: errors.join("; ") };
  }
  if (settings.localRecipeEnabled && settings.localRecipe.trim() !== "") {
    await idOfRecipe(settings.localRecipe);
  }
  await storeSettings(settings);
  await pruneSources(settings.sources.map((s) => s.id));
  await chrome.alarms.create(ALARM, { periodInMinutes: settings.refreshMinutes });
  await syncAll(settings);
  if (settings.devMode && settings.localRecipeEnabled) {
    // Build the engines now so a local recipe that parses but does not compile is reported right away.
    await engines(settings).then(releaseAll, () => {});
  }
  return { ok: true, settings, status: await status(settings) };
}

async function validateRecipe(text: string): Promise<ValidateResponse> {
  try {
    return { ok: true, id: await idOfRecipe(text) };
  } catch (e) {
    return { ok: false, error: errorText(e) };
  }
}

async function refresh(): Promise<StatusResponse> {
  const { settings, errors } = await syncNow();
  const s = await status(settings);
  return errors.length === 0 ? { ok: true, status: s } : { ok: false, error: errors.join("; "), status: s };
}

async function startup(): Promise<void> {
  const settings = await loadSettings();
  await chrome.alarms.create(ALARM, { periodInMinutes: settings.refreshMinutes });
  await syncNow();
}

chrome.runtime.onInstalled.addListener(() => void startup());
chrome.runtime.onStartup.addListener(() => void startup());

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === ALARM) {
    void syncNow();
  }
});

type Respond = (r: ExtractResponse | StatusResponse | SettingsResponse | ValidateResponse | FixtureResponse) => void;

const withStatus = (error: string) => ({ ok: false as const, error, status: null });
const plain = (error: string) => ({ ok: false as const, error });

function reply<T>(p: Promise<T>, send: (r: T) => void, onError: (error: string) => T): true {
  void p.then(send, (e: unknown) => send(onError(errorText(e))));
  return true;
}

chrome.runtime.onMessage.addListener((msg: Request, _sender, send: Respond) => {
  switch (msg.type) {
    case "extract":
      return reply(extract(msg.tabId, msg.recipeId), send, withStatus);
    case "status":
      return reply(
        loadSettings().then(status).then((s): StatusResponse => ({ ok: true, status: s })),
        send,
        withStatus,
      );
    case "refresh":
      return reply(refresh(), send, withStatus);
    case "getSettings":
      return reply(
        loadSettings().then(async (settings): Promise<SettingsResponse> => ({ ok: true, settings, status: await status(settings) })),
        send,
        plain,
      );
    case "saveSettings":
      return reply(saveSettings(msg.settings), send, plain);
    case "validateRecipe":
      return reply(validateRecipe(msg.text), send, plain);
    case "fixture":
      return reply(fixture(msg.tabId, msg.recipeId), send, plain);
    default: {
      const never: never = msg;
      return never;
    }
  }
});

// Test hooks (e2e builds only): drive the same code paths without a toolbar click.
if (__REX_E2E__) {
  (globalThis as unknown as { rexDebug: unknown }).rexDebug = {
    sync: async () => {
      const { settings, errors } = await syncNow();
      return { errors, status: await status(settings) };
    },
    status: async () => status(await loadSettings()),
    saveSettings: (s: Settings) => saveSettings(s),
    settings: () => loadSettings(),
    extractTab: (tabId: number, recipeId?: string, nowMs?: number) => extract(tabId, recipeId, nowMs),
    fixture: (tabId: number, recipeId: string | null) => fixture(tabId, recipeId),
    rules: async () => ({ hosts: await recipeHosts(await loadSettings()), rules: await currentRules() }),
    storage: () => chrome.storage.local.get(null),
  };
}
