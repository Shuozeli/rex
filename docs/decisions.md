<!-- agent-updated: 2026-10-14T03:30:00Z -->

# Decisions log

Every design decision of the **rex** project lives here.

- **U1–U5** were walked with the user one unit at a time.
- From **U6** on, the user delegated the design ("你直接 drive design"), then asked for the whole project
  to be designed independently of crawlc ("我们这个应该是独立于 crawlc 的，你直接把整个项目 design 写完").
  Those units are marked **[delegated]**, and any of them can be reopened.
- Superseded decisions stay listed and are struck through, so history is preserved.

Normative contracts are the protos under [`proto/rex/`](../proto/rex/). [design.md](design.md) is the
system design, and [designs/261012-generic-recipe-extractor.md](designs/261012-generic-recipe-extractor.md)
is the engine spec.

## U1: Output schemas are typed, registered, and semantic, 2026-10-12

The output is more general than a single fixed Article, but **never a generic key-value bag**:

1. A recipe targets exactly one named, versioned schema, and a path outside it is a compile error.
2. Field types are validated.
3. Records and fields bind to schema.org types and properties.
4. Validity (`required`/`primary`) is defined once, in the schema.

**Representation:** proto messages are the schemas. Their semantics are carried as custom options (U3),
and engines load them dynamically from descriptors.

## U2: Recipe matching is two-stage, with a schema.org fallback, 2026-10-12

Matching is a client-side concern (`RecipeMatch` in `extraction_recipe.proto`):

1. **URL prefilter** (`urls`: hosts with `www.` as an alias and `*.` for subdomains, plus include and
   exclude RE2 patterns on path and query). It runs locally, with no DOM.
2. **Page confirmation** (`page`: `any`/`all` of `exists`, `not_exists`, `json_ld_type`, `meta`). It
   reuses the single parse.
3. **Ranking:** an exact host beats a wildcard; a matched `include` beats none; then `priority`, then
   the id.
4. **Fallback:** map the JSON-LD `@type`, `og:type`, or microdata `itemtype` to a schema through
   `record.schema_org`, and run that schema's generic recipe.
5. **No type detected:** the user picks a schema manually.
6. The popup lists every confirmed candidate, so the user can switch.

## U3: Schema annotations, 2026-10-12

These are defined in [`proto/rex/schema/v1/annotations.proto`](../proto/rex/schema/v1/annotations.proto):

- `RecordOptions{schema_org, schema_org_also, primary}`;
- `FieldOptions{kind, schema_org, required, max_items}`;
- `EnumValueOptions{schema_org}`.

Extension numbers are 51200–51202 (the in-organization range; register a global range before any public
schema release).

- Semantics are options, not wrapper types.
- Decimal is a `string` field with `kind: DECIMAL`.
- Datetime is `google.protobuf.Timestamp`.
- `map<>` fields are forbidden in schemas.

## U4: Separate the Extraction envelope from the record, 2026-10-12

This is [`proto/rex/extract/v1/extraction.proto`](../proto/rex/extract/v1/extraction.proto): an
`Extraction` envelope plus a `google.protobuf.Any record`.

- **Status:** `OK`, `INVALID` (a schema `required` field is missing), `FAILED_EXPECTATION` (a recipe
  `expect` missed), or `EMPTY` (every `primary` field is missing). The precedence is
  EMPTY > INVALID > FAILED_EXPECTATION > OK.
- **`body_readability_agreement_bps`:** token-set Jaccard × 10000.
- **Provenance:** a `FieldSource` per populated field.

~~#4 crawlc impact (retire the content half of the crawlc Article, rename its `article` table to
`extraction`)~~ was **superseded by U14**. Changes to crawlc are now crawlc's own choice, not an action
of this project. U14 also made the envelope host-neutral: it has its own `RecipeRef{id, version, sha256}`,
`source_id`/`source_ref` replace `raw_id`/`raw_key`, and it adds `chain` and `FieldSource.recipe`.

## U5: First schema, `rex.schema.article.v1.Article`, 2026-10-12

This is [`proto/rex/schema/article/v1/article.proto`](../proto/rex/schema/article/v1/article.proto):

- `repeated Person authors`;
- `image` stays single-valued;
- there is no `custom` map (former custom contents are promoted to `subtitle` and `language`; a typed
  extension mechanism is deferred);
- `body_text` binds to `articleBody`, and `body_html` is unbound;
- the next schema is `feed.v1`.

## U6: Extraction recipe skeleton [delegated], 2026-10-12

`ExtractionRecipe{id, version, schema, extends, root, match, fields, requires}` and
`FieldRule{path, candidates, pipe, expect, replace_inherited, each, fields}`, defined in
[`extraction_recipe.proto`](../proto/rex/extract/v1/extraction_recipe.proto).

- Cardinality comes from the schema.
- Inheritance is a single `extends` (own candidates first, depth ≤ 3).
- There is no shorthand: YAML is protojson in YAML syntax.

~~#1 placement inside `crawlc.recipe.v1.Recipe`~~ was **superseded by U14**. `ExtractionRecipe` is the
top-level recipe document of rex.

## U7: Candidate sources [delegated], 2026-10-12

The sources are `css`, `json_ld`, `microdata`, `meta`, `link`, `readability`, `url`, `constant`,
`field`, and `hint`, each with an optional candidate-local `pipe`.

- `json_ld: {}` and `microdata: {}` read the field through its schema.org binding.
- An empty CSS selector means the scope element, and a leading combinator gets a `:scope` prefix.
- A JSON-LD object is coerced into a message field by field. A scalar fills the message's single
  required string field.
- `hint` is a host-supplied hint, listed last in generic recipes.
- A `field` reference inherits the origin of the field it points to.

## U8: Transforms [delegated], 2026-10-12

The transform set is closed:

- string: `trim`, `collapse_ws`, `change_case`, `strip`, `truncate`;
- regex (RE2 subset): `regex`, `regex_replace`, `split`;
- list: `join`, `slice`, `unique`, `filter`;
- conversion: `html_to_text`, `json`;
- parsing: `datetime`, `number`, `url`;
- `map` and `default`.

Coercion to the schema type is automatic. Explicit `datetime`/`number` transforms are only needed for
site formats, time zones, `zh`/`en` locales, CJK units, or relative dates.

## U9: `feed.v1` schema [delegated], 2026-10-12

This is [`proto/rex/schema/feed/v1/feed.proto`](../proto/rex/schema/feed/v1/feed.proto): `Feed{title,
items}` (`ItemList`) and `FeedItem{id*, title*, url*, preview, published_at, author, score,
comment_count}` (`ListItem`).

## U10: Where match rules come from [delegated], 2026-10-12

~~Derived by an exporter from crawlc `Plan.filters`~~ was **superseded by U14**. Site recipes must state
`match.urls` themselves. CI rejects a non-base recipe without it. Recipe authors (or the LLM-assisted
draft tool, U18) write `match.page`. A crawler integration may still generate match rules from its own
data, but only inside that integration.

## U11: ~~rex lives in crawlc~~ [superseded by U14]

~~Contracts in crawlc `core/contracts/`, engine in crawlc `extract/`.~~ See U14.

## U12: Readability and sanitizing [delegated], 2026-10-12 (amended during M1)

- ~~The `readability` source is a port of company-feed-server's `feed-content::process_html`.~~
  **Amended:** reading `feed-content` showed it is a whole-page *sanitizer* (it strips nav, header and
  footer) with no main-content selection. Sanitizing is already ammonia's job. So `rex-readability` is a
  deterministic, readability-style scorer:
  - paragraphs vote for their parent and grandparent;
  - tag and class/id weights adjust the scores;
  - link density penalizes;
  - the boilerplate subtrees, headline and byline are dropped from the chosen container;
  - title, byline and excerpt heuristics provide those parts.

  Mozilla Readability.js remains a differential benchmark only.

  Three further steps were added after real-page testing (Wikipedia, which wraps each section in its own
  element), all taken from Mozilla Readability:
  - paragraphs score up to 5 ancestors (dividers 1, 2, then level × 3);
  - a parent climb switches to a higher-scoring wrapper;
  - siblings scoring ≥ max(10, 20% of the best) are merged.

  M2 completed the port of Mozilla's selection steps: alternative candidates (≥ 3 of the top 5 within
  75% share an ancestor, which then wins), the parent climb compares against the *last* score with a
  threshold of top/3, an only-child climb, and a same-class bonus when merging siblings. Text and link
  statistics are computed in one pass. **Benchmark** (`tools/readability-bench`, 26 real pages, Mozilla
  Readability 0.6.0 under jsdom): token-set Jaccard median **0.996**, p25 0.962. The remaining outliers
  (BBC, Cloudflare blog) are covered by site recipes.
- HTML output is sanitized with `ammonia`.

## U13: Normative value semantics and limits [delegated], 2026-10-12

The engine spec is normative for value semantics (§8), limits and fuel (§10.3), and security (§13).

---

## U14: rex is an independent project [delegated, per user direction], 2026-10-12

rex owns its whole stack and depends on no other Shuozeli system:

- its contracts (`rex.*` protos);
- the engine (Rust library, CLI, and wasm);
- the recipe repository and review process;
- publishing and hosting;
- the browser extension;
- the authoring tooling.

**crawlc, myfeed, and others are optional consumers.** They embed the `rex-engine` crate or call the
CLI, and any adapters (for example crawlc `ExtractionSchema` → `ExtractionRecipe`) live in **their**
repos. Nothing in rex references crawlc names, protos, registries, or infrastructure.

## U15: Naming [delegated], 2026-10-12

- **Project and namespace:** `rex` (Recipe EXtractor). Proto packages are `rex.schema.*`,
  `rex.extract.v1`, and `rex.bundle.v1`. Crates are `rex-*`.
- **Extension display name:** "rex: Page to JSON".
- **Repository:** renamed from `Shuozeli/crawlc-extension` to **`Shuozeli/rex`** (private), and the
  local checkout moved to `~/projects/shuozeli/_wip/rex`. Done 2026-10-12 with the user's go-ahead.
- crates.io names are to be checked before any publish (`rex` itself is taken). Fallback prefix:
  `rexdata-`.

## U16: Bundle contract and hosting [delegated], 2026-10-12

This is [`proto/rex/bundle/v1/bundle.proto`](../proto/rex/bundle/v1/bundle.proto).

Layout under `https://<host>/v1/<channel>/`:

| Path | Contents |
|---|---|
| `index.json` | A `SignedIndex`. The Ed25519 signature covers `payload`, the canonical protojson of `Index`. |
| `r/<sha256>.json` | One recipe each. Immutable. |
| `s/<sha256>.fds` | A `FileDescriptorSet` with all referenced schemas. |

- **Channels:** `stable` and `dev`. `sequence` increases strictly per channel (anti-rollback).
- **Clients:** pin 1–2 public keys and skip entries whose `requires` they don't support.
- **Default host:** a Cloudflare R2 bucket on a custom domain, plain static objects with no Worker.
  Cache headers: `index.json` max-age 300; `r/` and `s/` immutable for 1 year.
