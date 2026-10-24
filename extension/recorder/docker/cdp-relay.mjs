// Relays CDP from BIND:PORT to Chrome on 127.0.0.1:9222. Chrome refuses a Host header that is not an IP or
// localhost (DNS-rebinding protection), so requests by MagicDNS name get Host rewritten; the websocket upgrade
// is piped through unchanged otherwise.
import http from "node:http";
import net from "node:net";

const [bind, port] = [process.argv[2] ?? "127.0.0.1", Number(process.argv[3] ?? 9541)];
const TARGET = { host: "127.0.0.1", port: 9222 };
const publicHost = (req) => req.headers.host ?? `${bind}:${port}`;

const server = http.createServer((req, res) => {
  const up = http.request({ ...TARGET, method: req.method, path: req.url, headers: { ...req.headers, host: "127.0.0.1:9222" } }, (r) => {
    // /json/* answers name ws://127.0.0.1:9222/...; point clients back at this relay.
    const chunks = [];
    r.on("data", (c) => chunks.push(c));
    r.on("end", () => {
      let body = Buffer.concat(chunks);
      if (/json/.test(r.headers["content-type"] ?? "")) {
        body = Buffer.from(body.toString("utf8").replaceAll("127.0.0.1:9222", publicHost(req)).replaceAll("localhost:9222", publicHost(req)));
      }
      const headers = { ...r.headers, "content-length": body.length };
      res.writeHead(r.statusCode ?? 502, headers);
      res.end(body);
    });
  });
  up.on("error", () => (res.writeHead(502), res.end()));
  req.pipe(up);
});

server.on("upgrade", (req, socket, head) => {
  const up = net.connect(TARGET, () => {
    const lines = [`${req.method} ${req.url} HTTP/1.1`];
    for (const [k, v] of Object.entries(req.headers)) lines.push(`${k}: ${k === "host" ? "127.0.0.1:9222" : v}`);
    up.write(lines.join("\r\n") + "\r\n\r\n");
    if (head.length) up.write(head);
    up.pipe(socket);
    socket.pipe(up);
  });
  const close = () => (up.destroy(), socket.destroy());
  up.on("error", close);
  socket.on("error", close);
});

server.listen(port, bind, () => console.log(`cdp relay on ${bind}:${port}`));
