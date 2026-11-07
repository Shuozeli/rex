// End-to-end test in a real Chrome (Chrome for Testing) with the unpacked e2e build loaded.
//
// 1. Sync: by default the repo's recipes are published with `rex bundle publish` and a throwaway key and
//    served at https://bundles.rex.test/ (hermetic, CI-safe). REX_E2E_SOURCE=configured instead syncs the
//    source baked into the build config (e.g. the serve-lib dev bundle or a MinIO/R2 channel).
// 2. Fixture pages are served under their real URLs, extracted through the extension's own code path
//    (serialize in the tab -> rex wasm in the service worker), and the record, status and provenance must
//    equal the native engine's expected.json (native/wasm parity, engine spec §14).
// 3. v0.2: icon rules from the index, developer mode (a local recipe overriding the bundle's, the record
//    copy format, Save as fixture -> `rex fixture import` -> `rex test`), and the options page.
// 4. Popups and the options page are screenshotted into test-results/.
// REX_E2E_LIVE=1 also extracts live pages. REX_E2E_LANG=zh-CN renders the UI in Chinese.
// Sync rejection paths are in tests/e2e-sync.mjs.
//
// Usage: pnpm build:e2e && node tests/e2e.mjs

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromePath, ext, launch, makeKey, publishBundle, reporter, repo, rex, serveBundle, settingsFor } from "./harness.mjs";

const out = join(ext, "test-results");
const lang = process.env.REX_E2E_LANG ?? "en-US";
const suffix = lang.startsWith("en") ? "" : `-${lang}`;
const hermetic = (process.env.REX_E2E_SOURCE ?? "hermetic") === "hermetic";
mkdirSync(out, { recursive: true });

