<!-- agent-updated: 2026-10-13T06:00:00Z -->

# Doc Manifest

| File | Covers | Update When |
|------|--------|-------------|
| ../README.md | What rex is, status, layout, build and CLI usage, doc links | Scope or status changes, the repo rename, the first runnable build |
| user-guide.md (+ user-guide/*.png) | How to install and use the extension: the icon, popup, buttons, statuses, generic recipes, trace, options, privacy, troubleshooting; screenshots from `extension/recorder/guide.mjs` | The popup, options or install flow change, or the screenshots go stale (rerun guide.mjs, operations.md) |
| ../PRIVACY.md | The extension's privacy policy (English and Chinese), linked from the Web Store listing | What the extension reads, stores or sends changes, or the report link changes |
| ../store/README.md (+ listing/, assets/) | Chrome Web Store listing: package script, every dashboard field with its text, screenshots, icon, promo tiles, privacy-tab answers | Permissions, the description, the UI shown in screenshots, or the store's requirements change (rerun store.mjs / make_icons.py) |
| decisions.md | Every design decision U1–U30 (U1–U5 with the user, U6+ delegated), amendments, superseded items, open actions | A decision is made, reopened, or superseded, or an open action is done |
| design.md | System design: architecture, repo layout, recipe lifecycle and CI gates, authoring toolchain, publishing and keys, extension (flows, packaged recipes, permissions, UI), security, testing, feedback, integrations, milestones | Any component, workflow, permission, hosting, or milestone change |
| designs/261012-generic-recipe-extractor.md | Engine spec: contract map, evaluation semantics, normative value semantics §8, limits §10.3, crates, wasm API, security §13, testing, versioning | Any proto under `proto/rex/` changes, or engine semantics, limits, or the API change |
| operations.md | How recipes ship (packaged, U45), the bundle host for other engine hosts (R2, publishing by hand, GC, key rotation, MinIO), recording, liveness and judge triage | How the extension gets its recipes, `rex bundle` flags, the rotation procedure, or liveness/judge handling change |
| tasks.md | Milestone checklist, done items, known gaps and bugs | A task is completed, a gap is found, or a milestone changes |
| prd.md | Problem, goals and non-goals, personas, user stories, release scope, metrics, risks | Product scope or release plan changes |
| requirements.md | Numbered FR/NFR requirements (contracts, engine, recipes, publishing, extension) and v0.1 acceptance criteria | A requirement is added, changed, or shipped |
| index.html | Animated browser explainer of the design; renders the .md docs (preview only, not a source of truth) | design, decisions, or contracts change in a way the walkthroughs depict |
