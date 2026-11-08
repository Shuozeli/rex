// Records every network request the extension itself makes (its service worker, popup and options page). It
// may make none (decisions.md U45): the recipes are in the package, and rex reads the page the user opened and
// sends it nowhere.
//
// New extension targets are caught at start (browser-level auto-attach, paused until Network is enabled), so
// a popup's first request is not missed. The page itself is not audited: its requests are the site's own.

/** Starts auditing. `audit.requests` collects { url, type, target } for every non-local request. */
export async function startAudit(conn, extId, workerSession) {
  // `localBy`: the kinds of target ("worker", "page": the popup or options) that made a request into the package,
  // which proves the audit sees them at all. A target can be attached twice, so kinds are kept, not sessions.
  const audit = { requests: [], sessions: new Set(), seen: new Set(), localBy: new Set() };
  const kind = new Map();
  const kindOf = (info) => (info.type === "service_worker" ? "worker" : "page");
  const prefix = `chrome-extension://${extId}/`;
  const local = (url) => /^(chrome-extension|data|blob|chrome|about):/.test(url);

  conn.on((msg) => {
    if (msg.method === "Target.attachedToTarget" && !msg.sessionId) {
      const { sessionId, targetInfo, waitingForDebugger } = msg.params;
      const ours = targetInfo.url.startsWith(prefix);
      (async () => {
        if (ours) {
          audit.sessions.add(sessionId);
          kind.set(sessionId, kindOf(targetInfo));
          await conn.send("Network.enable", {}, sessionId).catch(() => {});
        }
        if (waitingForDebugger) await conn.send("Runtime.runIfWaitingForDebugger", {}, sessionId).catch(() => {});
      })();
    }
    if (msg.method === "Network.requestWillBeSent" && audit.sessions.has(msg.sessionId)) {
      const { url } = msg.params.request;
      // A target can be attached twice (the runner's own session and auto-attach): count a request once.
      const key = `${msg.params.requestId}|${url}`;
      if (local(url)) audit.localBy.add(kind.get(msg.sessionId) ?? "page");
      else if (!audit.seen.has(key)) {
        audit.seen.add(key);
        audit.requests.push({ id: msg.params.requestId, url, type: msg.params.type ?? "", target: msg.sessionId, blocked: null });
      }
    }
    // A request the browser refused before sending (the CSP, mixed content) shows up here; it never left.
    if (msg.method === "Network.loadingFailed" && audit.sessions.has(msg.sessionId) && msg.params.blockedReason) {
      for (const r of audit.requests) if (r.id === msg.params.requestId) r.blocked = msg.params.blockedReason;
    }
  });
  // The worker already runs; new targets (the popup, a restarted worker) are attached as they start.
  audit.sessions.add(workerSession);
  kind.set(workerSession, "worker");
  await conn.send("Network.enable", {}, workerSession);
  await conn.send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });
  return audit;
}

/** Requests that left the browser, as "type url" lines: every one is a violation. */
export function violations(audit) {
  return audit.requests.filter((r) => !r.blocked).map((r) => `${r.type || "request"} ${r.url}`);
}

const PROBE = "https://rex-audit-probe.invalid";

/** Runs in a target: starts `probe` and resolves to the CSP directive that blocked it, or null after 2 s. */
const violation = (scope, probe) => `new Promise((done) => {
  const on = (e) => { ${scope}.removeEventListener("securitypolicyviolation", on); done(e.effectiveDirective || e.violatedDirective); };
  ${scope}.addEventListener("securitypolicyviolation", on);
  ${probe};
  setTimeout(() => done(null), 2000);
})`;

/**
 * Proves the audit can catch a leak, without leaking (decisions.md U45):
 * - a fetch of a packaged file from the worker and one from the popup are both recorded, so it is attached to
 *   both;
 * - the CSP stops a fetch from the worker, a fetch from the popup and an image in the popup, each with a
 *   securitypolicyviolation for connect-src or img-src (so they never reach the network, where the audit would
 *   report them).
 * The probes are removed from the record. Returns the problems found (empty when the audit and the CSP work).
 */
export async function selfCheck(audit, inWorker, inPopup) {
  const problems = [];
  audit.localBy.clear();
  const own = `fetch(chrome.runtime.getURL("manifest.json"), { cache: "no-store" }).then((r) => r.ok)`;
  await inWorker(own);
  await inPopup(own);
  await new Promise((r) => setTimeout(r, 500));
  if (!audit.localBy.has("worker")) problems.push("audit self-check: the service worker's request was not recorded");
  if (!audit.localBy.has("page")) problems.push("audit self-check: the popup's request was not recorded");
  const checks = [
    ["the worker's fetch", "connect-src", () => inWorker(violation("self", `fetch("${PROBE}/worker").catch(() => {})`))],
    ["the popup's fetch", "connect-src", () => inPopup(violation("document", `fetch("${PROBE}/popup").catch(() => {})`))],
    ["the popup's image", "img-src", () => inPopup(violation("document", `new Image().src = "${PROBE}/image"`))],
  ];
  for (const [what, want, run] of checks) {
    const got = await run().catch((e) => `error: ${e}`);
    if (got !== want) problems.push(`audit self-check: the CSP did not block ${what} with ${want} (got ${got})`);
  }
  audit.requests = audit.requests.filter((r) => !r.url.startsWith(PROBE));
  return problems;
}
