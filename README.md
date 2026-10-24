<!-- agent-updated: 2026-10-13T06:00:00Z -->

# rex: Page to JSON

rex turns a web page into **one typed, schema-conformant JSON record**: an article, a listing feed,
and later products, threads, and more. It does this with declarative, signed, remotely updatable
**recipes**.

- **Schemas** are proto messages with schema.org semantics (`rex.schema.*`), never a key-value bag.
- **Recipes** (`rex.extract.v1.ExtractionRecipe`) are pure data: CSS, JSON-LD, meta, and readability
  candidates plus a closed transform set. They live in git with redacted fixtures and pass CI on every
  change.
- **One engine.** A deterministic Rust engine (native, CLI, and wasm) produces a
  `rex.extract.v1.Extraction` with a status, per-field provenance, and the typed record.
- **The browser extension** ("rex: Page to JSON", Chrome MV3) extracts the page you're reading with
  one click. Recipes sync from a signed bundle on an S3-compatible host (Cloudflare R2 by default), so
  sites are added and fixed without an extension release.

rex is an independent project. Crawlers and apps (crawlc, myfeed, …) can embed the engine, but rex
depends on none of them.

> Status: **M1–M6 done, M7 partly** (2026-10-13): engine, wasm, 12 site recipes with redacted fixtures,
> publishing with GC, extension v0.2, `rex draft`, and CI. Pending (needs you): the Cloudflare account,
> domain and production key, and the Chrome Web Store decision. See [tasks](docs/tasks.md).

## Layout

```
proto/rex/      contracts: schema annotations, article.v1, feed.v1, Extraction, ExtractionRecipe, bundle
crates/         rex-proto, rex-schema, rex-dom, rex-readability, rex-engine, rex-redact,
                rex-bundle (build/sign/verify), rex-wasm (engine for the extension), rex-cli (`rex`)
extension/      Chrome MV3 extension (TypeScript + esbuild); tests/e2e.mjs, tests/e2e-sync.mjs
recipes/        generic/ base recipes, sites/<host>/<name>.yaml, drafts/ (dev channel only)
fixtures/       per-recipe page fixtures (redacted page.html, meta.json, expected.json)
conformance/    engine-level cases with their own recipe.yaml
liveness.yaml   live sample pages checked daily
tools/          readability-bench (vs Mozilla Readability.js), icons
scripts/        build-extension.sh, ci-local.sh
.github/        ci.yml, publish.yml, liveness.yml
docs/           design docs, operations, index.html (animated explainer)
```

## Build and use

Requires Rust 1.95+ and `protoc` (the proto contracts are compiled at build time).

```bash
cargo build --release -p rex-cli           # target/release/rex
rex check recipes --fixtures fixtures      # contract, >= 2 fixtures per site recipe
rex test                                   # fixtures/ + conformance/ against expected.json
rex extract --html page.html --url https://www.wsj.com/articles/x       # match + extract
rex explain --html page.html --url https://… --recipe wsj.com/main      # per-field trace
rex match --url https://news.ycombinator.com/ --html page.html
rex liveness                               # live sample pages in liveness.yaml
```

### Adding a site

```bash
rex fixture capture https://example.com/a --id example.com/article --case a   # -> fixtures-private/ (gitignored)
rex fixture redact fixtures-private/example.com/article/a fixtures/example.com/article/a
rex draft --from fixtures-private/example.com/article/a --llm <endpoint>     # optional BYO-LLM draft -> recipes/drafts/
rex test --update example.com && rex check recipes --fixtures fixtures       # review expected.json
```

`--llm` accepts an OpenAI-compatible `.../chat/completions` URL (`--model`, `REX_LLM_API_KEY`) or a Gemini
`...:generateContent` URL. Login-gated pages: use the extension's developer mode, **Save as fixture**,
then `rex fixture import <file>`.

### Recipe bundles

```bash
rex bundle keygen --key-id rex-dev-$(hostname) --out ~/.config/rex/dev-signing-key.json   # keep it out of the repo
rex bundle publish --key ~/.config/rex/dev-signing-key.json --to ~/.local/share/rex/bundle   # a directory
REX_S3_ENDPOINT=… REX_S3_ACCESS_KEY_ID=… REX_S3_SECRET_ACCESS_KEY=… \
  rex bundle publish --channel stable --recipes recipes/generic --recipes recipes/sites \
  --key <key> --to s3://<bucket>/<prefix>                                                   # R2 / S3 / MinIO
rex bundle verify --url https://<host>/<prefix>/v1/stable/ --key-id <id> --public <hex>
rex bundle gc --channel stable --key <key> --to s3://<bucket>/<prefix>                       # 30-day retention
```

Hosting setup, key rotation and GC are in [operations](docs/operations.md).

### Chrome extension

```bash
cp extension/config.example.json extension/config.local.json   # default source baseUrl + pinned public key
scripts/build-extension.sh                                     # -> extension/dist (load unpacked)
REX_E2E=1 scripts/build-extension.sh
(cd extension && node tests/e2e.mjs && node tests/e2e-sync.mjs)   # real Chrome, hermetic
```

- **Popup:** status, the typed record with per-field origins, alternatives, copy (envelope or record),
  download, and a report for broken recipes.
- **Options:** sources (order, enable, pinned keys, host permission), refresh interval, copy format, and
  developer mode (local unsigned recipe, expanded trace, Save as fixture). The UI is in English and
  Chinese.
- **e2e:**
  - the fixture pages, under their real URLs, must give wasm output byte-identical to native;
  - developer mode and the fixture round trip are exercised;
  - `e2e-sync.mjs` forges every sync rejection path;
  - screenshots go to `extension/test-results/`;
  - `REX_E2E_LIVE=1` adds live pages, and `REX_E2E_LANG=zh-CN` switches the UI to Chinese.

### CI

`.github/workflows/ci.yml` runs fmt, clippy, tests, `rex check/test`, the fixture lint, the wasm size budget
(≤ 1.15 MB gzipped), conformance under wasm, and the extension build plus both e2e suites.
`scripts/ci-local.sh` runs the same gates locally. `publish.yml` publishes `stable` and `dev` once it is
configured (operations.md); `liveness.yml` runs daily.

## Docs

- [Decisions](docs/decisions.md): every design decision (U1–U30) and the open actions
- [Tasks](docs/tasks.md): milestone status, known gaps
- [Design](docs/design.md): system architecture, recipe lifecycle, publishing, extension, security
- [Operations](docs/operations.md): hosting, publishing, GC, key rotation, liveness triage
- [Engine spec](docs/designs/261012-generic-recipe-extractor.md): evaluation and normative value semantics
- [PRD](docs/prd.md) and [Requirements](docs/requirements.md)
- [Doc manifest](docs/MANIFEST.md)