- **GC:** unreferenced objects are kept for 30 days.
- Any static host (GitHub Pages, a local server) works, because the client side is a pluggable
  `RecipeSource` over a base URL.

## U17: Recipe source of truth and lifecycle [delegated], 2026-10-12

- **Recipes live in git, in this repository under `recipes/`.**
  - `recipes/generic/` holds base recipes.
  - `recipes/sites/<host>/<name>.yaml` holds site recipes.
  - `recipes/drafts/` publishes to the `dev` channel only.
- **A pull request is the human review gate.** CI must pass:
  - contract validation;
  - every site recipe has ≥ 2 fixtures (U19), and every fixture's expected `Extraction` matches;
  - `version` is bumped whenever the canonical content changes;
  - `match.urls` is present on non-base recipes;
  - selector, regex, and limit checks.
- **Publishing:** merging to `main` publishes `stable`. Pushes to `main` that touch `recipes/drafts/`
  publish `dev`. The publish job canonicalizes, hashes, builds the descriptor set, signs (the key is a
  GitHub Actions secret, and the job runs in a protected environment), and uploads to R2 (`r/` and `s/`
  first, `index.json` last).
- **Retiring a recipe:** delete the file. The next index omits it; its objects are GC'd after 30 days.

## U18: Authoring toolchain [delegated], 2026-10-12

The `rex` CLI (from the engine crates):

| Command | Purpose |
|---|---|
| `rex check <recipe…>` | Contract plus semantic validation (what `scripts/validate_recipes.py` prototypes today). |
| `rex extract --recipe … --html … --url …` | Prints the `Extraction`. |
| `rex explain …` | Per-field candidate trace: what each candidate yielded and why it missed. |
| `rex test [recipes…]` | Runs fixtures. |
| `rex fixture capture <url>` / `rex fixture redact` | Builds fixtures (U19). |
| `rex match <url> [--html]` | Runs U2 against the local recipe set. |
| `rex draft --from <fixture> --schema … --llm <openai-compatible endpoint>` | **Optional** BYO-LLM drafting. It writes a draft recipe into `recipes/drafts/`, and the draft is gated by `rex test` plus PR review exactly like a hand-written one. There is no self-repair loop. |

The extension's **developer mode** complements the CLI:

- run a local, unsigned recipe on the live tab;
- show the `explain` trace;
- "Save as fixture", which downloads a redacted fixture of the current page.

## U19: Fixtures and redaction [delegated], 2026-10-12

A fixture is `fixtures/<recipe-id>/<case>/{page.html, meta.json (url, fetch_time, hints),
expected.json (Extraction)}`.

**Committed fixtures must be redacted**, because publisher content cannot be stored in a public repo:

- **Structure is kept:** tags, `id`, `class`, `href`, `src`, `datetime`, `lang`, all `meta` keys, and
  JSON-LD keys and `@type`.
- **Text characters are replaced deterministically:**
  - Latin letters become `x`/`X` (case kept);
  - CJK characters become `字`;
  - digits, punctuation, whitespace, and date- or URL-shaped strings are kept.
- **JSON-LD string values are redacted the same way**, except URLs, dates, and enumerations.

Recipes therefore keep working on redacted pages. Unredacted captures stay in the gitignored
`fixtures-private/` for local debugging only.

**Amended in M3** (implemented as `rex-redact`, used by `rex fixture redact` and by the extension's
"Save as fixture" through wasm):

- **Kept vocabulary**, because recipes legitimately parse it:
  - month and weekday names, time zones, and relative-date words;
  - counter words (comment(s), point(s), vote(s), like(s), view(s), reply/replies, star(s), fork(s),
    share(s), discuss);
  - the CJK date, time and counter characters (年月日时分秒点前天周昨今刚小钟星期一…七上下午评论赞阅读条次人万亿).
- **Raw-text elements are emptied.** Inline `<script>` (other than JSON-LD), `<style>`, `<noscript>`,
  `<iframe>` and similar elements lose their content.
- **The lint is idempotence:** redacting a committed page again must not change it. `rex fixture lint`
  enforces this in CI and reports the first changed text.
- **Consequence for recipes:** a pattern on publisher wording (for example "on behalf of") cannot be tested
  against fixtures. Prefer structure: positions, separators, letter case (case survives redaction), digits.

## U20: Drift and feedback [delegated], 2026-10-12

There is **no telemetry**.

- The popup shows `status`, and on `FAILED_EXPECTATION`/`INVALID` it offers **"Report broken recipe"**.
  This opens a prefilled GitHub issue (recipe ref, status, failing paths, extension and engine
  versions; the page URL only if the user ticks a box). It is user-initiated and has no backend.
- **Liveness CI** (scheduled, daily) runs only for recipes marked `liveness: public` in a sidecar list.
  It fetches the public, login-free sample URLs over HTTP, runs extraction, and opens or updates an
  issue on a non-OK status. Paywalled or login-only sites rely on user reports.

  **Amended:** the sidecar list is `liveness.yaml` (recipe → sample URLs). `rex liveness --report` writes a
  JSON report. `.github/workflows/liveness.yml` runs daily and keeps one open issue labelled `liveness`
  (it comments when the issue already exists).

## U21: Integrations are consumer-owned [delegated], 2026-10-12

| Consumer | Uses | Lives in |
|---|---|---|
| crawlc / dragbv3 | `rex-engine` as its extractor; an `ExtractionSchema` → `ExtractionRecipe` translator; hints from its feed entries; `source_id` = its raw id | crawlc |
| myfeed | `feed.v1` recipes via the `rex-engine` crate or the CLI, replacing its JS recipes | myfeed |
| Any app | The wasm package (`@rex/engine`) or the crate | — |

rex guarantees only its contracts, the engine semver, and the bundle format.

## U22: `CssSource.exclude` [delegated], 2026-10-12

M1 fixtures showed that article bodies routinely contain ads, "read next" boxes and share bars, and CSS
cannot express "this element minus those descendants".

`CssSource` gains `repeated string exclude = 5`: descendant selectors whose subtrees are removed before a
text or HTML value is taken. This is an additive change within `rex.extract.v1`, and it has no effect on
`TAKE_ATTRIBUTE` or `TAKE_EXISTS`.

## U23: Semantics fixed while implementing M1 [delegated], 2026-10-12

- **`expect` is not inherited.** It only constrains the recipe that states it. A more specific recipe
  that redefines the rule without `expect` clears it.
- **`default` only in a rule pipe.** A `default` transform is only allowed in a rule pipe, never in a
  candidate pipe (compile error). It applies only after every candidate missed, with origin `CONST`.
- **Required fields in `each` items.** An `each` item missing a required field is dropped, with diagnostic
  `item_invalid`. Only top-level required fields affect the record status.
- **List provenance.** Provenance for `each` lists is recorded for the parent path and for item 0's
  sub-fields (`items[0].title`), not for every item.
- **Fuel** is charged per DOM node visited (including selector scans), per regex or transform step, and
  per text node walked. It is not part of the output, so the output stays deterministic.
- **Crate layout.** The engine-spec crates `rex-recipe`, `rex-sources` and `rex-transform` are modules of
  `rex-engine` instead of separate crates (fewer crate boundaries, same responsibilities). Generated
  types and the embedded descriptor set live in `rex-proto`.

## U24: Extension prototype and dev hosting [delegated], 2026-10-12

The extension now runs end to end in Chrome.

| Area | Decision |
|---|---|
| Stack | Plain TypeScript (strict) bundled with esbuild. The UI is plain DOM, not WXT/Preact (design.md §3 named WXT/Preact). This is smaller and has fewer moving parts; revisit if the UI grows. |
| Engine | `rex-wasm` (wasm-bindgen `--target web`) runs in the module service worker under `wasm-unsafe-eval`. It is initialized lazily, because service workers forbid top-level await. |
| Signature check | Bundle signatures are verified **inside the wasm engine** (`rex-bundle`, ed25519-dalek), not with WebCrypto. One implementation serves the CLI and the extension, with no dependency on a Chrome version for Ed25519. |
| Storage | v0.1 keeps one `chrome.storage.local` value per source (index, recipes, base64 descriptor set), written in a single atomic `set`. |
| Dev hosting | Until Cloudflare is set up, the dev bundle is published with `rex bundle build` to `~/.local/share/rex/bundle` and served by serve-lib at `http://$TAILSCALE_HOST:8091/rex-bundle/v1/dev/`. The signing key lives in `~/.config/rex/dev-signing-key.json` (never in the repo). The machine-local source URL and public key are in the gitignored `extension/config.local.json`. |
| e2e build | `REX_E2E=1` adds `<all_urls>` host access and the `rexDebug` hooks, because a toolbar click (which grants `activeTab`) cannot be automated. The normal build has neither (verified). |

## U25: Wasm size and speed budget [delegated], 2026-10-13

The original target (NFR-6: ≤ 600 KB gzipped) was not reachable without dropping features. After trimming,
the wasm is about **1.06 MB gzipped**. The trims were:

- `chrono-tz` filtered to about 70 zones (`CHRONO_TZ_TIMEZONE_FILTER` in `.cargo/config.toml`);
- `regex` without the Unicode tables except perl classes and case folding;
- `ammonia` pinned to the html5ever that `scraper` already uses;
- the `wasm` cargo profile (`opt-level = "s"`, LTO, `panic = "abort"`, stripped).

`opt-level = "z"` saves about 8% more but runs about 6× slower than native, so it is not used. **Budget:
≤ 1.15 MB gzipped**, enforced in CI (`WASM_GZ_BUDGET`). The packed extension is about 1.1 MB, inside the
2 MB limit.

| Metric (1.27 MB real page) | Native | Wasm (Chrome) |
|---|---|---|
| Parse | 9 ms | — |
| Full extraction | ~140 ms | ~450 ms |

Recipe consequences: regex classes like `\p{Lu}` (general categories) are not available; use ASCII
classes or `\w`.

## U26: Publishing to S3-compatible hosts [delegated], 2026-10-13

`rex bundle publish` and `rex bundle gc` implement U16 against any S3-compatible host. Cloudflare R2 is
the default; MinIO and AWS S3 also work. A local directory is a host too.

- **Target:** `--to s3://bucket/prefix` (or a directory); `v1/<channel>/` is appended. SigV4 presigned
  requests (`rusty-s3`) over `ureq`. The endpoint and credentials come from `REX_S3_*` (or `AWS_*`)
  environment variables; R2 uses region `auto`.
- **Publish order:**
  1. read and **verify** the live `index.json`; refuse if it does not verify (`--trust key_id:hex` admits
     the previous key during a rotation);
  2. sign `sequence + 1`;
  3. upload missing content-addressed objects (`Cache-Control: public, max-age=31536000, immutable`);
  4. upload `index.json` last (`max-age=300`);
  5. read everything back and verify it.

  Unchanged content is a no-op (no new sequence) unless `--force` is given. `--min-client-version` gates
  older extensions (they keep their bundle).
