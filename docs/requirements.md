<!-- agent-updated: 2026-10-13T06:00:00Z -->

# Requirements: rex

These are derived from [prd.md](prd.md) and implemented per [design.md](design.md) and the
[engine spec](designs/261012-generic-recipe-extractor.md). **MUST** and **SHOULD** follow RFC 2119.
**Rel** is the first release that ships the requirement.

## 1. Contracts

| ID | Requirement | Rel |
|---|---|---|
| FR-CT-1 | Schemas MUST be proto messages annotated with `rex.schema.v1` options. `map<>` fields MUST be rejected. | v0.1 |
| FR-CT-2 | Recipes MUST be `rex.extract.v1.ExtractionRecipe`. YAML and protojson are 1:1; unknown keys, unknown transforms, and unknown sources MUST be compile errors. | v0.1 |
| FR-CT-3 | The output MUST be `rex.extract.v1.Extraction`, with the record as `google.protobuf.Any`, serialized as protojson. | v0.1 |
| FR-CT-4 | Publishing MUST use `rex.bundle.v1` (`SignedIndex`, content-addressed `r/` and `s/` objects). | v0.1 |
| FR-CT-5 | Within a v1 package, contract evolution MUST be additive. New engine capabilities MUST be gated by `requires`. | v0.1 |

## 2. Engine

| ID | Requirement | Rel |
|---|---|---|
| FR-EN-1 | Given HTML, a URL, hints, a recipe with its bases, and schema descriptors, the engine MUST produce an `Extraction` per engine spec §7. | v0.1 |
| FR-EN-2 | Value semantics MUST follow engine spec §8: text, HTML profiles, dates (`en`/`zh`), numbers (CJK units), URLs. | v0.1 |
| FR-EN-3 | Candidates MUST be tried in order. The first one that survives its pipes and type coercion wins. Misses MUST be recorded as diagnostics. | v0.1 |
| FR-EN-4 | Status MUST be computed with the precedence EMPTY > INVALID > FAILED_EXPECTATION > OK. `expect` only counts the recipe's own candidates. | v0.1 |
| FR-EN-5 | `sources[]` MUST record path, origin, candidate index, and contributing recipe for every populated field. | v0.1 |
| FR-EN-6 | `json_ld: {}` and `microdata: {}` MUST default to the schema's schema.org bindings. JSON-LD objects MUST coerce into message fields through the bindings. | v0.1 |
| FR-EN-7 | The engine MUST expose `match_urls` and `match_page` implementing decisions U2. | v0.1 |
| FR-EN-8 | The engine MUST be deterministic, with time only through `Ctx.now`. Native and wasm outputs MUST be byte-identical on the conformance suite. | v0.1 |
| FR-EN-9 | Limits and fuel MUST follow engine spec §10.3. Exhaustion MUST give `INVALID` with `budget_exceeded`, never a panic or a hang. | v0.1 |

## 3. Recipes, fixtures, and CI

| ID | Requirement | Rel |
|---|---|---|
| FR-RC-1 | A generic recipe MUST exist for every published schema (`generic/<schema>`). | v0.1 |
| FR-RC-2 | Every site recipe MUST declare `match.urls` and have at least 2 redacted fixtures. CI MUST fail otherwise. | v0.1 |
| FR-RC-3 | CI MUST run `rex check` and `rex test` on every PR, and fail on any fixture mismatch. | v0.1 |
| FR-RC-4 | A change to a recipe's canonical content MUST increase its `version` by exactly 1 (CI gate). | v0.1 |
| FR-RC-5 | Committed fixtures MUST be redacted per decisions U19. A CI check MUST reject a fixture containing runs of more than 40 unredacted letters outside allow-listed attributes. | v0.1 |
| FR-RC-6 | `rex explain` MUST show, per field, each candidate's raw value, the transform steps, the coercion result, and the miss reason. | v0.1 |
| FR-RC-7 | `rex draft` (BYO-LLM) MAY write drafts only into `recipes/drafts/`, and MUST NOT bypass CI or review. | v0.3 |

## 4. Publishing

| ID | Requirement | Rel |
|---|---|---|
| FR-PB-1 | Merging to `main` MUST publish `stable`. Changes to `recipes/drafts/` publish `dev`. | v0.1 |
| FR-PB-2 | The index MUST be signed with Ed25519 in a protected CI environment, and `sequence` MUST strictly increase per channel. | v0.1 |
| FR-PB-3 | Objects MUST be uploaded before the index. After publishing, the job MUST verify the live bundle as a client would. | v0.1 |
| FR-PB-4 | Unreferenced objects MUST be retained for at least 30 days. | v0.1 |

