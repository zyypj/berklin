import { useEffect, useReducer } from "preact/hooks";

export const S = {
  platform: "web",
  conn: { relay: "connecting", agent: false },
  ready: false,
  pc: { name: "" },
  away: { on: false, auto: false },
  sessions: new Map(),
  pending: new Map(),
  schedules: new Map(),
  notes: [],
  items: new Map(),
  view: { name: "home" },
  stack: [],
  toast: null,
  config: null,
  native: null,
  appUpdate: null,
};

const listeners = new Set();
let scheduled = false;

export function changed() {
  if (scheduled) return;
  scheduled = true;
  const flush = () => {
    scheduled = false;
    listeners.forEach((l) => l());
  };
  if (document.hidden) setTimeout(flush, 0);
  else requestAnimationFrame(flush);
}

export function useStore() {
  const [, force] = useReducer((x) => x + 1, 0);
  useEffect(() => {
    listeners.add(force);
    return () => listeners.delete(force);
  }, []);
  return S;
}

let transport = { send() {} };
let nextId = 1;
const waiting = new Map();

export function req(op, args = {}, timeout = 45000) {
  return new Promise((resolve, reject) => {
    const id = nextId++;
    waiting.set(id, { resolve, reject });
    try {
      transport.send({ id, op, ...args });
    } catch (e) {
      waiting.delete(id);
      reject(e);
      return;
    }
    setTimeout(() => {
      if (waiting.delete(id)) reject(new Error("O PC não respondeu."));
    }, timeout);
  });
}

let toastTimer = 0;
export function toast(text, kind = "info") {
  S.toast = { text, kind };
  changed();
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    S.toast = null;
    changed();
  }, 3200);
}

export function fail(e) {
  toast(e && e.message ? e.message : String(e), "error");
}

function setConn(c) {
  S.conn = { relay: c.relay || "offline", agent: !!c.agent };
  changed();
}

function upsertItems(sid, items) {
  const cur = S.items.get(sid);
  if (!cur || cur.loading) return;
  const last = cur.list.length ? cur.list[cur.list.length - 1].i : -1;
  const fresh = items.filter((it) => it.i > last);
  if (!fresh.length) return;
  S.items.set(sid, { ...cur, list: cur.list.concat(fresh) });
}

function onEvent(m) {
  switch (m.ev) {
    case "session":
      if (m.s) S.sessions.set(m.s.sid, m.s);
      break;
    case "items":
      upsertItems(m.sid, m.items || []);
      break;
    case "pending":
      S.pending.set(m.p.pid, m.p);
      break;
    case "pending.done":
      S.pending.delete(m.pid);
      break;
    case "note":
      if (!S.notes.some((n) => n.nid === m.n.nid)) S.notes = [m.n, ...S.notes].slice(0, 60);
      break;
    case "schedule":
      S.schedules.set(m.s.id, m.s);
      break;
    case "schedule.gone":
      S.schedules.delete(m.id);
      break;
    case "away":
      S.away = m.a;
      break;
    default:
      return;
  }
  changed();
}

function onMessage(m) {
  if (m && m.re !== undefined) {
    const w = waiting.get(m.re);
    if (!w) return;
    waiting.delete(m.re);
    if (m.ok) w.resolve(m.data);
    else w.reject(new Error(m.err || "erro"));
    return;
  }
  if (m && m.ev) onEvent(m);
}

function applySnapshot(d) {
  S.pc = d.pc || S.pc;
  S.away = d.away || S.away;
  for (const s of d.sessions || []) S.sessions.set(s.sid, s);
  S.pending = new Map((d.pending || []).map((p) => [p.pid, p]));
  S.schedules = new Map((d.schedules || []).map((s) => [s.id, s]));
  const known = new Set(S.notes.map((n) => n.nid));
  const extra = (d.notes || []).filter((n) => !known.has(n.nid)).reverse();
  S.notes = [...extra, ...S.notes].slice(0, 60);
  S.ready = true;
  changed();
  if (S.view.name === "session" && S.view.sid) loadSession(S.view.sid, true);
}

export function hello() {
  const name = S.platform === "android" ? "Android" : "Windows";
  req("hello", { dev: "ui", name, platform: S.platform })
    .then(applySnapshot)
    .catch(() => {});
}