- **GC:** deletion only happens in `rex bundle gc`, against a verified index. An object becomes
  unreferenced, is recorded in `gc-state.json` with the time it was first seen, and is deleted after
  `--retention-days` (default 30). Clients holding an older index keep working through the retention
  window. Two rules close the race with a concurrent publish: gc re-reads the index right before deleting,
  and publish re-uploads any object that gc has marked. CI also runs both under one concurrency group.
- **Verified** against MinIO on the tailnet: publish, no-op, incremental publish, mark/delete GC, client
  `rex bundle verify --url`, refusal of an unpinned key, a clear SigV4 error for bad credentials, and an
  extension sync with a 304 conditional GET.

## U27: Extension v0.2 [delegated], 2026-10-13

| Area | Decision |
|---|---|
| Settings | `rex.settings` in `chrome.storage.local`, defaulting from the build config. Covers sources (ordered, enable/disable, base URL, channel, pinned keys), refresh interval (1 h–7 d), copy format (envelope or record), and developer mode. Validated in the options page and again in the worker. |
| Source identity | A stored bundle is bound to (base URL, channel, pinned keys). Changing any of them starts that source from scratch instead of trusting a bundle verified under other settings. Removed sources are pruned. |
| Host permission | The default source's origin is in `host_permissions`. Other origins are `optional_host_permissions` (`https://*/*`, `http://*/*`), requested from the options page on Save, inside the click's user activation. Sync refuses a source without permission and says why. |
| Several sources | One engine per synced source. For a page, the first enabled source with a stage-1 URL hit is used, otherwise the first source (its generic fallback). Schema sets are never merged across sources. |
| FR-EX-5 | Entries whose `requires` names a feature this engine lacks are skipped with a reason, and so is everything that `extends` a skipped or absent recipe. The rest of the bundle applies. An index whose `min_client_version` is newer than the extension is rejected as a whole. |
| Developer mode | A local unsigned recipe in **protojson** (`rex fmt --json`). YAML would need `serde_yaml` in the wasm. It replaces the bundle recipe with the same id, wins when it is confirmed, and is labelled "LOCAL · unsigned". The trace panel opens expanded. |
| Save as fixture | Redaction runs in the wasm (`rex-redact`). The page is linted and downloaded as one `*.rexfixture.json`; `rex fixture import` re-lints it and writes `page.html` and `meta.json`. This amends design.md §7.3, which said three files: browsers prompt per download, and `expected.json` must come from the native engine and review anyway (`rex test --update`). |
| Icon state | One `declarativeContent` rule per sync covers the hosts of every usable site recipe, and adds a green dot (`SetIcon`, drawn with `OffscreenCanvas`). The action is never disabled, because the generic fallback works everywhere. Matching is host-level only; stage-1 path filters are not mirrored. |
| Concurrency | Settings saves and syncs (install, startup, alarm, refresh) run through one queue in the worker. Each reads the settings current when it starts. A real race was found by e2e: an install-time sync clobbered a just-saved source. |
| i18n | `src/messages.ts` holds typed `en` and `zh_CN` tables; a missing translation is a compile error. The build renders `_locales` from them and fails on unknown `data-i18n` keys. |
| Icons | Rendered by `tools/icons/make_icons.py` (`{}` on the accent colour) at 16/32/48/128. |

## U28: `rex draft` [delegated], 2026-10-13

Implements design.md §7.2.

- **Endpoints:** any OpenAI-compatible chat-completions URL, or a Gemini `...:generateContent` URL
  (detected from the URL). The model is set with `--model`; the key with `REX_LLM_API_KEY`.
  `--inline-system` folds the system prompt into the first user turn, for endpoints and proxies that
  mishandle system roles.
- **The prompt:**
  - the system prompt embeds `extraction_recipe.proto` (with its comments) as the format specification;
  - the evidence pack holds: the task, the target schema with schema.org bindings, JSON-LD, meta tags,
    repeating sibling groups, a DOM skeleton (runs of identical siblings collapsed, about 24 KB), and the
    schema's base recipe and one reviewed example.
- **Strict parsing:** the reply's JSON object is parsed and compiled together with the existing recipes.
  The id, version and schema are forced. A site draft must have `match.urls.hosts` and at least one field
  rule. Up to 2 retries happen, for these structural errors only.
- **No quality loop:** the draft's status and fields on the page are reported, never iterated on.
- **Output:** snake_case YAML in `recipes/drafts/<id>.yaml`, with a header naming the API kind and model
  but **never the endpoint URL** (it can be private). Drafts pass the same gates as any recipe
  (≥ 2 fixtures, `rex test`, review) and publish to `dev` only.
- **First use:** `go.dev/blog` was drafted from a captured page (one attempt, status OK), reviewed (author
  and date rules added), and given 2 redacted fixtures and a liveness URL.

## U29: CI [delegated], 2026-10-13

`.github/workflows/`:

- **`ci.yml`** (push and PR) has four jobs:
  - fmt;
  - clippy `-D warnings`, tests, `rex check` (≥ 2 fixtures, version gate against the PR base or the
    previous push), `rex test`, and the fixture lint;
  - wasm: size budget and conformance under Node (every case byte-identical to native);
  - extension: typecheck, a store build checked for leaks (`<all_urls>`, `rexDebug`) and zipped, both
    e2e suites in Chromium, and screenshots uploaded.
- **`publish.yml`** (main): stable from `recipes/generic` and `recipes/sites`, dev from all of `recipes/`;
  client-side verification by public URL; weekly GC. It stays off until `REX_PUBLISH_ENABLED=true` and the
  R2 secrets exist.
- **`liveness.yml`:** see U20.

The e2e suites are **hermetic**. Bundles are published by the real CLI with a throwaway key and served
to the extension's own service-worker fetches through Playwright routing
(`PW_EXPERIMENTAL_SERVICE_WORKER_NETWORK_EVENTS=1`), so CI needs no server and no network.
`tests/e2e-sync.mjs` forges every rejection path (bad signature, unpinned key, edited payload, rollback,
recipe and schema hash mismatch, wrong channel, client-version gate, unsupported `requires`, re-keying,
invalid settings) and requires the previous bundle to survive each one. `scripts/ci-local.sh` runs the
same gates locally.

## U30: Operations [delegated], 2026-10-13

Hosting setup, key rotation and GC are documented in [operations.md](operations.md). Key rotation uses
two pinned keys in the extension (old and new) for one release cycle. `publish --trust old_id:hex` lets the
new key take over the live index.

## U31: Batch harvesting of surveyed sites [user decisions + delegated], 2026-10-13

The user asked for every site of the survey (4340 candidates from Tranco, Majestic and Chinaz rankings;
`tools/harvest/`, survey scripts and data in `~/projects/scratch/rex-site-survey`) to get a recipe. The
user chose:
- fixtures in a separate repo, `Shuozeli/rex-fixtures`, gzip-compressed;
- automatic quality gates, publishing to `dev` first and promoting to `stable` after 7 days of liveness;
- up to 2 repair rounds;
- articles and feeds first, while the reference/product/video/thread schemas are designed with the user.

**Pipeline** (`tools/harvest/harvest.py`), per site:

1. Capture the homepage. It becomes the negative page: an article recipe must not match it.
2. Find article URLs, in this order:
   - RSS items;
   - homepage links on the site or any subdomain, with articles often on one subdomain (`blog.csdn.net`),
     so the host with the most article links is targeted;
   - sitemaps from robots.txt, when the first two give fewer than 3.
3. Group the URLs by shape (depth plus first segment; one-segment slugs form one group). Try the groups
   from largest to smallest, and keep the first one that yields at least 2 article-like pages.
   - A page is article-like if it has a schema.org article type, `og:type=article`, or a publish date
     (date markup, or a date and time in the text). Product and topic pages are big groups too.
   - Pages with enough readable text are preferred.
4. Try a **thin recipe**: `extends: generic/article.v1`, the host, and the page's own article type (or
   og:type) as the page condition when the homepage lacks it.
5. Then try a **URL-shape thin recipe**: an `include` regex computed from the sample URLs.
   - Equal segments stay literal, numbers become `\d+`, and other segments become `[^/]+` with any shared
     extension kept.
   - At least one constraining segment is required.
6. Otherwise **draft** with `rex draft`.
   - The URL rules are fixed by the tool; the model writes only page conditions and fields.
   - The evidence pack leads with a focused section: the readability scorer's main container (path and
     subtree), the headlines, and byline/date candidates. The whole-page skeleton cut off before the article
     body on big sites.
   - Gate problems go back to the model for up to 2 repair rounds. For endpoints that cannot handle system
     roles or multi-turn requests, the conversation is flattened into one message.
7. Every recipe must pass **`rex gate`** (`crates/rex-cli/src/gate.rs`):
   - every sample page is matched and chosen, and has status OK;
   - the title agrees with og:title, the JSON-LD headline or `<title>` (token overlap ≥ 0.6);
   - the body is at least 300 characters, and agrees with readability at ≥ 50% when reported;
   - the homepage is not matched.
   - Pages whose readable text is under 600 characters (paywall teasers, video or live pages) are stubs: they
     must match, but their body is not judged, and at least 2 full pages are required.
   - For feeds: at least 5 items, and at least 80% of them with a title and a URL.

**Pilot** (50 P1 article sites): 22 of 46 new sites accepted, 4 already covered.
- Skips are mostly bot walls or TLS resets (need a real browser; the ARM2 CDP endpoint is down) and sites
  with no article links.
- Product-page sites (Zol, PConline) are rejected by the article-like requirement.

**Repo layout:**
- **Recipes:** `recipes/auto/<id>.yaml`, published to `dev` only, since stable takes `recipes/generic`
  and `recipes/sites`.
- **Fixtures:** 2 fixtures per recipe in rex-fixtures as `page.html.gz` (redacted), about 75 KB per page.
- **Liveness:** one live URL each in `liveness-auto.yaml`.
- **Promotion** to `recipes/sites/` happens after 7 days of passing liveness.
- **Tooling:**
  - `rex check --fixtures` takes several roots;
  - fixtures may be gzip-compressed (`rex-engine` feature `gz`, off in wasm);
  - `fixture redact --gzip`;
  - captures decode the page charset (GBK/GB2312 sites).

This amends U28: the batch flow allows the gate-driven repair loop that U28 excluded for single drafts.

## U32: Category schemas; search, commerce, media [user decisions + delegated], 2026-10-14

The user chose to classify sites by category and give each category one generic schema. The first three
were requested together, with the field sketches accepted as-is:

