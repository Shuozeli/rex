// The extension's update server inside the desktop (127.0.0.1 only): /updates.xml (the gupdate manifest that
// ExtensionSettings.update_url points at) and /rex.crx (the CRX packed from /ext at start).
// Usage: node ext-server.mjs <crx> <extension id> <version> [port]
import { readFileSync } from "node:fs";
import http from "node:http";

const [crx, id, version, port = "8099"] = process.argv.slice(2);
const base = `http://127.0.0.1:${port}`;
const manifest = `<?xml version='1.0' encoding='UTF-8'?>
<gupdate xmlns='http://www.google.com/update2/response' protocol='2.0'>
  <app appid='${id}'>
    <updatecheck codebase='${base}/rex.crx' version='${version}' />
  </app>
</gupdate>
`;

http
  .createServer((req, res) => {
    const path = new URL(req.url, base).pathname;
    console.log(`ext-server: ${req.method} ${path}`);
    if (path === "/updates.xml") {
      res.writeHead(200, { "content-type": "text/xml" });
      res.end(manifest);
    } else if (path === "/rex.crx") {
      const body = readFileSync(crx);
      res.writeHead(200, { "content-type": "application/x-chrome-extension", "content-length": body.length });
      res.end(body);
    } else {
      res.writeHead(404);
      res.end();
    }
  })
  .listen(Number(port), "127.0.0.1", () => console.log(`ext-server: ${id} ${version} on ${base}`));
