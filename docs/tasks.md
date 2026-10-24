<!-- agent-updated: 2026-10-14T03:30:00Z -->

# Tasks

Milestones are defined in [design.md §14](design.md); engine milestones E1–E5 are in the
[engine spec §17](designs/261012-generic-recipe-extractor.md). Everything after M1 is implemented and
tested locally but **not yet committed** (see the open actions in decisions.md).

## M0: design ✅

- [x] Contracts under `proto/rex` (annotations, article.v1, feed.v1, Extraction, ExtractionRecipe, bundle) (2026-10-12)
- [x] Decisions log, system design, engine spec, PRD, requirements (2026-10-12)
- [x] Repo renamed to `Shuozeli/rex` (2026-10-12)

## M1: engine core + CLI (E1–E3) ✅

- [x] `rex-proto`, `rex-schema`, `rex-dom`, `rex-readability`, `rex-engine` (all sources, transforms, coercion, `each`/`extends`/`expect`/`default`, agreement, two-stage matching) (2026-10-12)
- [x] `CssSource.exclude` (U22); `rex check / extract / explain / match / test`; conformance c01–c06 (2026-10-12)

## M2: readability benchmark + wasm (E4–E5) ✅

- [x] `rex-wasm` in the extension service worker; native/wasm parity (2026-10-12)
- [x] Readability: the full Mozilla selection steps (U12 amended) (2026-10-13)
- [x] Differential benchmark vs Mozilla Readability.js 0.6.0 on 26 real pages: Jaccard median 0.996, p25 0.962 (`tools/readability-bench`) (2026-10-13)
- [x] Size: 1.06 MB gzipped, budget ≤ 1.15 MB in CI (U25; NFR-6 amended) (2026-10-13)
- [x] All fixture and conformance cases under wasm in Node, byte-identical to native (`crates/rex-wasm/tests/conformance.mjs`) (2026-10-13)
- [x] Criterion benchmark and stage profile (`benches/extract.rs`, `examples/profile_stages.rs`): 1.27 MB page parse 9 ms, extraction ~140 ms native, ~450 ms wasm (2026-10-13)

## M3: recipe repo, redaction, CI gates ✅

- [x] `rex-redact` + `rex fixture capture / redact / lint / import` (U19 amended) (2026-10-13)
- [x] Recipes for 12 sites (wsj, HN, Wikipedia, BBC, Guardian, NPR, Ars Technica, ruanyifeng, sspai, Cloudflare blog, lobste.rs, GitHub trending) plus the `go.dev/blog` draft, each with ≥ 2 fixtures (real ones redacted) (2026-10-13)
- [x] Version gate (`rex check --git-base`) (2026-10-13)
- [x] `rex liveness` + `liveness.yaml` (14 live checks over 12 recipes pass; only wsj.com is excluded, paywalled) (2026-10-13)
- [x] `.github/workflows/ci.yml`, `liveness.yml`; `scripts/ci-local.sh` (U29) (2026-10-13)

## M4: publishing ✅ (hosting account pending)

- [x] `rex bundle publish / gc` for S3-compatible hosts and directories; `bundle verify --url` (U26) (2026-10-13)
- [x] Verified end to end on MinIO, including an extension sync with a 304 (2026-10-13)
- [x] `publish.yml` (stable/dev, client verification, weekly GC); key rotation procedure (operations.md) (2026-10-13)
- [ ] **Needs the user**: Cloudflare account, R2 bucket, domain, production key, repo secrets

## M5: extension v0.1 ✅

- [x] Sync, click-to-extract, popup, options, e2e (2026-10-12)

## M6: extension v0.2 ✅

- [x] Editable sources (order, enable, keys, runtime host permission), refresh interval, copy format (FR-EX-9) (2026-10-13)
- [x] Developer mode: local unsigned recipe, expanded trace, Save as fixture (FR-EX-10) (2026-10-13)
- [x] Unsupported `requires` and `min_client_version` handling (FR-EX-5) (2026-10-13)
- [x] `declarativeContent` icon state (FR-EX-11); icons; en/zh UI (NFR-9) (2026-10-13)
- [x] Hermetic e2e: fixture parity, dev mode, fixture round trip, options; sync rejection paths (`tests/e2e-sync.mjs`) (2026-10-13)
- [x] **Bug fixed**: an install-time sync could clobber a just-saved source; worker operations are now serialized (2026-10-13)