## 5. Extension

| ID | Requirement | Rel |
|---|---|---|
| FR-EX-1 | On action click or shortcut, the extension MUST serialize the active tab's DOM and run U2 matching plus extraction in the service worker (rex wasm). | v0.1 |
| FR-EX-2 | The extension MUST package its recipes and schema set, and MUST NOT connect to the internet (decisions.md U45; this reverses the v0.1 rule, which required syncing them). A fresh offline install works at once. | v0.3 |
| FR-EX-3 | ~~Sync verification~~: removed with the sync (U45). The packaged recipes are covered by Chrome's package verification. | v0.3 |
| FR-EX-4 | ~~Sync schedule~~: removed with the sync (U45). New recipes ship with a new extension version. | v0.3 |
| FR-EX-5 | Entries with unsupported `requires` or `min_client_version` MUST be skipped with a diagnostic. | v0.1 |
| FR-EX-6 | The popup MUST show: the matched recipe (or "generic: <schema>"), the status, a typed record preview, per-field origins, and the other confirmed candidates (switchable), plus Copy (envelope or record) and Download. | v0.1 |
| FR-EX-7 | On `FAILED_EXPECTATION` or `INVALID`, the popup MUST offer "Report broken recipe", which opens a prefilled GitHub issue. The URL is included only if the user opts in. | v0.1 |
| FR-EX-8 | Matching and extraction MUST NOT cause any network request. | v0.1 |
| FR-EX-9 | Options: the packaged recipes (count, build date, skipped entries); output format. (Recipe sources, channel and refresh interval were removed with the sync, U45.) | v0.3 |
| FR-EX-10 | Developer mode: a local unsigned recipe (labeled), the `explain` panel, and "Save as fixture" (redacted, downloaded). | v0.2 |
| FR-EX-11 | The icon state SHOULD reflect stage-1 matches, using `declarativeContent` rules generated from the index. | v0.2 |

## 6. Non-functional

| ID | Category | Requirement |
|---|---|---|
| NFR-1 | Policy | MV3 and Chrome Web Store compliant. No remotely hosted code, no `eval` or `new Function`. Only packaged wasm (`wasm-unsafe-eval`). |
| NFR-2 | Permissions | `activeTab`, `scripting`, `storage`, `declarativeContent`. No host permissions, no `tabs`, no `<all_urls>`. |
| NFR-3 | Privacy | No telemetry and no network: CSP `connect-src 'self'`, and every e2e run checks that the extension makes no request (U45). The only way out is a user-initiated issue link. |
| NFR-4 | Security | Engine spec §13 and design.md §10. |
| NFR-5 | Performance | Click to result p95 < 500 ms on a typical page. Engine budgets per engine spec §11. |
| NFR-6 | Footprint | ~~wasm ≤ 600 KB gzipped~~ wasm ≤ 1.15 MB gzipped (amended, decisions.md U25; enforced in CI); packed extension ≤ 2 MB. |
| NFR-7 | Determinism | Byte-identical native and wasm output on conformance. |
| NFR-8 | Compatibility | Chrome ≥ 137. Chromium-based browsers best-effort. |
| NFR-9 | i18n and accessibility | UI in `en` and `zh`, keyboard-navigable. CJK-correct text extraction. |
| NFR-10 | Code quality | Rust: clippy-clean with no allow-bypasses. TS: `strict`, no `any`, exhaustive enum switches. AAA tests. pnpm, and uv for scripts. |

## 7. Acceptance criteria (v0.1)

1. On a WSJ-style article page matching `wsj.com/main`, the extension returns
   `status = EXTRACTION_STATUS_OK`, with `record` of type `rex.schema.article.v1.Article` containing a
   title, authors, `published_at`, and `body_text`, and `recipe.id = "wsj.com/main"`.
2. On the HN front page, it returns a `rex.schema.feed.v1.Feed` with 30 items, each with an `id`,
   `title`, and absolute `url`.
3. On an unknown blog page carrying `BlogPosting` JSON-LD, it uses `generic/article.v1` through the
   schema.org fallback, with origins `JSON_LD` and `READABILITY`.
4. If the site body selector misses on a paywall stub, it returns `FAILED_EXPECTATION`, with
   `expectations_failed = ["body_html"]` and the fallback body filled, and offers the report button.
5. A tampered index, a wrong object hash, or a lower `sequence` is rejected, and the previous bundle
   stays active.
6. The same fixtures produce byte-identical `Extraction`s through `rex extract` (native) and the
   extension's wasm.
