// Capture pages as the extension sees them: load in Chromium, wait for the page to settle, then serialize the
// rendered DOM exactly like the extension's serializePage (doctype + documentElement.outerHTML). Writes
// <root>/<id>/<case>/{page.html, meta.json} like `rex fixture capture`, into fixtures-private/ by default.
//
// Usage: node scripts/capture.mjs <id> <case> <url> [--root ../fixtures-private] [--wait <css selector>]
//                                   [--cdp http://host:9222]
//   --cdp       use an already-running Chrome (a real profile, e.g. for sites that challenge fresh
//               automated browsers): a new tab is opened and only that tab is closed afterwards
//   REX_CHROME  Chromium binary for the default local launch (the Playwright cache, as in tests/)

import { chromium } from "playwright-core";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  if (i < 0) return fallback;
  const v = args[i + 1];
  args.splice(i, 2);
  return v;
};
const root = resolve(opt("--root", "../fixtures-private"));
const wait = opt("--wait", "");
const cdp = opt("--cdp", "");
const [id, caseName, url] = args;
if (!id || !caseName || !url) {
  console.error("usage: capture.mjs <id> <case> <url> [--root dir] [--wait selector]");
  process.exit(2);
}
let browser = null;
let context;
let profile = "";
if (cdp) {
  browser = await chromium.connectOverCDP(cdp);
  context = browser.contexts()[0];
} else {
  const chrome = process.env.REX_CHROME ?? join(homedir(), ".cache/ms-playwright/chromium-1234/chrome-linux64/chrome");
  profile = mkdtempSync(join(tmpdir(), "rex-capture-"));
  context = await chromium.launchPersistentContext(profile, {
    executablePath: chrome,
    headless: false,
    args: ["--headless=new", "--no-first-run", "--lang=en-US"],
    locale: "en-US",
    viewport: { width: 1366, height: 1000 },
  });
}
const page = await context.newPage();
try {
  await page.goto(url, { waitUntil: "domcontentloaded", timeout: 45000 });
  if (wait) await page.waitForSelector(wait, { timeout: 20000 });
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  // Lazy lists render on scroll; one pass down the page, then back to the top.
  await page.evaluate(async () => {
    for (let y = 0; y < document.body.scrollHeight && y < 6000; y += 800) {
      window.scrollTo(0, y);
      await new Promise((r) => setTimeout(r, 250));
    }
    window.scrollTo(0, 0);
  });
  const html = await page.evaluate(() => {
    const doctype = document.doctype ? new XMLSerializer().serializeToString(document.doctype) : "";
    return doctype + document.documentElement.outerHTML;
  });
  const dir = join(root, id, caseName);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "page.html"), html);
  writeFileSync(
    join(dir, "meta.json"),
    JSON.stringify({ url: page.url(), fetch_time: new Date().toISOString().replace(/\.\d{3}Z$/, "Z") }, null, 2) + "\n",
  );
  console.log(`captured ${html.length} chars (rendered) -> ${dir} [${await page.title()}]`);
} finally {
  if (browser) {
    await page.close();
    await browser.close(); // disconnects; the remote Chrome keeps running
  } else {
    await context.close();
    rmSync(profile, { recursive: true, force: true });
  }
}
