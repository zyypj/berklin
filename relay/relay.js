"use strict";

const http = require("http");
const { WebSocketServer } = require("ws");

const PORT = Number(process.env.SERVER_PORT || process.env.PORT || 8787);
const HOST = process.env.BIND_HOST || "0.0.0.0";
const TOKEN_FILE = require("path").join(__dirname, "token.txt");
const TOKEN = (process.env.RELAY_TOKEN || readTokenFile() || createTokenFile()).trim();

function readTokenFile() {
  try {
    return require("fs").readFileSync(TOKEN_FILE, "utf8").trim();
  } catch {
    return "";
  }
}

function createTokenFile() {
  const token = require("crypto").randomBytes(24).toString("base64url");
  try {
    require("fs").writeFileSync(TOKEN_FILE, token + "\n", { mode: 0o600 });
    console.log("token.txt criado com um token novo; copie o conteúdo dele para o Berklin no PC");
  } catch {
    console.error("não consegui gravar token.txt");
  }
  return token;
}
const MAX_CLIENTS_PER_ROOM = Number(process.env.MAX_CLIENTS || 8);
const MAX_ROOMS = Number(process.env.MAX_ROOMS || 64);
const MAX_FRAME = 8 * 1024 * 1024;
const PING_MS = 25000;

if (!TOKEN) {
  console.error("Defina RELAY_TOKEN ou crie token.txt ao lado do relay.js");
  process.exit(1);
}

const rooms = new Map();

function roomFor(id) {
  let room = rooms.get(id);
  if (!room) {
    room = { agent: null, clients: new Map() };
    rooms.set(id, room);
  }
  return room;
}

function dropRoomIfEmpty(id, room) {
  if (!room.agent && room.clients.size === 0) rooms.delete(id);
}

function sendText(ws, obj) {
  if (ws && ws.readyState === 1) ws.send(JSON.stringify(obj));
}

function header(dev) {
  const id = Buffer.from(dev, "ascii");
  const out = Buffer.allocUnsafe(1 + id.length);
  out[0] = id.length;
  id.copy(out, 1);
  return out;
}

function timingSafeEqual(a, b) {
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  if (x.length !== y.length) return false;
  return require("crypto").timingSafeEqual(x, y);
}

const server = http.createServer((req, res) => {
  if (req.url === "/health") {
    res.writeHead(200, { "content-type": "text/plain" });
    res.end("ok");
    return;
  }
  res.writeHead(404);
  res.end();
});

const wss = new WebSocketServer({ noServer: true, maxPayload: MAX_FRAME, perMessageDeflate: false });

server.on("upgrade", (req, socket, head) => {
  const url = new URL(req.url, "http://relay");
  if (url.pathname !== "/ws") {
    socket.destroy();
    return;
  }
  wss.handleUpgrade(req, socket, head, (ws) => accept(ws, url.searchParams));
});

function accept(ws, params) {
  const room = params.get("room") || "";
  const role = params.get("role") || "";
  const dev = params.get("dev") || "";
  const token = params.get("t") || "";

  if (!timingSafeEqual(token, TOKEN)) {
    ws.close(4001, "token");
    return;
  }
  if (!/^[0-9a-f]{32}$/.test(room) || !/^[A-Za-z0-9_-]{1,64}$/.test(dev) || (role !== "agent" && role !== "client")) {
    ws.close(4002, "params");
    return;
  }
  if (!rooms.has(room) && rooms.size >= MAX_ROOMS) {
    ws.close(4003, "rooms");
    return;
  }

  const r = roomFor(room);
  ws.isAlive = true;
  ws.on("pong", () => { ws.isAlive = true; });

  if (role === "agent") {
    if (r.agent) r.agent.close(4000, "replaced");
    r.agent = ws;
    for (const [id, c] of r.clients) {
      sendText(c, { relay: "agent", online: true });
      sendText(ws, { relay: "client", dev: id, online: true });
    }
    ws.on("message", (data, isBinary) => {
      if (!isBinary || data.length < 1) return;
      const n = data[0];
      if (data.length < 1 + n) return;
      const payload = data.subarray(1 + n);
      if (n === 0) {
        for (const c of r.clients.values()) if (c.readyState === 1) c.send(payload, { binary: true });
      } else {
        const target = r.clients.get(data.subarray(1, 1 + n).toString("ascii"));
        if (target && target.readyState === 1) target.send(payload, { binary: true });
      }
    });
    ws.on("close", () => {
      if (r.agent !== ws) return;
      r.agent = null;
      for (const c of r.clients.values()) sendText(c, { relay: "agent", online: false });
      dropRoomIfEmpty(room, r);
    });
  } else {
    const previous = r.clients.get(dev);
    if (previous) previous.close(4000, "replaced");
    else if (r.clients.size >= MAX_CLIENTS_PER_ROOM) {
      ws.close(4004, "full");
      return;
    }
    r.clients.set(dev, ws);
    const prefix = header(dev);
    sendText(ws, { relay: "agent", online: !!r.agent });
    sendText(r.agent, { relay: "client", dev, online: true });
    ws.on("message", (data, isBinary) => {
      if (!isBinary || !r.agent || r.agent.readyState !== 1) return;
      r.agent.send(Buffer.concat([prefix, data]), { binary: true });
    });
    ws.on("close", () => {
      if (r.clients.get(dev) !== ws) return;
      r.clients.delete(dev);
      sendText(r.agent, { relay: "client", dev, online: false });
      dropRoomIfEmpty(room, r);
    });
  }
  ws.on("error", () => ws.terminate());
}

const heartbeat = setInterval(() => {
  for (const ws of wss.clients) {
    if (!ws.isAlive) {
      ws.terminate();
      continue;
    }
    ws.isAlive = false;
    ws.ping();
  }
}, PING_MS);

wss.on("close", () => clearInterval(heartbeat));

const VERSION = typeof __BERKLIN_VERSION__ === "string" ? __BERKLIN_VERSION__ : "dev";

server.listen(PORT, HOST, () => {
  console.log(`berklin relay ${VERSION} rodando em ${HOST}:${PORT}`);
});

process.stdin.setEncoding("utf8");
process.stdin.on("data", (line) => {
  if (/^\s*(stop|exit|end)\s*$/i.test(line)) shutdown();
});
process.stdin.on("error", () => {});

function shutdown() {
  for (const ws of wss.clients) ws.close(1001, "shutdown");
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 2000).unref();
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