| Schema | Messages | Pages | Sites (recipes) |
|---|---|---|---|
| `rex.schema.search.v1` | SearchResults, SearchResult, Sitelink, ResultType | search result pages | Google (`google.com/search`), Bing (`bing.com/search`); DuckDuckGo later |
| `rex.schema.commerce.v1` | Product, Spec, Variant, ProductList, ProductItem, Availability | product detail, search/category lists | Amazon (`amazon.com/product`, `amazon.com/search`) |
| `rex.schema.media.v1` | Video, Channel, Chapter, VideoList, VideoItem | watch pages, search/channel lists | YouTube (`youtube.com/watch`, `youtube.com/search`) |

- **Generic bases** `generic/product.v1` and `generic/video.v1` read schema.org data (JSON-LD, microdata,
  Open Graph), so any page with Product or VideoObject markup works without a site recipe. On a YouTube
  watch page, `generic/video.v1` alone gives the full record.
- **Spec tables are typed rows** (`Spec {name, value}`), not a map (U1 still holds): the names are not a
  fixed vocabulary.
- **Engine additions** (additive within `rex.extract.v1`):
  - `ParseDuration` (`duration: {}`): ISO 8601 and clock durations to seconds;
  - `Decode` (`decode: { kind }`): percent, base64 or base64url, to unwrap redirect links (Bing
    `u=a1<base64url>`, Google `/url?q=`);
  - `FieldRule.each_index`: an integer item field that receives the 1-based position among kept items
    (search rank, list position);
  - compact relative dates ("3y ago", "2mo ago", "4h ago", "3个月前", "1年前"; months 30 days, years 365),
    truncated to whole seconds.
- **Redaction keep-list** gains shop and search UI labels: in, out, of, stock, currently, (un)available,
  rating(s), result(s). Recipes parse them ("In Stock", "4.6 out of 5 stars"); they are UI text, not
  publisher prose.
- **Fixtures of rendered pages** come from `extension/scripts/capture.mjs`. It loads the page in Chromium
  and serializes the DOM exactly like the extension, for YouTube lists and other JavaScript-built pages.
  Large fixture pages are stored as `page.html.gz` in the main repo too.
- **Findings:**
  - Amazon's visible price digits sit inside an `aria-hidden` span, so the recipe selects that span.
  - YouTube's rendered page sometimes lacks its microdata, so recipes read the visible metadata as well,
    with exact microdata values first.
  - YouTube nests buttons in ways that html5ever and Chrome repair differently on re-parse. Selectors avoid
    depending on those ancestors; the e2e parity check caught this.
  - Google answers fresh automated browsers from this network with an "unusual traffic" captcha. Its fixtures
    were captured with `capture.mjs --cdp` from the long-lived Chrome profile on the Alienware Windows host:
    a new tab is opened and closed, and other tabs are untouched. Google gets no automated liveness
    (scheduled queries are against its terms).

## U33: Social posts and threads (`rex.schema.social.v1`) [delegated], 2026-10-14

This is the first item of the user-approved order (social → Chinese platforms → schema.org bases →
quotes → tables and lists). The schema was designed without a unit-by-unit review, at the user's request
not to be interrupted; it is open to revision.

- **Thread** is one discussion page. Its `post` is the tweet, question or link post. Its `replies` are
  comments or answers in page order, flattened, with `depth` and `parent_id` keeping the nesting. It also
  carries `title`, `community`, `url`, `reply_count` (the page's total) and `tags`.
- **Post** has:
  - id, author, text/html, published_at;
  - likes, reposts, replies, views;
  - media, url (the permalink), and `link` (the URL a link post shares);
  - depth, parent_id, and `accepted` (Stack Overflow).
- **Author** has name, handle, url and verified. **PostList** is a timeline (profile or community feed),
  with an owner and posts.
- **Recipes**, 2 fixtures each:

  | Recipe | Schema | Captured with | Liveness |
  |---|---|---|---|
  | `news.ycombinator.com/item` | Thread | plain HTTP | daily |
  | `stackoverflow.com/question` (+ Stack Exchange sites) | Thread | CDP | — |
  | `reddit.com/thread` | Thread | CDP | — |
  | `reddit.com/community` | PostList | CDP | — |
  | `x.com/status` | Thread | CDP, logged in | — |
  | `x.com/profile` | PostList | CDP, logged in | — |
  | `zhihu.com/question` | Thread | CDP | — |

  - The CDP captures used the Alienware Chrome profile; Stack Overflow answers plain fetches with 403, and
    X needs a login.
  - The sites without daily liveness need a browser and are covered by fixtures and the e2e.
- **Site notes:**
  - Reddit's data sits in `<shreddit-post>`/`<shreddit-comment>` attributes.
  - The focal X post is the tweet article with `tabindex="-1"`, and its metrics come from the action bar's
    aria-label.
  - Zhihu answers carry microdata.
  - A Stack Overflow answer's author is the last user link (editors come first).
- **Redaction keep-list** gains repost(s), retweet(s), bookmark(s) and quote(s), the metric labels that
  recipes parse.
- **Build fix:** `rex-proto` now reruns its build script when a proto directory changes, so new schema
  packages are picked up.

## U34: Chinese platforms on the existing schemas [delegated], 2026-10-14

This is the second item of the approved order. No new schema was needed: every page maps onto an existing
category schema.

- **Recipes**, 2 fixtures each:

  | Recipe | Schema | Captured with | Liveness |
  |---|---|---|---|
  | `mp.weixin.qq.com/article` | Article (extends `generic/article.v1`) | CDP | — |
  | `bilibili.com/video` | Video (extends `generic/video.v1`) | CDP | daily |
  | `bilibili.com/search` | VideoList | CDP | daily |
  | `douyin.com/video` | Video (extends `generic/video.v1`) | CDP | — |
  | `baidu.com/search` | SearchResults | CDP | daily |

  - The captures used the Alienware Chrome profile, the same as U33.
  - WeChat answers plain fetches from servers with a verification page, and Douyin renders client-side.
    Both are covered by fixtures and the e2e only.
- **Site notes:**
  - **WeChat:**
    - The publish time comes from the inline `var ct = "<epoch>"`. The visible `#publish_time` is
      localized ("Oct 14, 2026, 11:15 AM" / "2026年10月14日 11:15") and is the fallback.
    - `og:description` carries JavaScript escapes (`\x26quot;`).
    - Links reached through Sogou search are signed and temporary. They become the canonical URL because
      the page has no `og:url`.
  - **Bilibili:**
    - The video page's JSON-LD already fills most fields through the generic recipe. The site recipe adds
      the BV id, likes and the visible counters.
    - In search, ad cards have no `/video/BV` link and drop out because the id is required. Dates are
      absolute ("2020年10月21日"), this-year ("7月13日") or relative ("14小时前"), so the recipe requires
      `relative_dates`.
  - **Douyin:**
    - There is no view count and no duration in the DOM. The title doubles as the description, after the
      "第N集 |" series prefix is stripped.
    - Tags are the page keywords minus Douyin's own site words.
  - **Baidu:**
    - Organic results are `#content_left > div.result`. Baidu's own cards (`div.result-op`: AI answers,
      notes, products) are skipped.
    - The visible link is a non-decodable `baidu.com/link` redirect, so the destination comes from the
      result's `mu` attribute.
    - A leading date in the snippet becomes `published_at`.
- **Engine fix:** with a `tz` configured, the date preclean stripped a trailing "AM"/"PM" as if it were a
  zone abbreviation, so `%p` formats never matched. It now leaves the meridiem alone (test
  `meridiem_survives_zone_abbreviation_stripping`).
- **Redaction fix:** `data-*` attribute values without spaces were kept as tokens, but CJK prose has no
  spaces. Bilibili's `data-title="<the video title>"` leaked into the redacted fixture. Values with CJK
  characters are now redacted, and the lint flags them.
  - 80 committed fixtures had such leaks: 10 in this repo and 70 in rex-fixtures, mostly Japanese and
    Chinese sites.
  - They were re-redacted in place (redaction is idempotent). Their expected outputs did not change.
- **E2E harness fixes:**
  - **Hermetic pages:** each fixture page now gets only its own HTML, and every other request is aborted.
    Before, the external scripts loaded live: Bilibili's re-rendered the page from the network, and
    Douyin's held up the load event.
  - **Pinned clock:** the e2e extraction resolves relative dates against the fixture's `fetch_time`, as
    native `rex test` does, through an e2e-only `nowMs` argument to `rexDebug.extractTab`.
- **Deferred: JD** (`item.jd.com`).
  - The desktop product page requires a login, and the mobile page masks prices ("1??9").
  - It needs a logged-in JD session in a capture browser, then a `jd.com/product` recipe on Product.

## U35: US platforms on the existing schemas [delegated], 2026-10-14

The user asked to continue with US sites after the Chinese platforms (U34). Like U34, every page maps onto
an existing category schema; schema.org-type sites (IMDb, Yelp, Zillow, Indeed, app stores) wait for the
schema.org bases step.

- **Recipes**, 2 fixtures each:

  | Recipe | Schema | Liveness | Notes |
  |---|---|---|---|
  | `walmart.com/product` | Product | daily | no Product JSON-LD; itemprop price, buy-box seller, variant tiles |
  | `walmart.com/search` | ProductList | daily | sponsored tiles unwrap the /sp/track `rd=` redirect |
  | `ebay.com/item` | Product | — | JSON-LD + item number, seller, list price, item specifics |
  | `ebay.com/search` | ProductList | — | "Shop on eBay" placeholder dropped; obfuscated "Sponsored" not read |
  | `target.com/product` | Product | daily | price API never answers the capture browser, so price is optional |
  | `target.com/search` | ProductList | — | cards carry prices; sponsored from the link aria-label |
  | `bestbuy.com/search` | ProductList | — | product pages reset automated navigation (HTTP/2 error) |
  | `tiktok.com/video` | Video | — | logged-out feed: the focal video is the first card |
  | `bsky.app/post` | Thread | — | JSON-LD for the focal post; replies from the DOM |
  | `threads.com/post` | Thread | — | focal post under pagelet `threads_post_page_1` |
  | `quora.com/question` | Thread | — | the question is the post, answers are replies |
  | `duckduckgo.com/search` | SearchResults | — | direct links, sitelinks |
  | `search.yahoo.com/search` | SearchResults | — | snippet date becomes published_at |
  | `medium.com/article` | Article | daily | header and byline kept out of the body |

  - The captures used the Alienware Chrome profile, logged out.
  - Substack posts need no site recipe: generic/article.v1 already extracts them cleanly.