const cases = [
  { name: "wsj-article", url: "https://www.wsj.com/politics/policy/how-trump-lost-midterms-33cf9fd4", fixture: "fixtures/wsj.com/main/midterms", recipe: "wsj.com/main" },
  { name: "wsj-paywall", url: "https://www.wsj.com/articles/fed-holds-rates-steady-4f2a", fixture: "fixtures/wsj.com/main/paywall-stub", recipe: "wsj.com/main" },
  { name: "hn-front", url: "https://news.ycombinator.com/", fixture: "fixtures/news.ycombinator.com/front/front-page", recipe: "news.ycombinator.com/front" },
  { name: "generic-blog", url: "https://blog.example.org/2026/10/declarative-recipes", fixture: "fixtures/generic/article.v1/blogposting-jsonld", recipe: "generic/article.v1" },
  { name: "generic-og", url: "https://notes.example.net/parsing/?ref=home", fixture: "fixtures/generic/article.v1/opengraph-only", recipe: "generic/article.v1" },
  // Category schemas (decisions.md U32): product, video, search results.
  { name: "amazon-product", url: "https://www.amazon.com/dp/B0CFPJYX7P", fixture: "fixtures/amazon.com/product/kindle-paperwhite", recipe: "amazon.com/product" },
  { name: "youtube-watch", url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ", fixture: "fixtures/youtube.com/watch/never-gonna", recipe: "youtube.com/watch" },
  { name: "google-search", url: "https://www.google.com/search?q=best+espresso+machine&hl=en", fixture: "fixtures/google.com/search/best-espresso-machine", recipe: "google.com/search" },
  // Social (decisions.md U33).
  { name: "reddit-thread", url: "https://www.reddit.com/r/rust/comments/1woatic/the_allocator_api_feature_has_been_stabilized_on/", fixture: "fixtures/reddit.com/thread/allocator-api", recipe: "reddit.com/thread" },
  { name: "x-status", url: "https://x.com/Tesla/status/2110165104713183485", fixture: "fixtures/x.com/status/tesla-post", recipe: "x.com/status" },
  { name: "bing-search", url: "https://www.bing.com/search?q=rust+webassembly&setlang=en", fixture: "fixtures/bing.com/search/rust-wasm", recipe: "bing.com/search" },
  // Chinese platforms (decisions.md U34).
  { name: "wechat-article", url: "https://mp.weixin.qq.com/s?src=11&timestamp=1791948848&ver=7025&signature=OPDBDGNbGr96ULqXdFf5spsR02QS-MScDkzLW3kgyAgMtUlLowjdtSUcoLU*WDEwzARRl7pGwAmL5DJXrbD2K5IzNHMLYIui1A3pBQqciFODoHuu9QaI4vKGZF6m3yS-&new=1", fixture: "fixtures/mp.weixin.qq.com/article/a1", recipe: "mp.weixin.qq.com/article" },
  { name: "bilibili-video", url: "https://www.bilibili.com/video/BV149WF6zEw2/", fixture: "fixtures/bilibili.com/video/anji", recipe: "bilibili.com/video" },
  { name: "bilibili-search", url: "https://search.bilibili.com/all?keyword=rust%E6%95%99%E7%A8%8B", fixture: "fixtures/bilibili.com/search/rust", recipe: "bilibili.com/search" },
  { name: "douyin-video", url: "https://www.douyin.com/video/7664968192621064930", fixture: "fixtures/douyin.com/video/v1", recipe: "douyin.com/video" },
  { name: "baidu-search", url: "https://www.baidu.com/s?wd=rust%20webassembly", fixture: "fixtures/baidu.com/search/rust", recipe: "baidu.com/search" },
  // US platforms (decisions.md U35).
  { name: "walmart-product", url: "https://www.walmart.com/ip/Wireless-Earbuds-Bluetooth-Headphones-Earphones-with-Wireless-Charging-Case-for-Android-iOS-for-TV-Smart-Phone-Computer-Laptop/20175403739", fixture: "fixtures/walmart.com/product/earbuds", recipe: "walmart.com/product" },
  { name: "walmart-search", url: "https://www.walmart.com/search?q=air+fryer", fixture: "fixtures/walmart.com/search/airfryer", recipe: "walmart.com/search" },
  { name: "ebay-item", url: "https://www.ebay.com/itm/307218209972", fixture: "fixtures/ebay.com/item/k1", recipe: "ebay.com/item" },
  { name: "ebay-search", url: "https://www.ebay.com/sch/i.html?_nkw=mechanical+keyboard", fixture: "fixtures/ebay.com/search/keyboard", recipe: "ebay.com/search" },
  { name: "target-product", url: "https://www.target.com/p/cuisinart-14-cup-programmable-coffeemaker-stainless-steel/-/A-94139349", fixture: "fixtures/target.com/product/cuisinart", recipe: "target.com/product" },
  { name: "target-search", url: "https://www.target.com/s?searchTerm=coffee+maker", fixture: "fixtures/target.com/search/coffee", recipe: "target.com/search" },
  { name: "bestbuy-search", url: "https://www.bestbuy.com/site/searchpage.jsp?id=pcat17071&st=laptop", fixture: "fixtures/bestbuy.com/search/laptop", recipe: "bestbuy.com/search" },
  { name: "tiktok-video", url: "https://www.tiktok.com/@nasa/video/7695090484124929293", fixture: "fixtures/tiktok.com/video/n1", recipe: "tiktok.com/video" },
  { name: "bsky-post", url: "https://bsky.app/profile/bsky.app/post/3l6oveex3ii2l", fixture: "fixtures/bsky.app/post/p1", recipe: "bsky.app/post" },
  { name: "threads-post", url: "https://www.threads.com/@zuck/post/DeZtp-TkQHh", fixture: "fixtures/threads.com/post/t1", recipe: "threads.com/post" },
  { name: "quora-question", url: "https://www.quora.com/What-is-the-meaning-of-life", fixture: "fixtures/quora.com/question/life", recipe: "quora.com/question" },
  { name: "duckduckgo-search", url: "https://duckduckgo.com/?q=best+espresso+machine&ia=web", fixture: "fixtures/duckduckgo.com/search/espresso", recipe: "duckduckgo.com/search" },
  { name: "yahoo-search", url: "https://search.yahoo.com/search?p=best+espresso+machine", fixture: "fixtures/search.yahoo.com/search/espresso", recipe: "search.yahoo.com/search" },
  { name: "medium-article", url: "https://medium.com/rustaceans/corewarz-core-war-learns-to-call-subroutines-67bd18f45c62", fixture: "fixtures/medium.com/article/corewarz", recipe: "medium.com/article" },
  // schema.org bases (decisions.md U36).
  { name: "recipe-generic", url: "https://www.bbcgoodfood.com/recipes/easy-pancakes", fixture: "fixtures/generic/recipe.v1/bbc-pancakes", recipe: "generic/recipe.v1" },
  { name: "howto-generic", url: "https://www.wikihow.com/Tie-a-Tie", fixture: "fixtures/generic/howto.v1/wikihow-tie", recipe: "generic/howto.v1" },
  { name: "faq-generic", url: "https://stripe.com/pricing", fixture: "fixtures/generic/faq.v1/stripe-pricing", recipe: "generic/faq.v1" },
  { name: "event-generic", url: "https://www.eventbrite.com/e/petit-biscuit-at-the-foundry-in-san-francisco-tickets-1996742113412", fixture: "fixtures/generic/event.v1/eventbrite-concert", recipe: "generic/event.v1" },
  { name: "business-generic", url: "https://www.tripadvisor.com/Restaurant_Review-g60713-d360056-Reviews-La_Taqueria-San_Francisco_California.html", fixture: "fixtures/generic/business.v1/tripadvisor-taqueria", recipe: "generic/business.v1" },
  { name: "job-generic", url: "https://jobs.lever.co/palantir/ac978161-6f46-4f6b-ad9e-a258e642751c", fixture: "fixtures/generic/job.v1/lever-admin", recipe: "generic/job.v1" },
  { name: "app-generic", url: "https://play.google.com/store/apps/details?id=com.spotify.music&hl=en_US", fixture: "fixtures/generic/app.v1/googleplay-spotify", recipe: "generic/app.v1" },
  { name: "appstore-rendered", url: "https://apps.apple.com/us/app/duolingo-learn-languages/id570060128", fixture: "fixtures/apps.apple.com/app/duolingo-rendered", recipe: "apps.apple.com/app" },
  { name: "title-generic", url: "https://www.imdb.com/title/tt0111161/", fixture: "fixtures/generic/title.v1/imdb-shawshank", recipe: "generic/title.v1" },
  { name: "book-generic", url: "https://books.google.com/books/about/The_Hobbit.html?id=pD6arNyKyi8C", fixture: "fixtures/generic/book.v1/googlebooks-hobbit", recipe: "generic/book.v1" },
  { name: "goodreads-book", url: "https://www.goodreads.com/book/show/5907.The_Hobbit", fixture: "fixtures/goodreads.com/book/hobbit", recipe: "goodreads.com/book" },
  // Market quotes (decisions.md U37).
  { name: "yahoo-quote", url: "https://finance.yahoo.com/quote/AAPL/", fixture: "fixtures/finance.yahoo.com/quote/aapl", recipe: "finance.yahoo.com/quote" },
  { name: "yahoo-crypto", url: "https://finance.yahoo.com/quote/BTC-USD/", fixture: "fixtures/finance.yahoo.com/quote/btc", recipe: "finance.yahoo.com/quote" },
  { name: "cnbc-quote", url: "https://www.cnbc.com/quotes/AAPL", fixture: "fixtures/cnbc.com/quote/aapl", recipe: "cnbc.com/quote" },
  { name: "coinmarketcap-coin", url: "https://coinmarketcap.com/currencies/bitcoin/", fixture: "fixtures/coinmarketcap.com/coin/btc", recipe: "coinmarketcap.com/coin" },
  { name: "eastmoney-stock", url: "https://quote.eastmoney.com/sh600519.html", fixture: "fixtures/quote.eastmoney.com/stock/maotai", recipe: "quote.eastmoney.com/stock" },
  { name: "eastmoney-fund", url: "https://fund.eastmoney.com/110022.html", fixture: "fixtures/fund.eastmoney.com/fund/f110022", recipe: "fund.eastmoney.com/fund" },
  { name: "xueqiu-stock", url: "https://www.xueqiu.com/S/TSLA", fixture: "fixtures/xueqiu.com/stock/tsla", recipe: "xueqiu.com/stock" },
];

const r = reporter();
const { context, sw, extensionId, close } = await launch({ lang });

const tabIdFor = (url) => sw.evaluate(async (u) => (await chrome.tabs.query({})).find((t) => t.url === u)?.id ?? null, url);

function fixturePage(dir) {
  const plain = join(repo, dir, "page.html");
  return existsSync(plain) ? readFileSync(plain, "utf8") : gunzipSync(readFileSync(`${plain}.gz`)).toString("utf8");
}

async function openFixture(c) {
  const html = fixturePage(c.fixture);
  const page = await context.newPage();
  // Hermetic: the fixture is the only response. Its external scripts would otherwise load live and
  // re-render the page (Bilibili) or hold up the load event (Douyin).
  await page.route("**/*", (route) =>
    route.request().isNavigationRequest() && route.request().frame() === page.mainFrame()
      ? route.fulfill({ status: 200, contentType: "text/html; charset=utf-8", body: html })
      : route.abort(),
  );
  await page.goto(c.url);
  return { page, tabId: await tabIdFor(page.url()) };
}

async function screenshotPopup(tabId, name) {
  const popup = await context.newPage();
  await popup.setViewportSize({ width: 440, height: 1000 });
  await popup.goto(`chrome-extension://${extensionId}/popup.html?tab=${tabId}`);
  await popup.waitForSelector("#record .field, #message:not([hidden])", { timeout: 15000 });
  await popup.screenshot({ path: join(out, `${name}${suffix}.png`), fullPage: true });
  return popup;
}

try {
  console.log(`extension ${extensionId} loaded in ${chromePath.split("/").slice(-3).join("/")}`);

  // 1. Sync.
  let synced;
  if (hermetic) {
    const key = makeKey("e2e-dev", 5);
    const { files } = publishBundle({ key, channel: "dev" });
    const { baseUrl } = await serveBundle(context, "dev", files);
    const saved = await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), settingsFor({ id: "dev", baseUrl, channel: "dev", keys: [{ keyId: key.keyId, public: key.public }], enabled: true }));
    if (!saved.ok) throw new Error(`saveSettings: ${saved.error}`);
    synced = { errors: [], status: saved.status };
  } else {
    synced = await sw.evaluate(() => globalThis.rexDebug.sync());
  }
  const src = synced.status.sources[0];
  if (synced.errors.length > 0 || !src.sequence || src.lastError) {
    r.fail(`sync: ${JSON.stringify(synced).slice(0, 400)}`);
    throw new Error("cannot continue without a synced bundle");
  }
  console.log(`synced ${src.id} bundle #${src.sequence} (${src.recipeCount} recipes) from ${src.baseUrl}; engine ${synced.status.engineVersion}`);

  // 2. Fixture parity.
  for (const c of cases) {
    const { page, tabId } = await openFixture(c);
    const expected = JSON.parse(readFileSync(join(repo, c.fixture, "expected.json"), "utf8"));
    // Native tests resolve relative dates against the capture time; so does this extraction.
    const fetched = Date.parse(JSON.parse(readFileSync(join(repo, c.fixture, "meta.json"), "utf8")).fetch_time);
    const res = await sw.evaluate(([id, now]) => globalThis.rexDebug.extractTab(id, undefined, now), [tabId, fetched]);
    if (!res.ok) {
      r.fail(`${c.name}: ${res.error}`);
      await page.close();
      continue;
    }
    const got = res.output.extraction;
    const checks = {
      recipe: [got?.recipe?.id, c.recipe],
      status: [got?.status, expected.status],
      record: [JSON.stringify(got?.record), JSON.stringify(expected.record)],
      sources: [JSON.stringify(got?.sources), JSON.stringify(expected.sources)],
      expectationsFailed: [JSON.stringify(got?.expectationsFailed), JSON.stringify(expected.expectationsFailed)],
      missingRequired: [JSON.stringify(got?.missingRequired), JSON.stringify(expected.missingRequired)],
      agreement: [got?.bodyReadabilityAgreementBps, expected.bodyReadabilityAgreementBps],
    };
    const bad = Object.entries(checks).filter(([, [a, b]]) => a !== b);
    if (bad.length > 0) {
      for (const [k, [a, b]] of bad) {
        const [x, y] = [String(a), String(b)];
        let i = 0;
        while (i < x.length && x[i] === y[i]) i++;
        const at = Math.max(0, i - 80);
        r.fail(`${c.name}.${k} (first difference at ${i}): wasm=…${x.slice(at, i + 120)} native=…${y.slice(at, i + 120)}`);
      }
    } else {
      console.log(`ok   ${c.name}: ${got.recipe.id} ${got.status.replace("EXTRACTION_STATUS_", "")} — record identical to native (fuel ${res.output.fuelUsed})`);
    }
    await (await screenshotPopup(tabId, c.name)).close();
    await page.close();
  }

  // 3a. Icon state (FR-EX-11): one declarativeContent rule covering every site recipe host.
  const rules = await sw.evaluate(() => globalThis.rexDebug.rules());
  const want = ["news.ycombinator.com", "wsj.com", "*.wikipedia.org"];
  r.check(
    `icon rules (${rules.hosts.length} hosts)`,
    want.every((h) => rules.hosts.includes(h)) && rules.rules.length === 1 && rules.rules[0].conditions.length >= rules.hosts.length,
    JSON.stringify(rules).slice(0, 300),
  );

  // 3b. Developer mode (FR-EX-10): a local unsigned recipe replaces the bundle recipe with the same id.
  const wsjCase = cases[0];
  const local = JSON.parse(execFileSync(rex, ["fmt", "--json", join(repo, "recipes/sites/wsj.com/main.yaml")], { encoding: "utf8" }));
  local.version = 99;
  const base = await sw.evaluate(() => globalThis.rexDebug.settings());
  const saved = await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), {
    ...base,
    devMode: true,
    localRecipe: JSON.stringify(local),
    localRecipeEnabled: true,
    outputFormat: "record",
  });
  r.check("developer settings saved", saved.ok && saved.status.localRecipeId === "wsj.com/main", JSON.stringify(saved).slice(0, 300));
  {
    const { page, tabId } = await openFixture(wsjCase);
    const res = await sw.evaluate(([id]) => globalThis.rexDebug.extractTab(id), [tabId]);
    const expected = JSON.parse(readFileSync(join(repo, wsjCase.fixture, "expected.json"), "utf8"));
    r.check(
      "developer mode: local wsj.com/main@99 used, record identical to the bundle recipe's",
      res.ok && res.local && res.output.extraction?.recipe?.version === 99 && JSON.stringify(res.output.extraction.record) === JSON.stringify(expected.record),
      JSON.stringify({ ok: res.ok, local: res.local, recipe: res.output?.extraction?.recipe, error: res.error }).slice(0, 300),
    );
    const popup = await screenshotPopup(tabId, "dev-local");
    const ui = await popup.evaluate(() => ({
      fixture: !document.getElementById("fixture").hidden,
      other: document.getElementById("copyOther").textContent,
      open: document.getElementById("details").open,
      local: document.querySelector(".badge.local")?.textContent ?? null,
    }));
    r.check(`dev popup (badge "${ui.local}", secondary copy "${ui.other}")`, ui.fixture && ui.open && ui.local !== null && ui.other !== "", JSON.stringify(ui));
    await popup.close();

    // Save as fixture -> rex fixture import -> rex fixture lint -> rex test --update -> rex test.
    const fx = await sw.evaluate(([id]) => globalThis.rexDebug.fixture(id, "wsj.com/main"), [tabId]);
    const root = mkdtempSync(join(tmpdir(), "rex-fixture-"));
    try {
      if (!fx.ok) throw new Error(fx.error);
      const file = join(root, "saved.rexfixture.json");
      writeFileSync(file, JSON.stringify(fx.fixture));
      execFileSync(rex, ["fixture", "import", file, "--root", join(root, "fixtures")], { stdio: "pipe" });
      execFileSync(rex, ["fixture", "lint", join(root, "fixtures")], { stdio: "pipe" });
      execFileSync(rex, ["test", "--recipes", join(repo, "recipes"), "--fixtures", join(root, "fixtures"), "--update"], { stdio: "pipe" });
      const report = execFileSync(rex, ["test", "--recipes", join(repo, "recipes"), "--fixtures", join(root, "fixtures")], { encoding: "utf8" });
      r.check(`Save as fixture round trip (${fx.fixture.pageHtml.length} redacted bytes; ${report.trim().split("\n").pop()})`, true, "");
    } catch (e) {
      r.fail(`fixture round trip: ${e?.stderr?.toString() ?? e}`);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
    await page.close();
  }

  // 3b'. A local recipe that parses but does not compile is reported, and extraction falls back to the bundle.
  {
    const broken = { ...local, fields: [{ path: "title", candidates: [{ css: { selector: "h1[[[" } }] }, ...local.fields.slice(1)] };
    const res = await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), {
      ...base,
      devMode: true,
      localRecipe: JSON.stringify(broken),
      localRecipeEnabled: true,
      outputFormat: "record",
    });
    const { page, tabId } = await openFixture(wsjCase);
    const ex = await sw.evaluate(([id]) => globalThis.rexDebug.extractTab(id), [tabId]);
    r.check(
      "broken local recipe reported, bundle recipe used",
      res.ok && res.status.localRecipeError !== null && res.status.localRecipeId === null && ex.ok && !ex.local && ex.output.extraction?.recipe?.id === "wsj.com/main" && ex.output.extraction.recipe.version !== 99,
      JSON.stringify({ error: res.status?.localRecipeError, ok: ex.ok, local: ex.local, recipe: ex.output?.extraction?.recipe, exError: ex.error }).slice(0, 300),
    );
    await page.close();
  }

  // 3c. Options page (FR-EX-9): renders the sources with their status.
  {
    const opts = await context.newPage();
    await opts.setViewportSize({ width: 900, height: 1100 });
    await opts.goto(`chrome-extension://${extensionId}/options.html`);
    await opts.waitForSelector("#sources tbody tr .st", { timeout: 15000 });
    const rows = await opts.evaluate(() =>
      [...document.querySelectorAll("#sources tbody tr")].map((tr) => ({ id: tr.querySelector(".src-id").value, status: tr.querySelector(".st").textContent })),
    );
    r.check(`options page (${rows.map((x) => `${x.id} [${x.status.split("\n")[0]}]`).join(", ")})`, rows.length > 0 && /#\d+/.test(rows[0].status), JSON.stringify(rows));
    await opts.screenshot({ path: join(out, `options${suffix}.png`), fullPage: true });
    await opts.close();
  }
  await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), base);

  // 4. Live pages (optional).
  if (process.env.REX_E2E_LIVE === "1") {
    const live = [
      { name: "live-hn", url: "https://news.ycombinator.com/" },
      { name: "live-wikipedia", url: "https://en.wikipedia.org/wiki/Web_scraping" },
      { name: "live-go-blog", url: "https://go.dev/blog/go1.22" },
    ];
    for (const c of live) {
      const page = await context.newPage();
      await page.goto(c.url, { waitUntil: "domcontentloaded", timeout: 30000 });
      const tabId = await tabIdFor(page.url());
      const res = await sw.evaluate(([id]) => globalThis.rexDebug.extractTab(id), [tabId]);
      if (!res.ok) {
        r.fail(`${c.name}: ${res.error}`);
        continue;
      }
      const ex = res.output.extraction;
      const rec = ex?.record ?? {};
      const summary = Array.isArray(rec.items)
        ? `${rec.items.length} items; first: ${JSON.stringify(rec.items[0])}`
        : `title=${JSON.stringify(rec.title)} authors=${JSON.stringify(rec.authors)} published=${rec.publishedAt} body=${String(rec.bodyText ?? "").length} chars`;
      console.log(`live ${c.url}: ${ex?.recipe?.id} ${ex?.status} (types ${res.output.plan?.detectedTypes.join(",")}) — ${summary}`);
      await (await screenshotPopup(tabId, c.name)).close();
      await page.close();
    }
  }
} catch (e) {
  r.fail(String(e?.stack ?? e));
} finally {
  await close();
}

console.log(r.failures === 0 ? "\nE2E PASSED" : `\nE2E FAILED (${r.failures})`);
process.exit(r.failures === 0 ? 0 : 1);
