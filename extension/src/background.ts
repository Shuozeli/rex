// Service worker: owns settings, the wasm engines, and extraction. The recipes are packaged with the
// extension (decisions.md U45): nothing is fetched from the network, and page content never leaves the browser.

import type { RexEngine } from "../wasm/rex_wasm.js";
import { engineFor, idOfRecipe, redactForFixture, releaseEngine, retainEngines, runExtract, sha256, version } from "./engine";
import { t } from "./i18n";
import { builtin } from "./recipes";
import { currentRules, recipeHosts, updateRules } from "./rules";
import { loadSettings, storeSettings, validateSettings } from "./settings";
import type { ExtractResponse, FixtureResponse, Request, Settings, SettingsResponse, Status, StatusResponse, ValidateResponse } from "./types";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

// Settings writes run one at a time, each reading the settings current when it starts.
let queue: Promise<unknown> = Promise.resolve();
function exclusive<T>(op: () => Promise<T>): Promise<T> {
  const run = queue.then(op, op);
  queue = run.catch(() => undefined);
  return run;
}

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
  const recipes = await builtin().then(
    (b) => ({ count: b.recipes.length, generatedAt: b.index.generatedAt ?? null, skipped: b.skipped }),
    () => null,
  );
  return {
    extensionVersion: __REX_VERSION__,
    engineVersion: await version(),
    recipes,
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
  engine: RexEngine;
  /** Id of the local recipe merged into this engine. */
  localId: string | null;
}

// Compile errors of local recipes that parse but do not build into an engine, by recipe hash; the options page
// shows them, and extraction falls back to the packaged recipes alone.
const localCompileErrors = new Map<string, string>();

/** The engine over the packaged recipes, with the developer-mode local recipe merged in when it compiles. */
async function engine(settings: Settings): Promise<Candidate> {
  const local = await localRecipe(settings);
  const b = await builtin();
  const key = `builtin|${b.index.generatedAt ?? ""}|${b.index.schemas.sha256}`;
  if (local.id !== null) {
    const localId = local.id;
    const localHash = await sha256(new TextEncoder().encode(settings.localRecipe));
    if (!localCompileErrors.has(localHash)) {
      const merged = b.recipes.filter((r) => (JSON.parse(r) as { id: string }).id !== localId).concat(settings.localRecipe);
      const localKey = `${key}+local:${localHash}`;
      try {
        const e = await engineFor(localKey, b.fds, merged);
        retainEngines(new Set([localKey]));
        return { engine: e, localId };
      } catch (e) {
        localCompileErrors.clear();
        localCompileErrors.set(localHash, errorText(e));
      }
    }
  }
  const e = await engineFor(key, b.fds, b.recipes);
  retainEngines(new Set([key]));
  return { engine: e, localId: null };
}

// `nowMs` (e2e only) pins the clock that relative dates ("3 hours ago") resolve against.
async function extract(tabId: number, recipeId?: string, nowMs?: number): Promise<ExtractResponse> {
  try {
    const settings = await loadSettings();
    const c = await engine(settings).catch((e: unknown) => {
      throw new Error(`${t("errNoRecipes")}: ${errorText(e)}`);
    });
    try {
      const page = await readTab(tabId);
      let output = runExtract(c.engine, page.url, page.html, recipeId, {}, nowMs);
      // Developer mode: a confirmed local recipe wins over the bundle's ranking.
      if (recipeId === undefined && c.localId !== null && output.plan?.confirmed.includes(c.localId) && output.plan.chosen !== c.localId) {
        output = runExtract(c.engine, page.url, page.html, c.localId, {}, nowMs);
      }
      const local = c.localId !== null && output.extraction?.recipe?.id === c.localId;
      return { ok: true, page: { url: page.url, title: page.title }, output, local, status: await status(settings) };
    } finally {
      releaseEngine(c.engine);
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
  if (settings.devMode && settings.localRecipeEnabled) {
    // Build the engine now so a local recipe that parses but does not compile is reported right away.
    await engine(settings).then((c) => releaseEngine(c.engine), () => {});
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

/** The toolbar icon rules follow the packaged recipes; they are set again on install, update and browser start. */
async function startup(): Promise<void> {
  await updateRules();
}

chrome.runtime.onInstalled.addListener(() => void startup());
chrome.runtime.onStartup.addListener(() => void startup());

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
    status: async () => status(await loadSettings()),
    saveSettings: (s: Settings) => saveSettings(s),
    settings: () => loadSettings(),
    extractTab: (tabId: number, recipeId?: string, nowMs?: number) => extract(tabId, recipeId, nowMs),
    fixture: (tabId: number, recipeId: string | null) => fixture(tabId, recipeId),
    rules: async () => ({ hosts: await recipeHosts(), rules: await currentRules() }),
    storage: () => chrome.storage.local.get(null),
  };
}