- **Site notes:**
  - **Bluesky and Threads** render with generated class names, so the recipes use `data-testid`,
    `data-pagelet`, aria-labels and `:has()` (supported by the engine's selector library).
  - **Viewer-local times:** Bluesky's DOM shows times in the viewer's local zone without naming it. The
    focal post's time comes from JSON-LD (UTC), and replies carry no `published_at` rather than a time
    that would be silently wrong.
  - **Threads:** `time@datetime` is exact UTC.
- **Deferred:**
  - Vimeo: Cloudflare Turnstile asks for a human click, and rex does not automate challenges.
  - Etsy: DataDome captcha.
  - Best Buy product pages: see the table.
  - LinkedIn, Instagram and Facebook: login walls.
  - Profile/timeline pages (TikTok, Bluesky, Threads): PostList/VideoList recipes are a follow-up.
- **Engine:** a month and day without a year ("Sep 23", "Sept. 23", "7月13日") now resolves, under
  `relative`, to its latest occurrence on or before today. Two Bilibili search expectations gained dates
  (test `month_day_without_year_is_the_latest_past_occurrence`).
- **Redaction:**
  - Units glued to numbers survive ("1.7K", "9.5M", "3d", "2mo", "11y"); before, the unit letter was
    redacted and compact counts and ages no longer parsed.
  - The keep-list gains the US shop and social labels recipes parse: "Average customer rating is … with …
    reviews", "Add to cart", "Sponsored", "Answer (1 of N)", "Updated", "Verified", "verifications",
    "profile".

## U36: schema.org bases: how-to, FAQ, event, business, job, app, film/TV, book [delegated], 2026-10-14

This is the third item of the approved order. The schemas were designed without a unit-by-unit review, as
U33-U35 were, and are open to revision.

- **New schema packages**, each with a generic base recipe driven by JSON-LD, then microdata, then meta:

  | Package | Records | Generic recipe | Maps from schema.org |
  |---|---|---|---|
  | `howto.v1` | Recipe, HowTo, Step | `generic/recipe.v1`, `generic/howto.v1` | Recipe; HowTo |
  | `faq.v1` | FaqPage, Faq | `generic/faq.v1` | FAQPage |
  | `event.v1` | Event, Venue, Ticket | `generic/event.v1` | Event and 17 subtypes |
  | `place.v1` | Business | `generic/business.v1` | LocalBusiness and about 40 specific subtypes |
  | `job.v1` | JobPosting, Salary | `generic/job.v1` | JobPosting |
  | `app.v1` | App | `generic/app.v1` | SoftwareApplication, MobileApplication, WebApplication, VideoGame |
  | `creative.v1` | Title, Book | `generic/title.v1`, `generic/book.v1` | Movie, TVSeries, TVEpisode, ...; Book |
  | `common.v1` | Person, Organization, PostalAddress, GeoPoint | — | shared by the packages above |

  - Bare `Organization` and `Place` map to nothing: almost every site declares them as the publisher.
  - Durations become whole seconds through the `duration` transform.
  - Dates of films and books are dates (`VALUE_KIND_DATE`), not timestamps.
  - Ratings carry `rating_scale` (bestRating), because IMDb rates out of 10 and Rotten Tomatoes out of 100.
- **Fallback ranking (amends U2 and the locked U3 annotations; additive).**
  - `RecordOptions.fallback_rank` is new. When a page declares several types that map to schemas with a
    base recipe, the highest rank wins; ties keep the old document order.
  - Specific entities rank 20: Recipe, HowTo, Event, Business, JobPosting, App, Title, Book and Product.
    FaqPage ranks -10. Article and Video stay at 0.
  - Why: BBC Good Food and wikiHow put a hero `VideoObject` first, so the old first-type rule picked
    generic/video for a recipe page. FAQ blocks also ride along on event pages and how-tos.
  - All 2044 existing cases are unchanged.
- **media.v1 Video no longer claims `Movie`.** A film's page is about the work (creative.v1 Title); Video is
  for watch pages.
- **Value handling:**
  - A date-only event end ("2026-10-24") is left out, because it cannot be placed without a zone.
  - The JSON-LD job description is HTML, so `description_text` goes through `html_to_text`.
  - IMDb's JSON-LD keeps HTML entities, so the title description is decoded the same way.
- **Fixtures**, 2 or more per recipe, all plain HTTP except IMDb and Allrecipes (Alienware CDP):

  | Recipe | Fixtures |
  |---|---|
  | `generic/recipe.v1` | BBC Good Food, Allrecipes |
  | `generic/howto.v1` | wikiHow ×2 |
  | `generic/faq.v1` | Stripe pricing, Bankrate |
  | `generic/event.v1` | Eventbrite ×2 |
  | `generic/business.v1` | Michelin Guide, Tripadvisor |
  | `generic/job.v1` | Lever ×2 |
  | `generic/app.v1` | App Store, Google Play, Zapier |
  | `generic/title.v1` | IMDb film and series, Rotten Tomatoes |
  | `generic/book.v1` | Google Books ×2 |

  - **Site recipe:** only `goodreads.com/book`, because Goodreads truncates og:description and keeps genres
    and the first-published date out of its JSON-LD. The generic results elsewhere were already complete.
- **Not covered:**
  - Yelp, Etsy-style DataDome captchas, and ThriftBooks (one Book node per edition offer, none for the work).
  - Greenhouse's new job boards, which have no JobPosting JSON-LD (a site recipe is a follow-up).
  - Liveness skips event and job pages because they expire.

## U37: Market quotes (`rex.schema.finance.v1`) [delegated], 2026-10-15

This is the fourth item of the approved order. The schema was designed without a unit-by-unit review, as
U33-U36 were, and is open to revision.

- **One record, `Quote`, covers stocks, ETFs, mutual funds, indexes and crypto:**
  - Identity: symbol, name, instrument type, exchange, currency.
  - Price: price, change, change_percent, as_of, market_state.
  - The usual statistics: previous close, open, day and 52-week range, volume, average volume, market cap,
    P/E, EPS, dividend yield.
  - Fund fields: nav_date, expense_ratio, net_assets. Crypto fields: circulating_supply, rank.
  - `stats`: the page's summary table as labelled, so nothing shown is lost.
  - Values are as displayed, possibly delayed, and never converted. Percentages are percent points.
  - Volume is in shares. A-share sites show 手 (lots of 100 shares) and are converted.
- **Recipes** (`generic/quote.v1` reads FinancialQuote JSON-LD):

  | Recipe | Covers | Fixtures | Liveness |
  |---|---|---|---|
  | `finance.yahoo.com/quote` | stock, ETF, fund, crypto, index, FX, futures | AAPL, SPY, VFIAX, BTC-USD | daily |
  | `cnbc.com/quote` (extends generic) | stock, index | AAPL, .SPX | daily |
  | `coinmarketcap.com/coin` | crypto | BTC, ETH | daily |
  | `quote.eastmoney.com/stock` | A-shares | 600519, 000001 | — |
  | `fund.eastmoney.com/fund` (天天基金) | Chinese open-end funds | 110022, 161725 | daily |
  | `xueqiu.com/stock` | A-shares, HK, US | SH600519, TSLA | — |

  - Eastmoney stock pages and Xueqiu render client-side, so they are covered by fixtures and the e2e.
- **Site notes:**
  - Yahoo's live values sit in `fin-streamer[data-field]`. Its EPS row's streamer is mislabelled
    `trailingPE`, so P/E and EPS go by row label.
  - The instrument type comes from the symbol form (`-USD`, `=X`, `=F`, `^`) or the expense-ratio label
    ("Expense Ratio (net)" means an ETF).
  - Yahoo's time notice has no date, so `as_of` is left out; the notice still gives the market state.
  - CNBC labels are plain text, so rows are picked by matching their text.
  - CoinMarketCap gives the 24h change unsigned, with the direction in `data-change`.
  - Eastmoney shows "-" before the 09:30 open, and the price then drops out.
- **Engine:** numbers take "T" (trillion) and 万亿 (10^12) suffixes (test `magnitude_suffixes_scale_numbers`).
- **Redaction:**
  - The keep-list gains quote-row labels: English (prev/previous, close, open, high, low, volume, market,
    cap, P/E, EPS, TTM, dividend, yield, expense, ratio, net, assets, circulating, supply, after hours, at,
    as, by, overnight, pre, cryptocurrency) and the label characters of 今开 / 昨收 / 成交量 / 总市值 /
    市盈率 / 单位净值 / 规模 / 货币单位 / 交易中 / 已收盘.
  - `slot` is kept as a structural attribute.
  - Every new fixture keeps every extracted field after redaction.
- **Capture:** the Alienware host went offline, so the captures used the ARM Windows Chrome (port 9403/9405;
  9401 has 63 leaked blank tabs).
- **Deferred:**
  - Google Finance: a redesign in beta with no stable hooks.
  - Morningstar: generated classes only.
  - CoinGecko: answers plain fetches with 403.

## U38: Live-page test in a real Chrome, and what it found [user decision + delegated], 2026-10-15

**User decision:** test the extension on live pages in a real browser on the ARM Windows host, rather than
only against fixtures in a local headless Chrome.

- **Set-up** (operations.md, "Live-page test in a real Chrome"):
  - Chrome for Testing 155 runs in its own profile on the ARM Windows host (residential network), with the e2e
    build of the extension loaded. It syncs the real dev bundle from serve-lib over the tailnet.
  - A scheduled task starts it; CDP sits on 127.0.0.1:9530 behind an SSH tunnel.
  - The 15 Chrome instances that host runs for other jobs are left alone.
- **The runner:** `extension/tests/live-remote.mjs` covers one case per site and generic recipe
  (`extension/tests/live-cases.json`).
  - It speaks raw CDP to the extension's service worker only. The worker opens the live page in a tab, waits
    for it to load and settle, and extracts with the extension's own code path (serialize in the tab, wasm
    in the worker).
  - It checks the matched recipe, the status, the extraction time, and which fields of the fixture's
    expected record the live record lacks. Values differ on live pages by design.
  - A first version drove pages through Playwright, which attached a debugger to every page. It stalled on
    Yahoo Finance's streaming page and left the browser refusing a second connection.
