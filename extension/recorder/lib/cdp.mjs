// Minimal raw CDP client (flattened sessions) with event subscriptions; no dependencies.
export async function connect(httpBase) {
  const { webSocketDebuggerUrl } = await (await fetch(`${httpBase}/json/version`)).json();
  const ws = new WebSocket(webSocketDebuggerUrl);
  ws.binaryType = "arraybuffer";
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = () => fail(new Error("ws open failed")); });
  let next = 1;
  const pending = new Map();
  const listeners = new Set();
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.id) {
      const p = pending.get(msg.id);
      if (!p) return;
      pending.delete(msg.id);
      msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result);
    } else for (const l of listeners) l(msg);
  };
  const send = (method, params = {}, sessionId, timeoutMs = 60000) =>
    new Promise((resolve, reject) => {
      const id = next++;
      const t = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, timeoutMs);
      pending.set(id, { resolve: (v) => { clearTimeout(t); resolve(v); }, reject: (e) => { clearTimeout(t); reject(e); } });
      ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  return { send, on: (f) => (listeners.add(f), () => listeners.delete(f)), close: () => ws.close() };
}

export async function evaluate(conn, sessionId, expression, timeoutMs = 60000) {
  const res = await conn.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId, timeoutMs);
  if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description?.split("\n")[0] ?? res.exceptionDetails.text);
  return res.result.value;
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Attach to the extension's service worker, waking it by opening its options page if needed. */
export async function attachWorker(conn, extId) {
  const find = async () => (await conn.send("Target.getTargets")).targetInfos.find((t) => t.type === "service_worker" && t.url.startsWith(`chrome-extension://${extId}/`));
  let t = await find();
  if (!t) {
    const { targetId } = await conn.send("Target.createTarget", { url: `chrome-extension://${extId}/options.html` });
    for (let i = 0; i < 60 && !t; i++) { await sleep(500); t = await find(); }
    await conn.send("Target.closeTarget", { targetId });
  }
  if (!t) throw new Error("no extension service worker");
  return (await conn.send("Target.attachToTarget", { targetId: t.targetId, flatten: true })).sessionId;
}
