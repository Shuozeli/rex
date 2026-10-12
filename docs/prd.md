<!-- agent-updated: 2026-10-12T06:45:00Z -->

# PRD: rex

| | |
|---|---|
| Status | Complete draft v1 (design delegated; see [decisions.md](decisions.md)) |
| Owner | Shuoze Li |
| Date | 2026-10-12 |
| Related | [requirements.md](requirements.md), [design.md](design.md), [engine spec](designs/261012-generic-recipe-extractor.md) |

## 1. Summary

**rex** turns the page you are reading into **one typed JSON record**: an article, a listing feed, and
later products, threads, and more. One click in the browser does it. The conversion is driven by
**recipes**, which are declarative, reviewed, signed data hosted remotely (Cloudflare by default). That
means new sites and fixes ship without an extension release.

The output is never a loose key-value bag. Every record conforms to a registered **schema** (a proto
message with types and schema.org semantics), and every field records where it came from.

rex is an independent project. Crawlers, feed bots, and apps can reuse its engine and recipes, but it
depends on none of them.

## 2. Problem

1. **Saving a page in structured form today is manual or ad hoc.** Copy and paste, one-off scrapers,
   and "save as HTML" all produce inconsistent, unstructured output.
2. **Existing scraping recipes are code.** myfeed's are JavaScript blobs. They can't be hosted
   remotely for a Chrome MV3 extension, can't be audited offline, and break silently.
3. **Server-side crawlers can't see what a person sees.** Logged-in pages, subscriber content, and
   JS-rendered views are only available in the user's own browser.
4. **Sites change constantly.** Fixes must reach users in hours, not after a store review.
5. **A fixed "article" model is too narrow, and generic key-value output is meaningless.** We need
   *typed, extensible* schemas.

## 3. Goals

- **G1. One-click structured capture.** Clicking the action yields a schema-conformant `Extraction`
  that can be copied or downloaded.
- **G2. Typed, semantic, extensible schemas.** Proto schemas with schema.org bindings, starting with
  `article.v1` and `feed.v1`. Adding a schema is data, not an engine release.
- **G3. Dynamic site support.** Recipes and schemas update remotely within hours. Unsupported pages
  still get a generic extraction through the schema.org fallback.
- **G4. Trustworthy.** Recipes are inert data, published as a signed, content-addressed bundle. The
  extension is MV3-compliant. There is no telemetry, and page content never leaves the browser
  unless the user exports it.
- **G5. Maintainable recipe ecosystem.** Recipes live in git with redacted fixtures. CI proves every
  recipe on every change. Authoring tools exist: CLI, `explain`, developer mode, and optional
  LLM-assisted drafting.
- **G6. Reusable engine.** One deterministic Rust engine runs natively, as a CLI, and as wasm, so any
  consumer gets identical results.

## 4. Non-goals (v1)

- Running site-specific JavaScript of any kind, including `chrome.userScripts`.
- Crawling, scheduling, pagination, or following links. The engine processes **one page** that a host
  already has.
- Paywall bypass. The extension reads only what the user's browser renders.
- Uploading captures to any server by default. There is no account and no backend.
- Firefox and Safari. The engine is portable, but only the Chrome extension ships in v1.
- A hosted multi-tenant recipe registry. Git plus static hosting is the registry.

## 5. Users and use cases

| Persona | Need |
|---|---|
| **Reader / researcher** (primary) | On an article or listing, click once to get clean typed JSON for notes, LLM context, or an archive, including logged-in pages. |
| **Recipe author / maintainer** | Write and fix recipes quickly: see per-field provenance and why each candidate missed, capture redacted fixtures from the live page, and get CI proof before merging. |
| **Integrator** (crawlc, myfeed, apps) | Embed the engine or consume the bundle to get the same typed records server-side. |

### User stories

1. As a reader on a supported article, I click the icon and see the title, authors, date, and body
   preview, with **Copy** and **Download**.
2. As a reader on a listing page (for example the HN front page), I get a `Feed` with typed items.
3. As a reader on an unsupported page that has JSON-LD, I still get a typed record from the generic
   recipe for its schema.org type, clearly labeled.
4. As a reader, when a recipe has drifted, I see "the site may have changed", and I can file a
   prefilled report in two clicks.
5. As a maintainer, a merged recipe fix reaches every user within the refresh interval (6 h), or
   immediately on **Refresh**.
6. As an author, I run a local recipe on the live tab, read the `explain` trace, and "Save as fixture"
   to get a redacted test case for my PR.
7. As an integrator, I run `rex extract` or the crate on HTML I already have and get the same
   `Extraction` the extension would produce.

## 6. Release scope

| Release | Scope |
|---|---|
| **v0.1** | Engine (native + wasm). `article.v1` and `feed.v1` with generic recipes plus about 10 site recipes. Signed bundle on R2. Extension: sync, two-stage match, extract, popup, copy and download, report broken. Unpacked or unlisted distribution. |
| **v0.2** | Options (multiple sources, dev channel). Developer mode (local recipe, explain, save as fixture). Supported-page icon state. Keyboard shortcut. Liveness CI. |
| **v0.3** | `rex draft` (BYO-LLM). More schemas on demand (`product`, `thread`). The first consumer integration (myfeed `feed.v1`). Chrome Web Store decision. |

## 7. Success metrics

| Metric | Target |
|---|---|
| Recipe-matched pages extracting `OK` (fixtures plus a liveness sample) | ≥ 95% |
| Generic-recipe pages with a non-empty `primary` field (article pages carrying JSON-LD or OG) | ≥ 85% |
| Click to rendered result, p95, typical page | < 500 ms |
| Merge to available in the extension (forced refresh) | < 15 min; < 6 h passive |
| Native vs wasm conformance | 100% byte-identical |
| wasm size | ≤ 600 KB gzipped |
| Broken-recipe report to fixed release, median | < 3 days |
| Remote-code policy violations | 0 |

## 8. Risks

| Risk | Mitigation |
|---|---|
| Drift degrades results silently | `expect` gives `FAILED_EXPECTATION`; fallbacks still fill fields; one-click reports; liveness CI |
| Recipe authoring is too slow to cover many sites | Generic recipes with schema.org defaulting cover a lot of pages with no site recipe; `explain` and developer-mode fixtures; optional LLM drafting |
| Compromised hosting serves bad recipes | Signed index, pinned keys, sha256 objects, anti-rollback; recipes are inert and bounded |
| Fixtures leak publisher content | Mandatory redaction; unredacted captures stay gitignored |
| The wasm engine is too large or slow for the extension | Size and performance budgets in CI with known levers |
| Schema sprawl | New schemas need a concrete use case, a PR, a generic recipe, and fixtures |

## 9. Open questions

1. The Cloudflare account, bucket, and domain; ownership of the signing key.
2. Repo visibility (`Shuozeli/rex` is private for now).
3. Chrome Web Store: unlisted at v0.1, or unpacked only?