- **Findings that fixtures could not show, now fixed:**
  - **App Store:** after the page renders, its SoftwareApplication JSON-LD and Open Graph tags are gone. The
    plain-HTTP fixtures had them, so the extension got nothing. New site recipe `apps.apple.com/app`
    (extends generic/app.v1) reads the DOM. The plain-HTTP fixture moved under it, joined by two rendered
    captures.
  - **BBC:** browsers get a new design (data-component layout blocks) while plain HTTP still gets the
    data-block design. The body expectation failed live; `bbc.co.uk/news-article` v2 reads both.
  - **YouTube search:** the duration badge is now plain text in the overlay. `youtube.com/search` v2 reads
    it.
  - **Target product:** the ratings block loads lazily below the fold; the top summary always has the
    rating. `target.com/product` v2 reads the summary.
  - **Extension:** on pages that swap their document after load (Bing's redirect, NPR, GitHub, Xueqiu),
    Chrome dropped the injected serializer ("Frame with ID 0 was removed"), and a user's click would have
    shown an error. `readTab` now retries once on that error.
  - Each recipe fix comes with a fixture captured from the live run.
- **Findings that are the environment, not recipes:**
  - X needs a logged-in session.
  - Bot checks shown to this fresh profile on this IP: WeChat's verification page, Baidu's 百度安全验证,
    Medium's Cloudflare challenge, Douyin, Target's search API, and eBay's rate limit at times.
  - Best Buy prices arrive late; some runs have them.
  - Extraction time on a real click is mostly 0.1-0.7 s. Heavy pages take longer: Yahoo Finance 1.7-2 s,
    IMDb 2.5 s, CNBC 3.4-5.3 s. That is a performance item.
- **Result after the fixes** (61 live pages, latest valid run per site, 2026-10-15): 51 OK, 4 partial,
  5 wrong recipe, 1 error.
  - Wrong recipe, the environment rather than the recipes: X ×2 (login), WeChat (verification), Douyin (the
    page did not finish loading for this profile), eBay search (rate-limit error page).
  - Partial: Best Buy prices load late. YouTube search durations render lazily (present in one run). CNBC
    has no market state for its ambiguous "Last |" label, by design. The generic article case compares
    Substack with a synthetic fixture.
  - Error: Ars Technica, intermittently ("Frame removed" twice; it passed in the previous full run).
  - The runner re-attaches when Chrome restarts the extension's service worker. Every step in the worker
    (load, read, extract) has its own timeout, so one slow page cannot stall the run.
- **Rule from this:** a site whose page is rendered client-side gets fixtures captured in a browser (CDP),
  not over plain HTTP. When a live run disagrees with a fixture, the live page becomes a new fixture.

## U39: Code-review fixes [user decision + delegated], 2026-10-19

**User decision:** fix every finding of the code review, in the suggested order: engine hardening, redaction,
publish gate, anti-rollback, then the medium and low findings.

- **Engine hardening** (hostile pages and recipes must cost bounded time and memory):
  - Pipe transforms are charged per item and per byte, and lists stop at `MAX_PIPE_ITEMS` (10 000);
    `split` and `unique` stay linear.
  - JSON-LD `@id` fan-out, microdata paths and repeated fields are charged against the shared budget and capped.
  - `max_output_bytes` is enforced; candidate collection stops at 4x it.
  - Pages nested deeper than `max_dom_depth` (256) are pruned at parse time instead of overflowing the stack.
  - Fallback lookup by schema.org type is O(1) per type; each recipe's match stage gets its own fuel.
  - Values: checked arithmetic in relative dates and durations (and clock fields below 60); numbers need
    real 3-digit grouping, and K/M/B suffixes must touch the digits; month names must be exact or 3-letter;
    a wall time in a DST gap moves forward; Feb 29 without a year looks back to a leap year.
  - Recipe validation: defaults are coerced at compile time; microdata paths are checked; `relative` needs
    `requires: [relative_dates]`; `replace_inherited` also replaces `default`, `each` and nested specs.
  - Readability keeps `<form>` content when the form holds the article (3+ paragraphs, ASP.NET pages).
  - Schema validation detects self-referencing records instead of recursing forever.
- **Redaction:**
  - URLs in text, JSON and URL attributes are redacted per component (prose-shaped path segments and query
    values); a URL token ends at the first non-URL character.
  - JSON keeps only `@type`, `@context`, `@language`, `@vocab` and token-shaped `@id`.
  - Lint flags non-empty comments and `<style>` text.
  - All 2077 captured fixtures in both repos were re-redacted; 163 expected outputs changed (values only).
- **Publish gate:**
  - The publish workflow runs only after CI passed on a push to main, on that commit.
  - It gates on both fixture repos.
  - `rex check --git-base` fails on a bad ref or outside a work tree instead of skipping the version gate.
- **Anti-rollback:**
  - The extension keeps the highest accepted sequence per (base URL, channel), apart from the stored bundle.
  - Changing the pinned keys or re-adding a source forgets the bundle but never lowers that floor.
  - The stored bundle records the engine's features; a different engine re-selects the entries (no
    conditional GET), so entries skipped for an unsupported feature come back after an update.
  - operations.md now states what a key compromise really means.
- **Extension:**
  - The engine cache key covers the source identity (id, channel, base URL, keys), sequence and schema set.
  - Engines are leased: one is freed only when no extraction holds it.
  - A local developer recipe that parses but does not compile falls back to the bundle. The options page
    shows the compile error.
  - `parseKeys` keeps a line without `id:` as an invalid key, so validation reports it.
  - The first sync fetches recipe objects 8 at a time, in index order.
- **CLI:**
  - `bundle publish` reads every object back before writing the index, and re-uploads any that is missing
    or corrupt, also when nothing changed, so a half-published channel heals on the next run.
  - `bundle gc` refuses a retention shorter than the index cache lifetime (300 s).
  - `bundle build` refuses to continue from an index it cannot verify; it no longer restarts at sequence 1.
  - `bundle keygen` creates the key file exclusively with mode 0600.
  - Gate:
    - Title tokens cover every script.
    - Site-name tokens (og:site_name, application-name, JSON-LD publisher) do not count toward the title
      overlap.
    - A title that is only the site name fails.
  - `rex draft`:
    - `--force` replaces the old draft.
    - Only reviewed recipes (not drafts or auto recipes) are shown to the model as examples.
    - "Too few full pages" no longer spends repair rounds.
    - When repairs stop compiling, the last compiled recipe is kept.
    - LLM errors never name the endpoint, its host or the API key.
  - Fetching: the `charset` parameter is case-insensitive; an unknown label falls back to the page's
    `<meta>`.
- **Tools:**
  - `import_results.py` imports each recipe all-or-nothing and parses `rex test` lines by case path, not by
    splitting on `/`.
  - `live-remote.mjs` reattaches when Chrome replaced the worker.
  - `classify.py` retries UNLABELED rows.
- **Verification (2026-10-19):**
  - `scripts/ci-local.sh` passes: 107 workspace tests, 2085 fixture cases, wasm parity 143/143, both
    extension e2e suites.
  - Both dev channels republished as a no-op (no recipe changed); the new read-back found every object intact.
  - Live run on the ARM Chrome: 46/61 OK in the full run. Five of the six runner errors passed on a rerun
    (slow page reads, documents replaced mid-read); Ars Technica still drops its frame.
  - YouTube search lacks durations: the thumbnail overlays had not rendered, and most results were mixes.
  - The rest are login and bot walls, as in U38.

## U40: Recorded e2e runs on a Docker Ubuntu desktop [user decision + delegated], 2026-10-23

**User decision:** after the Windows recording worked, build a framework for recorded e2e runs, starting with a
Docker Ubuntu GUI.

