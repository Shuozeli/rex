// Shared e2e plumbing: launch Chrome with the unpacked e2e build (its recipes are in the package), wait for the
// service worker's rexDebug hooks, and record every request the extension itself makes (it must make none to
// the network: decisions.md U45).

import { chromium } from "playwright-core";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Lets the context see requests made by the extension's service worker.
process.env.PW_EXPERIMENTAL_SERVICE_WORKER_NETWORK_EVENTS = "1";

const here = dirname(fileURLToPath(import.meta.url));
export const ext = resolve(here, "..");
export const repo = resolve(ext, "..");
export const dist = join(ext, "dist");
// Playwright's own Chromium (`pnpm exec playwright-core install chromium`), the same build locally and in CI;
// the manifest needs Chrome 137+, so playwright-core is pinned to a release that ships one.
export const chromePath = process.env.REX_CHROME ?? chromium.executablePath();
export const rex = process.env.REX_BIN ?? join(repo, "target/release/rex");
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
  // Requests by the extension (its worker and its pages) to anything outside the package.
  // `internal` (requests into the package) proves the listener sees the extension's requests at all.
  const external = [];
  const internal = { count: 0 };
  context.on("request", (req) => {
    let from = "";
    try {
      from = req.serviceWorker()?.url() ?? req.frame().url();
    } catch {
      return;
    }
    if (!from.startsWith("chrome-extension://")) return;
    if (req.url().startsWith("chrome-extension://")) internal.count++;
    else external.push(req.url());
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
  return { context, sw, extensionId: new URL(sw.url()).host, close, external, internal };
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
