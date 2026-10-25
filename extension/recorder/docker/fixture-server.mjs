// Hermetic web for the desktop's Chrome: every host resolves to 127.0.0.1 (--host-resolver-rules), and this
// server answers on :443 and :80. Each fixture page of the scenario is served at its real URL (meta.json),
// the bundle at https://bundles.rex.test/v1/<channel>/ (with ETags), and everything else is a 404, so no
// request leaves the container.
//
// Usage: node fixture-server.mjs "<scenario.json> [...]" <repo> <bundle dir> <cert.pem> <key.pem>
//        node fixture-server.mjs --hosts "<scenario.json> [...]" <repo>   (hostnames the certificate must cover)
// It runs from the mounted repo (/repo/extension/recorder/docker/), so it shares lib/scenario.mjs with run.mjs.
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import http from "node:http";
import https from "node:https";
import { join } from "node:path";
import { gunzipSync } from "node:zlib";
import { fixturePages, loadScenario } from "../lib/scenario.mjs";

export const BUNDLE_HOST = "bundles.rex.test";

/** Every fixture page of the given scenarios (space-separated files). */
function pages(scenarioFiles, repo) {
  const merged = { scenes: scenarioFiles.split(/\s+/).filter(Boolean).flatMap((f) => loadScenario(f, repo).scenes) };
  const out = new Map();
  for (const [key, { file }] of fixturePages(merged, repo)) {
    out.set(key, file.endsWith(".gz") ? gunzipSync(readFileSync(file)) : readFileSync(file));
  }
  return out;
}

if (process.argv[2] === "--hosts") {
  const hosts = new Set([BUNDLE_HOST, ...[...pages(process.argv[3], process.argv[4]).keys()].map((k) => k.split("/")[0])]);
  console.log([...hosts].join("\n"));
  process.exit(0);
}

const [scenarioFiles, repo, bundleDir, certFile, keyFile] = process.argv.slice(2);
const byUrl = pages(scenarioFiles, repo);

function bundleFile(path) {
  const f = join(bundleDir, path);
  return f.startsWith(bundleDir) && existsSync(f) && statSync(f).isFile() ? readFileSync(f) : null;
}

function handle(req, res) {
  const host = (req.headers.host ?? "").replace(/:\d+$/, "");
  const url = new URL(req.url, `https://${host}`);
  if (host === BUNDLE_HOST) {
    const body = bundleFile(decodeURIComponent(url.pathname));
    if (!body) return res.writeHead(404).end();
    const etag = `"${createHash("sha256").update(body).digest("hex").slice(0, 16)}"`;
    if (req.headers["if-none-match"] === etag) return res.writeHead(304, { etag }).end();
    const type = url.pathname.endsWith(".json") ? "application/json" : "application/octet-stream";
    return res.writeHead(200, { "content-type": type, etag, "cache-control": "no-cache" }).end(body);
  }
  const html = byUrl.get(`${host}${url.pathname}${url.search}`);
  if (html) {
    return res.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end(html);
  }
  res.writeHead(404, { "content-type": "text/plain" }).end("not part of the fixture set");
}

https.createServer({ cert: readFileSync(certFile), key: readFileSync(keyFile) }, handle).listen(443, "127.0.0.1");
http.createServer(handle).listen(80, "127.0.0.1");
console.log(`fixture-server: ${byUrl.size} pages, bundle from ${bundleDir} (${readdirSync(bundleDir).length} entries)`);