- **Windows first** (CDP screencast, U38's ARM Chrome). It proved the flow but not the capture:
  - That Chrome runs in session 0, with no visible desktop, so only page and popup frames could be captured.
    The browser frame and cursor were drawn afterwards.
  - x64 emulation on ARM gives a frame every 30-50 ms, and the browser had to restart between scenes.
- **The Docker desktop** (`extension/recorder/`, operations.md):
  - Xvfb + openbox + a browser with the extension pinned (first Chrome for Testing 155, then Google Chrome; see
    below).
  - `xdotool` drives real input; ffmpeg x11grab records the screen at 50 fps (a frame every 20 ms).
  - CDP only observes.
  - Scenarios are JSON: scenes with a URL, a label and an `expect`.
- **First run** of `scenarios/tutorial.json`: 5/7 scenes OK (Wikipedia, HN, Bilibili, Xueqiu, YouTube).
  Amazon and Zhihu sent this host's IP to their bot checks; the same pages pass from the residential IP (U38).
- **Recipe fix found by the Windows recording:** this Chrome was sometimes redirected to the English Bilibili site
  (`/en/video/...`). `bilibili.com/video` v2 matches it and reads the id from the URL by regex, with a new
  fixture `anji-en-site`.
- **User decision (same day): Google Chrome stable, not Chrome for Testing.**
  - Branded Chrome ignores `--load-extension`, so the container installs the extension the way an
    organization deploys it: a CRX packed from `extension/dist` (key outside the repo), a local update server,
    and `ExtensionSettings` (`force_installed`, `toolbar_pin: force_pinned`).
  - `DeveloperToolsAvailability: 1` lets CDP attach to the policy-installed extension.
- **The runner reads results from the popup,** so it tests the store build through its UI.
  - The store build has no `<all_urls>`, so the real toolbar click grants `activeTab`, and that path is now
    covered (the tasks.md gap).
  - With the e2e build, the extraction behind the popup is also checked against what it shows.
- **Result:** the tutorial scenario passes 6/7 on Chrome 156 with the store build. Zhihu passes in branded
  Chrome; Chrome for Testing was sent to `account/unhuman`. Amazon still shows this host's IP an interstitial.
- **User decision (2026-10-24): a docker e2e.** `scripts/docker-e2e.sh` runs the store build in Google Chrome
  in a container with no network.
  - It serves fixture pages at their real URLs and a bundle published from `recipes/` with a throwaway key,
    over TLS from a local CA.
  - Each scene must match its fixture's `expected.json`: the recipe and version, the status and the field
    count.
  - First run: 7/7 (Wikipedia, Amazon, HN, Bilibili, Zhihu, Xueqiu, YouTube), about 3.5 min without the video.
  - It is an opt-in last step of `ci-local.sh` (`REX_DOCKER_E2E=1`).
- **User decision (2026-10-24): run it in GitHub Actions.** The `docker-e2e` job in `ci.yml`:
  - builds the image with a layer cache, refreshing Chrome weekly;
  - runs the script on the `rust` and `wasm` artifacts;
  - uploads the results and the video.
  - Simulated locally with the same steps: 7/7 with the video, 4.6 min. actionlint is clean.
  - **First GitHub runs (2026-10-24):** run 40144649915 is all green. Format 19 s, rust 6 m 48 s,
    wasm 1 m 8 s, extension 1 m 49 s, docker-e2e 5 m 41 s (9.5 min on the first, uncached image build).
    docker-e2e matched expected.json on 7/7, the same as locally.
    The first runs found three problems:
    1. Clippy on the runner's Rust 1.99 flags `question_mark` (fixed in values.rs).
    2. After a force push, `github.event.before` is gone, and `rex check --git-base` rightly refused it;
       the workflow now falls back to the parent.
    3. The extension job used playwright-core 1.48's Chromium 130, below the manifest's
       `minimum_chrome_version` 137, so the extension never loaded; local runs had used another
       project's cached Chromium. playwright-core is pinned to 1.62.1 (Chromium 151), and the harness takes
       Playwright's own Chromium path.
  - Fixtures repo: `Shuozeli/rex-fixtures` is now a private GitHub repo. CI reads it with a read-only
    deploy key (operations.md).
- **User decision (2026-10-25): a fixture scene for every site recipe.**
  - `scenarios/sites.json` expands `{"siteFixtures": "recipes/sites"}` to each recipe's first captured
    fixture (52 scenes now; new recipes are picked up without editing the scenario). wsj.com has only
    synthetic fixtures, so it uses one of them.
  - It runs the quick flow, checks only. The tutorial scenario switched HN to the captured `live-front`,
    because one URL may serve only one fixture.
  - Local run: 52/52 and 7/7. Site scenes take 7.6 s on average; the popup shows its result after a median
    of 0.9 s.
  - In CI, docker-e2e is a matrix over the two scenarios, run in parallel.
- **User decision (2026-10-25): the generic (schema.org fallback) recipes as scenes too.**
  - `scenarios/generic.json` covers all 13 generic recipes. The expansion key is now `recipeFixtures`
    (`siteFixtures` still works), and a recipe without a fixture is an error.
  - Three generic recipes had no fixture of their own; they had only served as bases for site recipes.
  - New fixtures for them:
    - **Product:** allbirds and kyliecosmetics product pages (Product JSON-LD, no site recipe).
    - **Video:** two TED talks (VideoObject). A C-SPAN page was tried and dropped: it redirects to a program
      page, where the fallback rightly picks `generic/article.v1`.
    - **Quote:** two synthetic pages (`synthetic: true`, example.com/example.org). One has a plain
      `FinancialQuote` with numeric prices; the other the CNBC-style `Intangible/FinancialQuote` inside
      `@graph`, with string prices. A search of quote sites found only CNBC publishing FinancialQuote, and
      CNBC has its own recipe, so no real page reaches this fallback.
  - Local run: 13/13. CI has a third matrix entry.
- **User request (2026-11-03): a residential proxy for the live recordings. Not needed.**
  - This host and the ARM Windows host leave through the same residential IP (98.42.201.9, Comcast,
    Sunnyvale, IPv4 only), so the Docker desktop already uses a residential line.
  - Amazon's interstitial persists with the time zone and language aligned to that location
    (America/Los_Angeles, en-US, now the defaults), and on a second visit with cookies. It judges the
    browser environment, not the IP. Getting around it would mean disguising the browser to defeat a bot
    check, which the recorder does not do.
  - Amazon is covered by the hermetic e2e (fixtures) instead.
- **Open:**
  - a Windows backend inside the framework (the screencast recorder is still a scratch script);
  - a scheduled live recording.

## U41: Liveness green on GitHub runners [user request + delegated], 2026-11-03

**User request:** fix the GitHub Actions. CI was green, but the daily Liveness run had failed every day since
its first run (6 of 40 checks; the failures are in issue #1).

- **Real problems, fixed:**
  - `bilibili.com/search` v2: plain HTTP is now redirected to `/zh-Hans/all?...`. The URL rule takes an
    optional locale segment, and the redirected page is a new fixture (`rust-zh-hans-locale`, 42 items).
  - `liveness.yaml` checked `generic/app.v1` on an App Store page, which `apps.apple.com/app` has owned since
    U38. The App Store URL is now that recipe's own check, and the generic one keeps Google Play.
  - `rex liveness` / `rex fixture capture` fetch over IPv4 only. ureq used the first (IPv6) address with no
    fallback and timed out on guide.michelin.com, Google Play and the App Store wherever IPv6 has no route.
- **Not recipe problems: sites that refuse runner IPs.** Baidu (captcha page), bilibili video (HTTP 412),
  ruanyifeng.com (HTTP 403) and guide.michelin.com (an empty 200) pass from the residential line.
  - Michelin was found by the second verification run: the report now gives the page's size, final URL and
    title when nothing matches, and an empty body is an error of its own.
  - amazon.com/search failed on 2 of the 3 verification runs: the runner got a 2251-byte page with no
    title (Amazon's challenge page) instead of results. It had passed every earlier day, so the block is
    intermittent. It is marked too, since a check that fails at random would make the daily run red for
    nothing. amazon.com/product still runs on the runners.
  - They are marked `datacenter_blocked` in `liveness.yaml`. The workflow passes
    `--skip-datacenter-blocked` and reports them as skipped; a local `rex liveness` checks them.
- **The first scheduled run after the fix failed:** `news.ycombinator.com/newest` returned HTTP 429. The run
  fetched the three HN URLs in parallel.
  - `rex liveness` now fetches one host's URLs in sequence, 1 s apart, with different hosts still in parallel.
  - A 429/503 is retried once after `Retry-After` (capped at 30 s).
  - This respects the site's rate limit; it does not work around it.
- **Local run as CI runs it:** 35 checked, 0 failing, 5 skipped, in 6.4 s.
- **Not changed:** the "Node.js 20 is deprecated" warnings. The actions are forced to Node 24 already
  (`FORCE_JAVASCRIPT_ACTIONS_TO_NODE24`); the warnings fail nothing.
  - Moving to the Node 24 majors (checkout v7, upload-artifact v7, download-artifact v8, setup-node v7,
    pnpm/action-setup v6, buildx v4, build-push v7) means breaking changes to review.
  - arduino/setup-protoc has no Node 24 release yet.
- **Same day: a residential proxy for the live recordings is not needed** (U40): this host and the ARM
  Windows host share one residential egress IP.

## U42: The extension sends nothing but recipe-source fetches [user request + delegated], 2026-11-04

**User request:** make sure the extension does not use network connections (asked while preparing a Chrome
Web Store listing, whose privacy form needs this answered truthfully).

- **Audit of the source**, every network-capable call:
  - `sync.ts` fetches the recipe sources: `index.json`, recipe objects, the schema set.
  - `rules.ts` fetches the extension's own icons (`chrome.runtime.getURL`); no network.
  - "Report broken recipe" opens a GitHub issue page in a new tab, only on the user's click.
  - The wasm engine has no network code (no networking crate, no `web_sys` requests).
  - **One leak:** the popup rendered a record's `image` field as `<img>`, so showing a result made the
    extension load the page's image from a third-party server (with no referrer, but still a request).
    It now shows the image URL as a link only.
- **Enforced by the browser:** the extension CSP now adds `img-src 'self' data:; media-src 'none';
  font-src 'self'; frame-src 'none'; connect-src 'self' https: http:`. `connect-src` stays open because
  recipe sources are user-configurable in the options page.
- **Verified on every e2e run:** `lib/netaudit.mjs` in the recorder records each request the extension
  makes. A request to anything but the manifest's recipe sources fails the scene.
  - A self-check per scenario proves the audit sees worker and popup fetches and that the CSP blocks a remote
    image (it showed up as `blocked: csp`).
  - First runs: 13 generic scenes, 0 violations. The only requests were the extension syncing its bundle
    (254 recipe-object fetches to `bundles.rex.test`).
- **For the store privacy form:**
  - The page is read on the user's click (`activeTab`), and both the page and the extracted record stay on
    the device.
  - The extension's only network use is downloading signed recipe bundles from its configured sources.
  - The page URL leaves only if the user files a broken-recipe report and ticks "include URL".

## U43: Popup redesign: Reader and Table views, JSON always at the bottom [user decisions + delegated], 2026-11-05

**User:** "currently it is so hard to read". Two mocks were made from real data (popup-mock.png on the
download page): A, a reader card; B, a data table. **Decision:** support both, chosen by the user, and
"always display JSON at the bottom".

- **Top (sticky):** rex, a Reader/Table switch (remembered in `chrome.storage.local`), the status in words
  (● Complete / Check / Incomplete / Empty / No match), and Copy JSON, Download, and the other copy form.
- **Reader:**
  - The heading: site · type, the title, the summary. A quote or product puts its price and change there,
    colored green up / red down; the Chinese UI uses red up / green down.
  - The facts in two columns with readable values: dates, grouped numbers, large counts shortened
    (3.1M / 3100万), video lengths as clocks, other durations in words (20 min), and enum values in words.
  - Long text folds (Show more). Lists show one entry per item: title, then who · numbers · when.
- **Table:** one row per field: the schema name, the exact value (every digit, UTC), and an origin dot with a
  legend. This replaces the old per-field `SITE_SELECTOR`/`JSON_LD` badges, which the reader no longer shows.
- **JSON panel:** at the bottom of both views, exactly what Copy JSON copies (envelope or record, per Options),
  colored by a small hand-written tokenizer.
- **No third-party UI code** (user, same day). Plain TypeScript and CSS: `src/format.ts` (pure formatting, 11
  unit tests in `tests/format.test.mjs`, in CI) and `src/render.ts` (the views). Enum prefixes are read from
  the protos at build time (`__REX_ENUM_PREFIXES__`).
- **Test contract:** fields carry `data-field`, `#status` carries `data-status` (the engine's name), and
  `#meta` carries `data-recipe`. The recorder reads these, not the visible words.
- **Found while shooting:** identifiers were grouped like numbers (`600,519`), so symbol, id, sku, isbn, year
  and similar now stay as written. An allrecipes notification prompt closed the popup, so the recorder's Chrome
  now blocks notification and geolocation prompts by policy.
- **The user guide** was retaken with both views and the JSON panel (16 screenshots).

## U44: A three-layer recipe judge [user request + delegated], 2026-11-05

The user found that the WSJ recipe put all of WSJ's categories into `tags`, and asked for the recipe system to
catch this kind of problem itself instead of them pointing out each one. Their decision: build all three layers.

**What was wrong.**

- The old checks proved only that recipes run: they compile, they match their pages, fixtures equal
  `expected.json`, and liveness returns `OK`. Nothing judged whether the values were right.
- The WSJ recipe was illustrative. It never matched a real WSJ URL, both of its fixtures were hand-written,
  and real pages fell through to the generic recipe. That recipe copied WSJ's CMS keyword field into `tags`:
  Dow Jones codes (`SYND`, `gpol`, `usdc`), taxonomy (`Dragonfly - World`), `LINK|…` keys, and the whole
  list a second time in lowercase.

**Layer 1: fixed rules, in every check.** `rex_engine::judge` walks a record by its schema.

- Rules: `tag-code`, `tag-duplicate`, `tag-count`, `title-site`, `person-junk`, `text-entity`,
  `text-untrimmed`, `url-relative`, `time-future`, `time-ancient`, `item-untitled`, `item-duplicate`,
  `price-negative`. Each one's precise test is in the module docs.
- Errors fail `rex test`, `rex liveness` and the draft gate. Warnings are only reported.
- A fixture or a liveness check can allow a finding with `judge_allow: [{rule, path?, reason}]`. For example,
  Hacker News item 1 really is titled "Y Combinator".
- The first run failed 80 of 2093 cases, all real problems.

**Layer 2: coverage, in `rex check`.**

- Every site recipe needs at least one captured fixture. Two hand-written ones no longer pass, and this is
  what would have caught WSJ.
- With `--liveness <file>…` (CI, publish, ci-local), every site recipe must be in a file's `checks` or in its
  `exempt: [{recipe, reason}]` list.
- The probe on 2026-11-05 found that a plain fetch works for Yahoo search only, so Yahoo is now checked daily.
  The 22 other site recipes are exempt; each is rendered-only, login-only, or refused (403/401).

**Layer 3: an LLM review, `rex judge`.**

- The model gets the page evidence (title, h1, meta tags, JSON-LD, a readability excerpt, the visible text),
  the schema, the source of each field, and the record. It returns findings against a fixed rubric (code
  `review.rs`).
- **Grounding:** an error must quote the evidence verbatim. An error whose quote is not in the page evidence
  becomes a warning marked `(unproven)`.
- Verdicts are cached by model plus prompt, so an unchanged page and record is not sent again.
- `rex draft --review` (harvest uses it) sends the review's errors back as a repair round.
- `tools/judge/audit.sh` is the periodic audit on this host, because the internal LLM proxy is unreachable
  from GitHub. It runs `rex liveness --save` (only pages the expected recipe matched), then judges those plus
  every captured fixture.
- **Model choice:** on the same 64 live pages, Gemini flash agreed with itself on 42 of 54 verdicts, and flash
  agreed with pro on 48. Pro's findings quoted the page and were specific, so the audit uses pro.
- **Verified false positives, now in the rubric:**
  - a product listed twice, once sponsored and once organic (Walmart);
  - lazy-loaded placeholders counted as missing items (Best Buy);
  - numbers paired with the wrong neighbor (bilibili plays / danmaku);
  - a time-zone conversion it did not do (NPR);
  - cnbc's regular close against its after-hours price.

**General fixes, added to the engine.**

- **Leftover character references:** decoded in string and URL coercion (`&#8217;`, `&amp;`), only where
  well-formed (`AT&T` stays as it is). 67 titles changed. L1 now also reports any numeric reference.
- **New pipes and options:**
  - `drop_codes` (the `tag-code` test);
  - `unique { ignore_case }`;
  - `not_future` (a publication date after the fetch is an event, a forecast or the next issue);
  - `filter { key }` (test a JSON-LD object's `name`);
  - `strip_site_name` (" | Site", " - Site", " · Site", using the page's own og:site_name with or without its
    tagline, application-name, twitter:site, or the host label).
- **JSON paths:** take `[key=Name]` filters
  (`interactionStatistic[interactionType=LikeAction].userInteractionCount`).
- **`replace_inherited` with no candidates:** leaves the field unset, for a site that has no good source.

**Recipe fixes.**

- **Generic recipes (all title/name rules):** `strip_site_name`.
- **generic/article.v1:**
  - tags `drop_codes` and case-folded `unique`;
  - authors skip URLs and e-mail addresses (og `article:author` is a profile URL by spec; WordPress SEO puts
    the login e-mail in JSON-LD) and dateline bylines;
  - dates `not_future`.
- **generic/recipe.v1:** keywords from Parse.ly tags first.
- **generic/video.v1:** views and likes by `interactionType`.
- **Sites:**
  - **wsj.com/main v3:** written from real pages. It matches `/<section>/…/<slug>-<8 hex>` as well as
    `/articles/`, has `data-testid` selectors, takes the body from the article section (readability also took
    the print footer), and has no tags. Two real fixtures replace the illustrative one; logged out, WSJ sends a
    teaser and the fixtures keep it.
  - **coinmarketcap:** the change came from another coin in the sidebar.
  - **bsky:** exact like and repost counts, not "63K".
  - **threads:** no empty items for deleted replies.
  - **github/trending:** title suffix removed.
  - **youtube/watch:** tags deduplicated ignoring case.
- **Result:** `rex test` passes all 2094 cases with no judge errors, and `rex check` is clean.

**Open (tasks.md):**

- A Michelin site recipe: generic/business takes Michelin's site-wide description.
- A wikiHow site recipe: its JSON-LD step text carries an "EXPERT TIP" block.
- The redactor rewrites letters inside character references (`&#x10D;` became `&#x10X;`).
- A schedule for `audit.sh` with a notification.

## U45: The extension packages its recipes and never connects to the internet [user decisions], 2026-11-07

The user's requirement: the packed Chrome extension must not use the internet. The recipes may live apart from
the code, but they cannot be loaded at run time. The user also decided:

- online sync is removed entirely, the developer builds included;
- the recipes stay in the rex repository;
- "Report broken recipe" keeps opening the GitHub issue form (a link the user clicks; the extension sends
  nothing).

**Before:** the package held no recipes. On first run, and every 6 hours after, it downloaded a signed bundle
(U16, U26, U27) from the source in its config (the dev source on the tailnet, later R2). Requirement FR-EX-2
required exactly this.

**Now:**

- **Build:** `rex bundle embed` writes `index.json` (the same Index, unsigned, because Chrome verifies the
  package), `schemas.fds` and `recipes.json` (the canonical recipes in index order). It embeds
  `recipes/generic`, `recipes/sites` and `recipes/auto`, 1036 recipes and about 950 KB; drafts are left out.
  - `scripts/build-extension.sh`, CI and `scripts/docker-e2e.sh` run it.
  - `build.mjs` refuses to build without it and copies it to `dist/recipes/`.
- **Extension (v0.3.0):** `src/recipes.ts` reads the three files from the package once, skips entries the
  engine cannot run (FR-EX-5), and builds one engine, with the developer-mode local recipe merged in.
  - Removed: `src/sync.ts` (verification, anti-rollback, ETags, alarms), the recipe sources and refresh
    interval in Options, and the popup's Refresh link.
  - Old stored settings keys are dropped on load.
  - The icon rules come from the packaged index, set on install, update and browser start.
- **Manifest:**
  - permissions are `activeTab`, `scripting`, `storage`, `declarativeContent`: no `alarms`, no
    `host_permissions`, no `optional_host_permissions`;
  - CSP has `connect-src 'self'`;
  - test builds alone add `<all_urls>`, for the e2e hooks.
- **Checks:**
  - `tests/e2e.mjs` records every request the extension makes and requires none outside the package. A run
    sees 206 requests into the package, which shows the listener works.
  - The recorder's network audit counts every request that leaves as a violation. Its self-check now also
    requires the CSP to block the worker's and the popup's probe fetches.
  - CI and ci-local check the store manifest (no host access, `connect-src 'self'`, recipes present).
  - `tests/e2e-sync.mjs` was deleted.
- **Removed:** the `publish.yml` workflow (it existed to feed the sync).
- **Kept:** `rex bundle build/publish/gc/verify` and the bundle format, for other hosts of the engine.
  `rex bundle build` is also what the wasm conformance test uses.

**Consequences:**

- A recipe fix reaches users with the next extension version, not within 6 hours.
- U16/U26/U27's signing and rollback protection no longer apply to the extension.
- The open action "choose the Cloudflare account, R2 bucket and domain" is not needed for the extension.

## U46: Chrome Web Store listing material in the repository [user decisions + delegated], 2026-11-07

The user asked for everything the Chrome Web Store needs to be ready in the repository, so they can upload it.
Their decisions:

- the repository becomes public, so the privacy policy, the support page and the popup's "Report broken recipe"
  link (GitHub issues) are reachable by every user;
- the listing is public;
- the listing is in English and Simplified Chinese.

**What the store needs** (Google's docs, checked 2026-11-07; links in store/README.md):

- the ZIP;
- a 128×128 store icon (96×96 artwork, 16 px transparent padding);
- 1 to 5 screenshots at 1280×800, full bleed, showing the real experience;
- a small promo tile (440×280) and an optional marquee tile (1400×560), both without text;
- the description, a category and a language;
- the privacy tab: single purpose, a justification for each permission, the remote-code answer, data-use
  disclosures and certifications, and a privacy policy URL.

The user data FAQ counts reading page content as handling user data even when nothing leaves the device. So
rex discloses **Website content** and needs a privacy policy, although it never connects to the internet.

**In the repository:**

- `store/README.md`: the dashboard tab by tab, with every value to paste;
- `store/listing/{en,zh_CN}.md`: the descriptions;
- `store/assets/`:
  - the icon;
  - 5 + 5 screenshots;
  - both promo tiles;
- `PRIVACY.md` (English and Chinese);
- `scripts/store-package.sh`: builds with the committed config, checks the manifest, and writes
  `target/store/rex-extension-<version>.zip`.

**How the images are made:**

- **Screenshots:** `extension/recorder/store.mjs`, in the recorder desktop at 1280×800, with the store build in
  real Google Chrome on live pages and the popup open under its icon.
  - Only pages without third-party ads are used: YouTube plays an ad, recipe sites and Eastmoney run banners.
  - No pages covered by login dialogs (Xueqiu).
  - No other people's video footage.
  - zh.wikipedia's variant prompt is closed with its own button.
- **Icon and tiles:** `tools/icons/make_icons.py`. The package's `icon-128.png` now has the store's padding too;
  16, 32 and 48 are unchanged.

**Found while shooting, and fixed:**

1. `wikipedia.org/article` did not match the language-variant paths (`/zh-cn/…`, `/zh-tw/…`, `/sr-ec/…`), so
   those pages fell to the generic recipe and kept the "- 维基百科，自由的百科全书" suffix in the title. Version 2
   matches them; a captured fixture covers it.
2. Rendered zh.wikipedia puts the "[编辑]" edit link inside `h1#firstHeading`. The title rule now excludes
   `.mw-editsection`; a fixture of the rendered page covers it.
3. Under Chrome on Linux with `--lang=zh-CN`, `chrome.i18n.getUILanguage()` reports `en-US` while Chrome serves
   the zh_CN messages. The popup's buttons were Chinese but its labels and dates English. The popup now takes the
   language of the messages in use (a `uiLang` message).
4. Without Segoe UI or Roboto (Linux), the popup's font stack fell to Noto Sans CJK, which draws `’` full width
   ("Apple’ s"). Latin fonts now come first.

**Made public safely:**

- Before the repository was made public, its whole history was scanned. It has no keys or credentials: the
  signing key's seed, the CRX key and the MinIO credentials are absent, and the internal LLM endpoint only appears
  as a test placeholder.
- The current docs replace the ARM Windows host's address and user and the tailnet hostnames with variables. The
  history still has them; they are not reachable from the internet.

## Open actions (need the user)

| Action | Why it needs you |
|---|---|
| ~~Choose the Cloudflare account, R2 bucket, and domain; generate the production Ed25519 key~~ | Not needed: the extension packages its recipes (U45) |
| Register a Chrome Web Store developer account and submit the listing (store/README.md); the repository must be public first | Account, fee and review (U46) |
| Commit and push the M2–M7 work; create the GitHub repo `Shuozeli/rex-fixtures` | Not committed until you say so |
| Fix the ARM2 browser endpoint (502) or allow another real browser for captures | Bot-walled and TLS-fingerprinting sites (U31) need a real browser |
| Bring the Alienware host back online (offline since 2026-10-14); clean the 63 leaked tabs on ARM Chrome :9401 | Captures fell back to ARM Chrome :9403/:9405 (U37) |
| Log the capture browser (Alienware Chrome) into JD | `jd.com/product` needs a logged-in session (U34) |
| ~~Rename the repo to `Shuozeli/rex`; commit and push~~ | Done 2026-10-12 |
