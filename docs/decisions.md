<!-- agent-updated: 2026-10-12T06:45:00Z -->

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

## U12: Readability and sanitizing [delegated], 2026-10-12

- The `readability` source is a port of company-feed-server's `feed-content::process_html` (wasm-capable
  dependencies). Mozilla Readability.js is only a differential benchmark.
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

## U20: Drift and feedback [delegated], 2026-10-12

There is **no telemetry**.

- The popup shows `status`, and on `FAILED_EXPECTATION`/`INVALID` it offers **"Report broken recipe"**.
  This opens a prefilled GitHub issue (recipe ref, status, failing paths, extension and engine
  versions; the page URL only if the user ticks a box). It is user-initiated and has no backend.
- **Liveness CI** (scheduled, daily) runs only for recipes marked `liveness: public` in a sidecar list.
  It fetches the public, login-free sample URLs over HTTP, runs extraction, and opens or updates an
  issue on a non-OK status. Paywalled or login-only sites rely on user reports.

## U21: Integrations are consumer-owned [delegated], 2026-10-12

| Consumer | Uses | Lives in |
|---|---|---|
| crawlc / dragbv3 | `rex-engine` as its extractor; an `ExtractionSchema` → `ExtractionRecipe` translator; hints from its feed entries; `source_id` = its raw id | crawlc |
| myfeed | `feed.v1` recipes via the `rex-engine` crate or the CLI, replacing its JS recipes | myfeed |
| Any app | The wasm package (`@rex/engine`) or the crate | — |

rex guarantees only its contracts, the engine semver, and the bundle format.

## Open actions (need the user)

| Action | Why it needs you |
|---|---|
| Choose the Cloudflare account, R2 bucket, and domain; generate the Ed25519 signing key pair | Accounts and secrets (U16) |
| ~~Rename the repo to `Shuozeli/rex`; commit and push~~ | Done 2026-10-12 |
