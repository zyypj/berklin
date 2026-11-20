"use strict";

const { fork, spawnSync } = require("child_process");
const fs = require("fs");
const https = require("https");

const VERSION = typeof __BERKLIN_VERSION__ === "string" ? __BERKLIN_VERSION__ : "dev";
const REPO = process.env.BERKLIN_REPO || "zyypj/berklin";
const CHECK_MS = Math.max(1, Number(process.env.BERKLIN_UPDATE_HOURS || 6)) * 3600 * 1000;
const AUTO_UPDATE = process.env.BERKLIN_RELAY_AUTOUPDATE !== "0";
const MAX_DOWNLOAD = 20 * 1024 * 1024;

if (process.env.BERKLIN_RELAY_CHILD === "1") {
  require("./relay.js");
} else {
  supervise();
}

function parseVersion(v) {
  return String(v)
    .replace(/^v/, "")
    .split(/[.-]/)
    .slice(0, 3)
    .map((n) => parseInt(n, 10) || 0);
}

function isNewer(candidate, current) {
  const a = parseVersion(candidate);
  const b = parseVersion(current);
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

function download(url, accept, redirects = 5) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { "User-Agent": "Berklin-Relay", Accept: accept } }, (res) => {
      if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location && redirects > 0) {
        res.resume();
        resolve(download(new URL(res.headers.location, url).toString(), accept, redirects - 1));
        return;
      }
      if (res.statusCode !== 200) {
        res.resume();
        reject(new Error(`HTTP ${res.statusCode} em ${url}`));
        return;
      }
      const chunks = [];
      let size = 0;
      res.on("data", (c) => {
        size += c.length;
        if (size > MAX_DOWNLOAD) {
          req.destroy(new Error("download grande demais"));
          return;
        }
        chunks.push(c);
      });
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    });
    req.on("error", reject);
    req.setTimeout(60000, () => req.destroy(new Error("tempo esgotado")));
  });
}

function supervise() {
  const self = __filename;
  let child = null;
  let stopping = false;
  let current = VERSION;
  let checking = false;

  const start = () => {
    child = fork(self, [], { env: { ...process.env, BERKLIN_RELAY_CHILD: "1" }, stdio: ["ignore", "inherit", "inherit", "ipc"] });
    child.on("exit", (code, signal) => {
      child = null;
      if (stopping) {
        process.exit(0);
        return;
      }
      console.log(`processo do relay saiu (${code ?? signal}); reiniciando`);
      setTimeout(start, 2000);
    });
  };

  const stop = () => {
    if (stopping) return;
    stopping = true;
    if (!child) process.exit(0);
    child.kill("SIGTERM");
    setTimeout(() => process.exit(0), 4000).unref();
  };

  const check = async () => {
    if (!AUTO_UPDATE || current === "dev" || checking) return;
    checking = true;
    try {
      const release = JSON.parse((await download(`https://api.github.com/repos/${REPO}/releases/latest`, "application/vnd.github+json")).toString("utf8"));
      const latest = String(release.tag_name || "").replace(/^v/, "");
      if (!isNewer(latest, current)) return;
      const asset = (release.assets || []).find((a) => /relay-server\.js$/.test(a.name));
      if (!asset) return;
      const body = await download(asset.browser_download_url, "application/octet-stream");
      const next = self.replace(/\.js$/, "") + ".next.js";
      fs.writeFileSync(next, body);
      if (spawnSync(process.execPath, ["--check", next], { stdio: "ignore" }).status !== 0) {
        fs.rmSync(next, { force: true });
        console.error(`atualização ${latest} do relay veio inválida; mantendo ${current}`);
        return;
      }
      fs.copyFileSync(self, `${self}.bak`);
      fs.renameSync(next, self);
      console.log(`relay atualizado de ${current} para ${latest}; reiniciando o processo do relay`);
      current = latest;
      if (child) child.kill("SIGTERM");
      else start();
    } catch (e) {
      console.error(`não consegui verificar atualização do relay: ${e.message}`);
    } finally {
      checking = false;
    }
  };

  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (line) => {
    const cmd = line.trim().toLowerCase();
    if (cmd === "stop" || cmd === "exit" || cmd === "end") stop();
    else if (cmd === "update") check();
  });
  process.stdin.on("error", () => {});
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);

  console.log(`berklin relay supervisor ${current}${AUTO_UPDATE ? " (atualização automática ligada)" : ""}`);
  start();
  setTimeout(check, 30000);
  setInterval(check, CHECK_MS);
}
