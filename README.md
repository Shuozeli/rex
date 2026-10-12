<!-- agent-updated: 2026-10-12T06:45:00Z -->

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
  one click. Recipes sync from a signed bundle on Cloudflare R2, so sites are added and fixed without
  an extension release.

rex is an independent project. Crawlers and apps (crawlc, myfeed, …) can embed the engine, but rex
depends on none of them.

> Status: design complete (v1), no implementation yet.

## Layout

```
proto/rex/      contracts: schema annotations, article.v1, feed.v1, Extraction, ExtractionRecipe, bundle
recipes/        generic/ base recipes, sites/<host>/<name>.yaml site recipes
scripts/        validate_recipes.py (prototype of `rex check`)
docs/           design docs + index.html (animated explainer)
```

Validate the example recipes against the contracts:

```bash
uv run --with protobuf --with pyyaml python scripts/validate_recipes.py recipes/generic/*.yaml recipes/sites/*/*.yaml
```

## Docs

- [Decisions](docs/decisions.md): every design decision (U1–U21) and the open actions
- [Design](docs/design.md): system architecture, recipe lifecycle, publishing, extension, security
- [Engine spec](docs/designs/261012-generic-recipe-extractor.md): evaluation and normative value semantics
- [PRD](docs/prd.md) and [Requirements](docs/requirements.md)
- [Doc manifest](docs/MANIFEST.md)
