// Records every network request the extension itself makes (its service worker, popup and options page) and
// checks it against the recipe sources the manifest grants: rex reads the page the user opened and sends it
// nowhere, so the only network it may use is fetching recipe bundles.
//
// New extension targets are caught at start (browser-level auto-attach, paused until Network is enabled), so
// a popup's first request is not missed. The page itself is not audited: its requests are the site's own.

/** The origins the extension may contact: its manifest host permissions, without <all_urls> (e2e builds). */
export async function allowedOrigins(evaluateInWorker) {
  const hosts = await evaluateInWorker("chrome.runtime.getManifest().host_permissions ?? []");
  return hosts.filter((h) => h !== "<all_urls>").map((h) => new URL(h.replace("/*", "/")).origin);
}

/** Starts auditing. `audit.requests` collects { url, type, target } for every non-local request. */
export async function startAudit(conn, extId, workerSession) {
  const audit = { requests: [], sessions: new Set(), seen: new Set() };
  const prefix = `chrome-extension://${extId}/`;
  const local = (url) => /^(chrome-extension|data|blob|chrome|about):/.test(url);

  conn.on((msg) => {
    if (msg.method === "Target.attachedToTarget" && !msg.sessionId) {
      const { sessionId, targetInfo, waitingForDebugger } = msg.params;
      const ours = targetInfo.url.startsWith(prefix);
      (async () => {
        if (ours) {
          audit.sessions.add(sessionId);
          await conn.send("Network.enable", {}, sessionId).catch(() => {});
        }
        if (waitingForDebugger) await conn.send("Runtime.runIfWaitingForDebugger", {}, sessionId).catch(() => {});
      })();
    }
    if (msg.method === "Network.requestWillBeSent" && audit.sessions.has(msg.sessionId)) {
      const { url } = msg.params.request;
      // A target can be attached twice (the runner's own session and auto-attach): count a request once.
      const key = `${msg.params.requestId}|${url}`;
      if (!local(url) && !audit.seen.has(key)) {
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
  await conn.send("Network.enable", {}, workerSession);
  await conn.send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });
  return audit;
}

/** Requests that left the browser for an origin outside the allowed ones, as "type url" lines. */
export function violations(audit, allowed) {
  return audit.requests
    .filter((r) => !r.blocked && !allowed.includes(new URL(r.url).origin))
    .map((r) => `${r.type || "request"} ${r.url}`);
}

const PROBE = "https://rex-audit-probe.invalid";

/**
 * Proves the audit sees what it should: a fetch from the worker and one from the popup must be recorded, and
 * an image the popup tries to load must be blocked by the CSP (never sent). The probes are then removed from
 * the record. Returns the problems found (empty when the audit and the CSP work).
 */
export async function selfCheck(audit, inWorker, inPopup, sleep) {
  await inWorker(`fetch("${PROBE}/worker").then(() => 0, () => 0)`);
  await inPopup(`(() => { fetch("${PROBE}/popup").catch(() => {}); const i = new Image(); i.src = "${PROBE}/image"; return 0; })()`);
  await sleep(800);
  const find = (path) => audit.requests.find((r) => r.url === `${PROBE}${path}`);
  const problems = [];
  if (!find("/worker")) problems.push("audit self-check: a fetch from the service worker was not recorded");
  if (!find("/popup")) problems.push("audit self-check: a fetch from the popup was not recorded");
  const image = find("/image");
  if (image && image.blocked !== "csp") {
    problems.push(`audit self-check: the popup loaded a remote image (blocked: ${image.blocked}); the CSP img-src is not enforced`);
  }
  audit.requests = audit.requests.filter((r) => !r.url.startsWith(PROBE));
  return problems;
}