export function loadSession(sid, force = false) {
  const cur = S.items.get(sid);
  if (cur && !force && !cur.loading) return;
  S.items.set(sid, { list: cur ? cur.list : [], more: false, loading: true });
  changed();
  req("session.open", { sid, limit: 160 })
    .then((d) => {
      S.items.set(sid, { list: d.items || [], more: !!d.more, loading: false });
      changed();
    })
    .catch((e) => {
      S.items.set(sid, { list: [], more: false, loading: false, error: e.message });
      changed();
    });
}

export function loadOlder(sid) {
  const cur = S.items.get(sid);
  if (!cur || !cur.list.length || cur.loadingOlder) return Promise.resolve();
  S.items.set(sid, { ...cur, loadingOlder: true });
  changed();
  return req("session.older", { sid, before: cur.list[0].i, limit: 160 }).then((d) => {
    const now = S.items.get(sid) || cur;
    S.items.set(sid, { ...now, list: (d.items || []).concat(now.list), more: !!d.more, loadingOlder: false });
    changed();
  });
}

export function navigate(view, replace = false) {
  if (S.view.name === "session" && S.view.sid && (view.name !== "session" || view.sid !== S.view.sid)) {
    req("session.close", { sid: S.view.sid }).catch(() => {});
    S.items.delete(S.view.sid);
  }
  if (!replace) S.stack.push(S.view);
  S.view = view;
  if (view.name === "session") loadSession(view.sid, true);
  changed();
}

export function goBack() {
  if (!S.stack.length) {
    if (S.view.name === "home") return false;
    navigate({ name: "home" }, true);
    return true;
  }
  const prev = S.stack.pop();
  navigate(prev, true);
  return true;
}

export function openSession(sid) {
  if (!sid) return;
  navigate({ name: "session", sid });
}

export function nativeConfig() {
  try {
    return JSON.parse(window.BerklinNative.config());
  } catch {
    return { paired: false };
  }
}

function startLocal(cfg) {
  let ws = null;
  let backoff = 400;
  const connect = () => {
    setConn({ relay: "connecting", agent: false });
    ws = new WebSocket(`ws://127.0.0.1:${cfg.port}/ws?t=${encodeURIComponent(cfg.token)}`);
    ws.onopen = () => {
      backoff = 400;
      setConn({ relay: "online", agent: true });
      hello();
    };
    ws.onmessage = (e) => {
      try {
        onMessage(JSON.parse(e.data));
      } catch {}
    };
    ws.onclose = () => {
      setConn({ relay: "offline", agent: false });
      setTimeout(connect, (backoff = Math.min(backoff * 2, 4000)));
    };
  };
  connect();
  return {
    send(o) {
      if (!ws || ws.readyState !== 1) throw new Error("Sem conexão com o agente.");
      ws.send(JSON.stringify(o));
    },
  };
}

function parse(j) {
  return typeof j === "string" ? JSON.parse(j) : j;
}

function startNative() {
  window.__berklin = {
    onMessage(j) {
      try {
        onMessage(parse(j));
      } catch {}
    },
    onStatus(j) {
      const s = parse(j);
      const was = S.conn;
      setConn(s);
      if (s.relay === "online" && s.agent && !(was.relay === "online" && was.agent)) hello();
    },
    onOpen(sid) {
      openSession(sid);
    },
    onUpdate(j) {
      try {
        S.appUpdate = parse(j);
        changed();
      } catch {}
    },
    onBack() {
      return goBack();
    },
  };
  S.native = nativeConfig();
  return {
    send(o) {
      window.BerklinNative.send(JSON.stringify(o));
    },
  };
}

export function boot() {
  if (window.BerklinNative) {
    S.platform = "android";
    transport = startNative();
    hello();
    return;
  }
  const q = new URLSearchParams(location.search);
  const local = window.__BERKLIN_LOCAL || (q.get("port") ? { port: q.get("port"), token: q.get("t") || "" } : null);
  S.platform = local && local.platform === "windows" ? "windows" : "web";
  if (local) transport = startLocal(local);
  else setConn({ relay: "offline", agent: false });
}