## M7: draft tool and integrations, partly done

- [x] `rex draft` (OpenAI-compatible or Gemini; U28); first reviewed draft `go.dev/blog` (2026-10-13)
- [ ] First consumer integration (myfeed `feed.v1`), owned by the consumer (U21)
- [ ] **Needs the user**: Chrome Web Store listing decision

## M8: batch harvest of surveyed sites (U31), in progress

- [x] Site survey: 4340 candidates from Tranco, Majestic and Chinaz (`tools/site-survey`, review page) (2026-10-13)
- [x] `rex gate`, draft repair rounds, focused evidence, fixed URL rules, `--inline-system` flattening (2026-10-13)
- [x] `tools/harvest`: discovery (RSS, homepage, sitemap), URL-shape clusters, thin / URL-shape thin / draft (2026-10-13)
- [x] Article run: 1793 sites processed, **971 recipes** in `recipes/auto/` (779 thin, 146 URL-shape, 46 drafted), 1942 redacted gzip fixtures in rex-fixtures (94 MB), all passing `rex test`/`rex check`/lint (2026-10-13)
- [x] Liveness for auto recipes (`liveness-auto.yaml`, `--jobs`): 962 of 971 pass; the 9 failures are fetch errors (2026-10-13)
- [x] Promotion script (`tools/harvest/promote.py`, 7 daily passes) (2026-10-13)
- [ ] **Needs the user**: create `Shuozeli/rex-fixtures` on GitHub; push both repos
- [ ] **Blocked**: ~420 bot-walled or TLS-fingerprinting sites need a real browser (ARM2 CDP endpoint returns 502)
- [ ] Feed sites (155): harvest path for `feed.v1` (list pages, gate with item checks)
- [ ] Retry the 172 gate failures and 263 "no article-like URL group" sites with better discovery (category pages, sitemap news indexes)
- [ ] New schemas with the user, then harvest them: document/reference (1274 sites), product (698), video (111), thread (90)
- [ ] **Scale**: bundle packing (first sync fetches ~1000 objects one by one) and IndexedDB/`unlimitedStorage` for bundles past ~10 MB; engine build is ~1.1 s for 985 recipes in wasm

## M9: category schemas (U32), in progress

- [x] Schemas `search.v1`, `commerce.v1`, `media.v1`; bases `generic/product.v1`, `generic/video.v1` (2026-10-14)
- [x] Engine: `duration`, `decode` transforms, `each_index`, compact relative dates (2026-10-14)
- [x] Recipes with 2 fixtures each: `amazon.com/product`, `amazon.com/search`, `youtube.com/watch`, `youtube.com/search`, `bing.com/search`; e2e parity in Chrome; liveness for the four reachable over plain HTTP (2026-10-14)
- [x] `extension/scripts/capture.mjs` for rendered-page fixtures (2026-10-14)
- [x] `google.com/search` (ads + organic, direct URLs, dates, related searches); fixtures captured over CDP from the Alienware Chrome profile (2026-10-14)
- [ ] More sites per schema: DuckDuckGo, Baidu; JD, Taobao; Bilibili; YouTube channel/playlist lists
- [ ] Remaining category schemas, one at a time with the user: government/institution documents, knowledge/docs, academic papers, jobs, threads, finance data

## M10: social posts and threads (U33)

