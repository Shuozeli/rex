// Sync rejection paths in a real Chrome (design.md §9.3, FR-EX-3/4/5), hermetic: the bundle host is
// https://bundles.rex.test/, answered by Playwright routing of the service worker's own fetches.
//
// The base bundle is produced by `rex bundle publish` (the real CLI) with a throwaway key; every variant
// (tampered, rolled back, wrong key, bad hashes, version gate, unsupported features) is forged here and
// signed with Node's Ed25519. After each step the extension must keep the last good bundle and report
// the reason.
//
// Usage: pnpm build:e2e && node tests/e2e-sync.mjs
//   REX_BIN        rex binary (default ../target/release/rex)
//   REX_E2E_MINIO  optional base URL of a live S3/MinIO channel signed with the dev key; adds a live
//                  sync + conditional-GET check against it (needs REX_E2E_MINIO_KEY=keyId:hex)

import { launch, makeKey, publishBundle, serveBundle, settingsFor, sha, signIndex } from "./harness.mjs";

const key = makeKey("e2e-key", 7);
const stranger = makeKey("e2e-key", 9); // same id, different key: must not verify

// Base bundle from the real CLI; every variant below is forged from it.
const { files, index: baseIndex } = publishBundle({ key, channel: "e2e" });
const signed = (index, k = key, mutate = (s) => s) => signIndex(index, k, mutate);
const clone = (o) => JSON.parse(JSON.stringify(o));
const withSeq = (seq, f = (i) => i) => f({ ...clone(baseIndex), sequence: String(seq) });

const { context, sw, close } = await launch();
const served = { index: 0, notModified: 0 };
const { baseUrl: BASE } = await serveBundle(context, "e2e", files, served);

let failures = 0;
const check = (name, ok, detail) => {
  if (ok) console.log(`ok   ${name}`);
  else {
    failures++;
    console.log(`FAIL ${name}: ${detail}`);
  }
};

