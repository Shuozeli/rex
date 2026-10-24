// Shared e2e plumbing: launch Chrome with the unpacked e2e build, wait for the service worker's rexDebug
// hooks, publish a bundle with the real `rex` CLI and a throwaway Ed25519 key, and serve it to the
// extension at https://bundles.rex.test/ through Playwright routing (no file server, no network).

import { chromium } from "playwright-core";
import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Lets context.route() see fetches made by the extension's service worker.
process.env.PW_EXPERIMENTAL_SERVICE_WORKER_NETWORK_EVENTS = "1";

const here = dirname(fileURLToPath(import.meta.url));
export const ext = resolve(here, "..");
export const repo = resolve(ext, "..");
export const dist = join(ext, "dist");
// Playwright's own Chromium (`pnpm exec playwright-core install chromium`), the same build locally and in CI;
// the manifest needs Chrome 137+, so playwright-core is pinned to a release that ships one.
export const chromePath = process.env.REX_CHROME ?? chromium.executablePath();
export const rex = process.env.REX_BIN ?? join(repo, "target/release/rex");
export const BUNDLE_HOST = "https://bundles.rex.test";

export const sha = (b) => createHash("sha256").update(b).digest("hex");

/** An Ed25519 key from a 32-byte seed filled with `seedByte` (deterministic). */
export function makeKey(keyId, seedByte) {
  const seed = Buffer.alloc(32, seedByte);
  const der = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]);
  const privateKey = createPrivateKey({ key: der, format: "der", type: "pkcs8" });
  const pub = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(-32);
  return { keyId, seed, privateKey, public: pub.toString("hex") };
}

/** A SignedIndex (protojson bytes) over `index`; `mutate` may tamper with the signed envelope. */
export function signIndex(index, key, mutate = (s) => s) {
  const payload = Buffer.from(JSON.stringify(index));
  const s = {
    keyId: key.keyId,
    alg: "Ed25519",
    signature: sign(null, payload, key.privateKey).toString("base64"),
    payload: payload.toString("base64"),
  };
  return Buffer.from(JSON.stringify(mutate(s)));
}

/**
 * `rex bundle publish` of `recipes` for `channel` with `key`. Returns the served file map (path under the
 * channel root -> bytes) and the decoded Index.
 */
export function publishBundle({ recipes = join(repo, "recipes"), key, channel }) {
  const work = mkdtempSync(join(tmpdir(), "rex-e2e-bundle-"));
  try {
    const keyFile = join(work, "key.json");
    writeFileSync(keyFile, JSON.stringify({ key_id: key.keyId, seed: key.seed.toString("hex"), public: key.public }));
    execFileSync(rex, ["bundle", "publish", "--recipes", recipes, "--key", keyFile, "--channel", channel, "--to", join(work, "out")], {
      stdio: "pipe",
    });
    const dir = join(work, "out/v1", channel);
    const files = new Map();
    for (const sub of ["r", "s"]) {
      for (const f of readdirSync(join(dir, sub))) files.set(`${sub}/${f}`, readFileSync(join(dir, sub, f)));
    }
    const indexBytes = readFileSync(join(dir, "index.json"));
    files.set("index.json", indexBytes);
    const index = JSON.parse(Buffer.from(JSON.parse(indexBytes.toString("utf8")).payload, "base64").toString("utf8"));
    return { files, index };
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/** Serve `files` at `${BUNDLE_HOST}/v1/${channel}/` with ETags and conditional GETs. */
export async function serveBundle(context, channel, files, counters = { index: 0, notModified: 0 }) {
  const prefix = `/v1/${channel}/`;
  await context.route(`${BUNDLE_HOST}/**`, async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.startsWith(prefix) ? url.pathname.slice(prefix.length) : null;
    const body = path === null ? undefined : files.get(path);
    if (body === undefined) return route.fulfill({ status: 404, body: "not found" });
    const etag = `"${sha(body)}"`;
    if (path === "index.json") {
      counters.index++;
      if ((await route.request().headerValue("if-none-match")) === etag) {
        counters.notModified++;
        return route.fulfill({ status: 304, headers: { ETag: etag } });
      }
    }
    return route.fulfill({ status: 200, body, headers: { ETag: etag, "Content-Type": "application/octet-stream" } });
  });
  return { baseUrl: `${BUNDLE_HOST}${prefix}`, counters };
}

/** Launch Chrome with the e2e build and wait until the service worker exposes rexDebug. */
export async function launch({ lang = "en-US" } = {}) {
  const profile = mkdtempSync(join(tmpdir(), "rex-e2e-profile-"));
  const context = await chromium.launchPersistentContext(profile, {
    executablePath: chromePath,
    headless: false,
    args: ["--headless=new", `--disable-extensions-except=${dist}`, `--load-extension=${dist}`, "--no-first-run", `--lang=${lang}`],
    locale: lang,
    viewport: { width: 1280, height: 900 },
  });
  let [sw] = context.serviceWorkers();
  if (!sw) sw = await context.waitForEvent("serviceworker", { timeout: 15000 });
  for (let i = 0; !(await sw.evaluate(() => typeof globalThis.rexDebug === "object").catch(() => false)); i++) {
    if (i > 100) throw new Error("service worker never exposed rexDebug (is dist/ an e2e build?)");
    await new Promise((r) => setTimeout(r, 100));
  }
  const close = async () => {
    await context.close();
    rmSync(profile, { recursive: true, force: true });
  };
  return { context, sw, extensionId: new URL(sw.url()).host, close };
}

/** Settings with a single source (developer mode off). */
export function settingsFor(source) {
  return { sources: [source], refreshMinutes: 360, outputFormat: "envelope", devMode: false, localRecipe: "", localRecipeEnabled: false };
}

/** Tracks failures and prints ok/FAIL lines. */
export function reporter() {
  let failures = 0;
  return {
    check(name, ok, detail) {
      if (ok) console.log(`ok   ${name}`);
      else {
        failures++;
        console.log(`FAIL ${name}: ${detail}`);
      }
    },
    fail(msg) {
      failures++;
      console.log(`FAIL ${msg}`);
    },
    get failures() {
      return failures;
    },
  };
}
