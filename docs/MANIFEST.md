<!-- agent-updated: 2026-10-12T06:30:00Z -->

# Doc Manifest

| File | Covers | Update When |
|------|--------|-------------|
| ../README.md | What rex is, status, layout, how to validate recipes, doc links | Scope or status changes, the repo rename, the first runnable build |
| decisions.md | Every design decision U1–U21 (U1–U5 with the user, U6–U21 delegated), superseded items, open actions | A decision is made, reopened, or superseded, or an open action is done |
| design.md | System design: architecture, repo layout, recipe lifecycle and CI gates, authoring toolchain, publishing and keys, extension (flows, sync, permissions, UI), security, testing, feedback, integrations, milestones | Any component, workflow, permission, hosting, or milestone change |
| designs/261012-generic-recipe-extractor.md | Engine spec: contract map, evaluation semantics, normative value semantics §8, limits §10.3, crates, wasm API, security §13, testing, versioning | Any proto under `proto/rex/` changes, or engine semantics, limits, or the API change |
| prd.md | Problem, goals and non-goals, personas, user stories, release scope, metrics, risks | Product scope or release plan changes |
| requirements.md | Numbered FR/NFR requirements (contracts, engine, recipes, publishing, extension) and v0.1 acceptance criteria | A requirement is added, changed, or shipped |
| index.html | Animated browser explainer of the design; renders the .md docs (preview only, not a source of truth) | design, decisions, or contracts change in a way the walkthroughs depict |