try {
  const source = { id: "e2e", baseUrl: BASE, channel: "e2e", keys: [{ keyId: key.keyId, public: key.public }], enabled: true };
  const settings = settingsFor(source);
  const sync = async () => (await sw.evaluate(() => globalThis.rexDebug.sync())).status.sources.find((s) => s.id === "e2e");
  /** Serve `index` bytes, sync, and expect the stored sequence and an error matching `re` (or none). */
  const step = async (name, indexBytes, wantSeq, re) => {
    files.set("index.json", indexBytes);
    const s = await sync();
    const errOk = re === null ? s.lastError === null : re.test(s.lastError ?? "");
    check(name, s.sequence === wantSeq && errOk, `sequence=${s.sequence} lastError=${s.lastError}`);
    return s;
  };

  // 1. First sync through the options flow (saveSettings validates, stores and syncs).
  files.set("index.json", signed(withSeq(1)));
  const saved = await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), settings);
  const first = saved.ok ? saved.status.sources[0] : null;
  check("initial sync via saveSettings", first?.sequence === "1" && first.lastError === null, JSON.stringify(saved).slice(0, 300));
  const total = baseIndex.entries.length;
  check("all entries applied", first?.recipeCount === total, `${first?.recipeCount} of ${total}`);

  // 2. Conditional GET: unchanged index.json is answered 304.
  const before = served.notModified;
  await step("unchanged index -> 304 not modified", signed(withSeq(1)), "1", null);
  // >= 1: the install-time sync may also run (serialized) and get its own 304.
  check("server answered 304", served.notModified >= before + 1, `${served.notModified - before} 304s`);

  // 3-5. Signature failures keep bundle #1.
  await step(
    "tampered signature rejected",
    signed(withSeq(2), key, (s) => ({ ...s, signature: Buffer.from(s.signature, "base64").map((b, i) => (i === 0 ? b ^ 1 : b)).toString("base64") })),
    "1",
    /signature/,
  );
  await step("index signed by an unpinned key rejected", signed(withSeq(2), stranger), "1", /signature/);
  await step(
    "payload edited after signing rejected",
    signed(withSeq(2), key, (s) => ({ ...s, payload: Buffer.from(JSON.stringify({ ...withSeq(2), sequence: "9" })).toString("base64") })),
    "1",
    /signature/,
  );

  // 6. A valid newer bundle applies.
  await step("valid bundle #2 applies", signed(withSeq(2)), "2", null);

  // 7. Rollback to #1 rejected.
  await step("rollback to #1 rejected", signed(withSeq(1)), "2", /rollback rejected/);

  // 8. Recipe object whose bytes do not match the index hash.
  const hn = baseIndex.entries.find((e) => e.ref.id === "news.ycombinator.com/front");
  const hnJson = JSON.parse(files.get(`r/${hn.ref.sha256}.json`).toString("utf8"));
  const hnV = Buffer.from(JSON.stringify({ ...hnJson, version: hnJson.version + 1 }));
  const hnSha = sha(hnV);
  files.set(`r/${hnSha}.json`, Buffer.concat([hnV, Buffer.from(" ")]));
  await step(
    "recipe hash mismatch rejected",
    signed(withSeq(3, (i) => ({ ...i, entries: i.entries.map((e) => (e.ref.id === hn.ref.id ? { ...e, ref: { ...e.ref, version: e.ref.version + 1, sha256: hnSha } } : e)) }))),
    "2",
    /hash mismatch/,
  );

  // 9. Schema set whose bytes do not match.
  const fdsSha = baseIndex.schemas.sha256;
  const fakeFds = "f".repeat(64);
  files.set(`s/${fakeFds}.fds`, files.get(`s/${fdsSha}.fds`));
  await step("schema set hash mismatch rejected", signed(withSeq(3, (i) => ({ ...i, schemas: { ...i.schemas, sha256: fakeFds } }))), "2", /schema set hash mismatch/);

  // 10. Channel mismatch.
  await step("wrong channel rejected", signed(withSeq(3, (i) => ({ ...i, channel: "stable" }))), "2", /channel/);

  // 11. Client version gate.
  await step("min_client_version gate", signed(withSeq(3, (i) => ({ ...i, minClientVersion: "99.0.0" }))), "2", /needs extension 99\.0\.0/);

  // 12. FR-EX-5: a base needing an unknown feature is skipped with everything extending it.
  const generic = baseIndex.entries.find((e) => e.ref.id === "generic/article.v1");
  const genJson = JSON.parse(files.get(`r/${generic.ref.sha256}.json`).toString("utf8"));
  const genV = Buffer.from(JSON.stringify({ ...genJson, requires: ["teleport"] }));
  files.set(`r/${sha(genV)}.json`, genV);
  const children = baseIndex.entries.filter((e) => e.extends === generic.ref.id).length;
  const s12 = await step(
    "unsupported requires: entries skipped, rest applies",
    signed(withSeq(3, (i) => ({ ...i, entries: i.entries.map((e) => (e.ref.id === generic.ref.id ? { ...e, requires: ["teleport"], ref: { ...e.ref, sha256: sha(genV) } } : e)) }))),
    "3",
    null,
  );
  check(
    "skipped = base + its children",
    s12.skipped.length === children + 1 && s12.recipeCount === total - children - 1 && s12.skipped.some((x) => /teleport/.test(x.reason)),
    JSON.stringify(s12.skipped).slice(0, 300),
  );
  const rules = await sw.evaluate(() => globalThis.rexDebug.rules());
  check("icon rules follow the applied entries", rules.hosts.includes("news.ycombinator.com") && !rules.hosts.includes("wsj.com"), rules.hosts.join(","));

  // 13. Recovery: a good bundle after failures clears the error.
  await step("good bundle #4 after failures", signed(withSeq(4)), "4", null);
  const rules4 = await sw.evaluate(() => globalThis.rexDebug.rules());
  check("icon rules cover every site recipe again", rules4.hosts.includes("wsj.com") && rules4.rules.length === 1, rules4.hosts.join(","));

  // 14. Changing the pinned keys forgets the old bundle (state is bound to the source identity).
  const other = makeKey("e2e-key-2", 11);
  const rekeyed = await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), {
    ...settings,
    sources: [{ ...source, keys: [{ keyId: other.keyId, public: other.public }] }],
  });
  const s14 = rekeyed.status.sources[0];
  check("re-keyed source starts from scratch and rejects the old signer", s14.sequence === null && /not pinned/.test(s14.lastError ?? ""), JSON.stringify(s14));

  // 14b. ...but the rollback floor of (base URL, channel) survives the key change: an older bundle signed by
  // the new key is still refused, a newer one applies.
  await step("floor survives re-keying: #3 under the new key rejected", signed(withSeq(3), other), null, /rollback rejected: sequence 3 < 4/);
  const s14c = await step("newer #5 under the new key applies", signed(withSeq(5), other), "5", null);

  // 14c. A bundle selected by an engine with other features is re-selected although index.json is unchanged.
  await sw.evaluate(async () => {
    const got = await chrome.storage.local.get("rex.source.e2e");
    await chrome.storage.local.set({ "rex.source.e2e": { ...got["rex.source.e2e"], features: "older-engine", skipped: [{ id: "x", reason: "stale" }] } });
  });
  const notModified = served.notModified;
  const s14d = await step("engine change re-selects the same bundle", signed(withSeq(5), other), "5", null);
  check("re-selection skipped the conditional GET", served.notModified === notModified && s14d.skipped.length === s14c.skipped.length, `${served.notModified - notModified} 304s, skipped ${JSON.stringify(s14d.skipped)}`);

  // 15. Invalid settings are refused without touching storage.
  const bad = await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), { ...settings, sources: [{ ...source, keys: [{ keyId: "x", public: "zz" }] }] });
  check("invalid settings refused", bad.ok === false && /64 hex/.test(bad.error), JSON.stringify(bad));

  // 16. Optional: live S3-compatible host (MinIO / R2) signed with the dev key.
  if (process.env.REX_E2E_MINIO) {
    const [keyId, pub] = (process.env.REX_E2E_MINIO_KEY ?? "").split(":");
    const live = { id: "minio", baseUrl: process.env.REX_E2E_MINIO, channel: "dev", keys: [{ keyId, public: pub }], enabled: true };
    const r = await sw.evaluate((s) => globalThis.rexDebug.saveSettings(s), { ...settings, sources: [live] });
    const m = r.ok ? r.status.sources[0] : null;
    check(`live sync from ${live.baseUrl}`, m?.sequence !== null && m?.lastError === null && m?.recipeCount > 0, JSON.stringify(r).slice(0, 300));
    const again = (await sw.evaluate(() => globalThis.rexDebug.sync())).status.sources[0];
    check("live re-sync keeps the bundle (conditional GET)", again.sequence === m?.sequence && again.lastError === null, JSON.stringify(again));
    console.log(`     live bundle #${m?.sequence}, ${m?.recipeCount} recipes`);
  }
} catch (e) {
  failures++;
  console.log(`FAIL ${String(e?.stack ?? e)}`);
} finally {
  await close();
}

console.log(failures === 0 ? "\nE2E SYNC PASSED" : `\nE2E SYNC FAILED (${failures})`);
process.exit(failures === 0 ? 0 : 1);