- [x] `rex.schema.social.v1` (Thread, Post, Author, PostList) (2026-10-14)
- [x] Recipes: HN item, Stack Overflow / Stack Exchange question, Reddit thread + community, X post + profile, Zhihu question; 2 fixtures each; e2e parity for Reddit and X (2026-10-14)
- [ ] More: Weibo, Xiaohongshu, Bilibili comments, V2EX, Douban groups, Threads, Bluesky, Mastodon
- [x] Chinese platforms on existing schemas (U34): WeChat article, Bilibili video + search, Douyin video, Baidu search; 2 fixtures each, e2e parity, daily liveness for Bilibili and Baidu (2026-10-14)
- [ ] JD product: needs a logged-in JD session in the capture browser (desktop requires login; mobile masks prices)
- [x] US platforms on existing schemas (U35): Walmart product + search, eBay item + search, Target product + search, Best Buy search, TikTok video, Bluesky post, Threads post, Quora question, DuckDuckGo + Yahoo search, Medium article; 2 fixtures each, e2e parity, daily liveness for Walmart, Target product and Medium (2026-10-14)
- [ ] US follow-ups: profile/timeline pages (TikTok, Bluesky, Threads) as PostList/VideoList; Vimeo (Turnstile), Etsy (DataDome), Best Buy product pages need a different capture path
- [x] schema.org bases (U36): howto (Recipe, HowTo), faq, event, place (Business), job, app, creative (Title, Book) + common; 9 generic recipes, Goodreads site recipe, `fallback_rank` for the schema.org fallback (2026-10-14)
- [ ] schema.org follow-ups: Greenhouse job boards (no JSON-LD), structured opening hours (openingHoursSpecification), recipe step sections, more site recipes where generic output is thin
- [x] Market quotes (U37): `rex.schema.finance.v1.Quote`; Yahoo Finance, CNBC (+ generic FinancialQuote), CoinMarketCap, Eastmoney stock, Tiantian Fund, Xueqiu (2026-10-15)
- [ ] Quote follow-ups: Google Finance (redesign), Morningstar, CoinGecko (403), as_of times (pages show times without dates), index pages on Chinese sites
- [x] Live-page test in a real Chrome on the ARM Windows host (U38): runner, case list, fixes for App Store, BBC, YouTube search, Target product (2026-10-15)
- [ ] Live test follow-ups: a scheduled daily run with a report; a logged-in profile for X; re-capture plain-HTTP fixtures of client-rendered sites in a browser
- [x] Code-review fixes (U39): engine budgets and depth limit, redaction of URLs/JSON/comments with all fixtures re-redacted, publish gate after CI, per-(base URL, channel) rollback floor, engine leases, publish self-heal, gate/draft/CLI/tool fixes (2026-10-19)
- [ ] Review follow-ups: extraction speed on heavy pages (CNBC 3-5 s, IMDb 2.5 s on a real click); restore the 6 synthetic fixtures the re-redaction x'd out (wsj x2, generic article x2, HN front x2) from their sources
- [x] Recorded e2e on a Docker Ubuntu desktop (U40): `extension/recorder/`, Google Chrome stable with the CRX force-installed by policy, real input via xdotool, 50 fps x11grab, captions, results.json; tutorial scenario 6/7 with the store build (2026-10-23)
- [x] Hermetic docker e2e (U40): `scripts/docker-e2e.sh`, store build in Google Chrome, no network, fixtures + locally published bundle, 7/7 against expected.json; opt-in ci-local step (2026-10-24)
- [x] GitHub Actions `docker-e2e` job (U40): image layer cache with a weekly Chrome refresh, results + video artifacts; simulated locally 7/7, first GitHub run pending a push (2026-10-24)
- [ ] Recorder follow-ups: more fixture scenes (one per site recipe?); residential proxy for live recordings (Amazon); Windows backend in the framework; scheduled live run with the video and results published
- [ ] Next in the approved order: table/list extraction

## Known gaps

- [ ] **Gap**: `srcset`-only `<img>` inside HTML-kind values loses the image, because ammonia keeps
  `src` only. The spec says to reduce `srcset` to its largest candidate in `src`; this is only
  implemented for `TAKE_ATTRIBUTE srcset`.
- [ ] **Gap**: `rex check` stops at the first `extends` error for a recipe, so later errors in that recipe
  are reported only after the first is fixed.
- [ ] **Gap**: JSON-LD object coercion does not check the object's `@type` against the target record (an
  `Organization` fills `Person`, which is intended for authors but is not checked in general).
- [ ] **Gap**: serve-lib sends no `ETag`, so dev syncs from it re-download `index.json`. S3/R2/MinIO send
  ETags, and the 304 path is tested.
- [ ] **Gap**: only the top frame is serialized; shadow DOM is not included yet (engine spec §18.1).
- [x] **Gap closed (U40)**: the `activeTab` toolbar-click path is covered by the recorder, which clicks the
  real toolbar icon of the store build in Google Chrome.
- [ ] **Gap**: icon state is host-level; stage-1 path filters (`include`/`exclude`) are not mirrored into
  `declarativeContent`.
- [ ] **Gap**: the developer-mode local recipe accepts protojson only (no YAML in the wasm, U27).
- [ ] **Gap**: the CI `extension` job's store build uses `config.example.json` (placeholder source and
  key). A release build needs the production config (operations.md).
