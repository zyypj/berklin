import { render } from "preact";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import "./style.css";
import { S, boot, changed, fail, goBack, loadOlder, navigate, nativeConfig, openSession, req, toast, useStore } from "./store.js";
import { Markdown, Transcript, highlight, shortPath, toolSummary } from "./render.jsx";
import jsQR from "jsqr";

const MODES = [
  ["default", "Perguntar permissões"],
  ["acceptEdits", "Aceitar edições"],
  ["plan", "Só planejar"],
  ["auto", "Automático"],
  ["bypassPermissions", "Sem perguntas (perigoso)"],
];

const DOW = ["Domingo", "Segunda", "Terça", "Quarta", "Quinta", "Sexta", "Sábado"];

function rel(ms) {
  if (!ms) return "";
  const d = Date.now() - ms;
  if (d < 45000) return "agora";
  if (d < 3600000) return `${Math.round(d / 60000)} min`;
  if (d < 86400000) return `${Math.round(d / 3600000)} h`;
  if (d < 7 * 86400000) return `${Math.round(d / 86400000)} d`;
  return new Date(ms).toLocaleDateString("pt-BR", { day: "2-digit", month: "2-digit" });
}

function when(ms) {
  if (!ms) return "—";
  return new Date(ms).toLocaleString("pt-BR", { weekday: "short", day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });
}

function fmtTokens(n) {
  if (!n) return "";
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 100000 ? 0 : 1)}k tokens` : `${n} tokens`;
}

function copy(text) {
  const done = () => toast("Copiado");
  if (navigator.clipboard && window.isSecureContext) {
    navigator.clipboard.writeText(text).then(done, () => fallbackCopy(text, done));
  } else fallbackCopy(text, done);
}

function fallbackCopy(text, done) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand("copy");
    done();
  } catch {}
  ta.remove();
}

document.addEventListener("click", (e) => {
  const btn = e.target.closest && e.target.closest("[data-copy]");
  if (btn) {
    const code = btn.closest("pre")?.querySelector("code");
    if (code) copy(code.innerText);
    e.preventDefault();
    return;
  }
  const a = e.target.closest && e.target.closest("a[href]");
  if (a && a.getAttribute("href") !== "#") {
    e.preventDefault();
    copy(a.href);
  }
});

const STATUS_LABEL = { running: "Rodando", waiting: "Esperando você", idle: "Parada" };

function Dot({ status, live }) {
  return <span class={`dot ${status || "idle"}${live ? " live" : ""}`} />;
}

function Switch({ on, onChange, label }) {
  return (
    <button type="button" class={"switch" + (on ? " on" : "")} role="switch" aria-checked={on} aria-label={label} onClick={() => onChange(!on)}>
      <span />
    </button>
  );
}

function Empty({ title, children }) {
  return (
    <div class="empty">
      <b>{title}</b>
      {children ? <p>{children}</p> : null}
    </div>
  );
}

function sessionsSorted() {
  return [...S.sessions.values()].sort((a, b) => b.updated - a.updated);
}

function SessionRow({ s, active }) {
  return (
    <button type="button" class={"srow" + (active ? " active" : "")} onClick={() => openSession(s.sid)}>
      <Dot status={s.status} live={s.live} />
      <div class="srow-main">
        <div class="srow-top">
          <span class="srow-title">{s.title}</span>
          <span class="srow-time">{rel(s.updated)}</span>
        </div>
        <div class="srow-sub">
          <span class="chip">{s.project || "—"}</span>
          {s.status !== "idle" ? <span class={"chip st " + s.status}>{STATUS_LABEL[s.status]}</span> : null}
          {s.headless ? <span class="chip">2º plano</span> : null}
          <span class="srow-prev">{s.preview}</span>
        </div>
      </div>
    </button>
  );
}

function QuestionForm({ p, busy, resolve }) {
  const qs = p.questions || [];
  const [sel, setSel] = useState({});
  const [other, setOther] = useState({});
  const single = qs.length === 1 && !qs[0].multiSelect;
  const pick = (q, label) => {
    if (single) {
      resolve({ answers: { [q.question]: label } });
      return;
    }
    const cur = sel[q.question] || [];
    const next = q.multiSelect ? (cur.includes(label) ? cur.filter((x) => x !== label) : [...cur, label]) : [label];
    setSel({ ...sel, [q.question]: next });
  };
  const submit = () => {
    const answers = {};
    for (const q of qs) {
      const parts = [...(sel[q.question] || [])];
      if ((other[q.question] || "").trim()) parts.push(other[q.question].trim());
      if (!parts.length) {
        toast("Responda todas as perguntas", "error");
        return;
      }
      answers[q.question] = parts.join(", ");
    }
    resolve({ answers });
  };
  return (
    <div class="qform">
      {qs.map((q) => (
        <div class="q" key={q.question}>
          {q.header ? <span class="chip">{q.header}</span> : null}
          <p class="q-text">{q.question}</p>
          <div class="opts">
            {(q.options || []).map((o) => {
              const on = (sel[q.question] || []).includes(o.label);
              return (
                <button type="button" key={o.label} class={"opt" + (on ? " on" : "")} disabled={busy} onClick={() => pick(q, o.label)}>
                  <b>{o.label}</b>
                  {o.description ? <span>{o.description}</span> : null}
                </button>
              );
            })}
          </div>
          <input
            class="input"
            placeholder={q.options && q.options.length ? "Outra resposta…" : "Sua resposta…"}
            value={other[q.question] || ""}
            onInput={(e) => setOther({ ...other, [q.question]: e.currentTarget.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter" && single && e.currentTarget.value.trim()) resolve({ answers: { [q.question]: e.currentTarget.value.trim() } });
            }}
          />
        </div>
      ))}
      <div class="row-end">
        <button type="button" class="btn ghost" disabled={busy} onClick={() => resolve({ release: true })}>Responder no PC</button>
        <button type="button" class="btn primary" disabled={busy} onClick={submit}>Responder</button>
      </div>
    </div>
  );
}

function PendingCard({ p, showSession = true }) {
  const [busy, setBusy] = useState(false);
  const [text, setText] = useState("");
  const [denyOpen, setDenyOpen] = useState(false);
  const resolve = (args) => {
    setBusy(true);
    req("pending.resolve", { pid: p.pid, ...args })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  const label = p.kind === "permission" ? "Permissão" : p.kind === "question" ? "Pergunta" : "Terminou";
  const input = p.input || {};
  const cmd = input.command || input.file_path || input.url || input.query;
  return (
    <div class={"pcard " + p.kind}>
      <div class="pcard-h">
        <span class={"badge " + p.kind}>{label}</span>
        {showSession ? (
          <button type="button" class="link" onClick={() => p.sid && openSession(p.sid)}>
            {p.title}
          </button>
        ) : null}
        <span class="muted">{rel(p.ts)}</span>
      </div>
      {p.kind === "permission" ? (
        <div>
          <p class="pcard-t">
            Claude quer usar <b>{p.tool}</b>
            {toolSummary(p.tool, input) && !cmd ? <> · {toolSummary(p.tool, input)}</> : null}
          </p>
          {cmd ? (
            <pre class="code small">
              <code class="hljs" dangerouslySetInnerHTML={{ __html: highlight(String(cmd), p.tool === "Bash" ? "bash" : p.tool === "PowerShell" ? "powershell" : "") }} />
            </pre>
          ) : (
            <pre class="code small">
              <code>{JSON.stringify(input, null, 2).slice(0, 1500)}</code>
            </pre>
          )}
          {input.old_string !== undefined || input.content !== undefined ? (
            <details class="more">
              <summary>Ver conteúdo</summary>
              <pre class="code small">
                <code>{(input.content || `- ${input.old_string}\n+ ${input.new_string}`).slice(0, 6000)}</code>
              </pre>
            </details>
          ) : null}
          {denyOpen ? (
            <div class="stack">
              <textarea class="input" rows={2} placeholder="O que o Claude deve fazer no lugar?" value={text} onInput={(e) => setText(e.currentTarget.value)} />
              <div class="row-end">
                <button type="button" class="btn ghost" onClick={() => setDenyOpen(false)}>Voltar</button>
                <button type="button" class="btn danger" disabled={busy} onClick={() => resolve({ allow: false, message: text.trim() || undefined })}>Negar e instruir</button>
              </div>
            </div>
          ) : (
            <div class="row-end">
              <button type="button" class="btn ghost" disabled={busy} onClick={() => resolve({ release: true })}>Decidir no PC</button>
              <button type="button" class="btn danger" disabled={busy} onClick={() => setDenyOpen(true)}>Negar</button>
              <button type="button" class="btn primary" disabled={busy} onClick={() => resolve({ allow: true })}>Permitir</button>
            </div>
          )}
        </div>
      ) : null}
      {p.kind === "question" ? <QuestionForm p={p} busy={busy} resolve={resolve} /> : null}
      {p.kind === "stop" ? (
        <div>
          {p.message ? (
            <div class="stop-msg">
              <Markdown text={p.message} />
            </div>
          ) : null}
          <textarea class="input" rows={2} placeholder="Responder para o Claude continuar…" value={text} onInput={(e) => setText(e.currentTarget.value)} />
          <div class="row-end">
            <button type="button" class="btn ghost" disabled={busy} onClick={() => resolve({ release: true })}>Encerrar por aqui</button>
            <button type="button" class="btn primary" disabled={busy || !text.trim()} onClick={() => resolve({ text: text.trim() })}>Enviar</button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function AwayCard() {
  const s = S;
  return (
    <div class="away-card">
      <div>
        <b>Modo ausente</b>
        <p class="muted">
          {s.away.on
            ? s.away.auto
              ? "Ligado automaticamente: o PC está ocioso. Perguntas, permissões e finais de tarefa vêm para cá."
              : "Ligado: perguntas, permissões e finais de tarefa vêm para cá."
            : "Desligado: o Claude pergunta na tela do PC. Liga sozinho quando o PC fica ocioso."}
        </p>
      </div>
      <Switch on={s.away.on} label="Modo ausente" onChange={(on) => req("away.set", { on }).catch(fail)} />
    </div>
  );
}

function Home() {
  const s = useStore();
  const pending = [...s.pending.values()].sort((a, b) => b.ts - a.ts);
  const all = sessionsSorted();
  const active = all.filter((x) => x.status !== "idle" || x.headless);
  const recent = all.filter((x) => x.status === "idle" && !x.headless).slice(0, 8);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const todayCount = all.filter((x) => x.updated >= today.getTime()).length;
  const next = [...s.schedules.values()].filter((x) => x.enabled && x.nextRun).sort((a, b) => a.nextRun - b.nextRun)[0];
  return (
    <div class="page">
      <AwayCard />
      <div class="tiles">
        <div class="tile">
          <span class="tile-n">{active.filter((x) => x.status === "running").length}</span>
          <span class="tile-l">rodando</span>
        </div>
        <div class={"tile" + (pending.length ? " warn" : "")}>
          <span class="tile-n">{pending.length}</span>
          <span class="tile-l">esperando você</span>
        </div>
        <div class="tile">
          <span class="tile-n">{todayCount}</span>
          <span class="tile-l">sessões hoje</span>
        </div>
        <button type="button" class="tile" onClick={() => navigate({ name: "schedules" })}>
          <span class="tile-n small">{next ? when(next.nextRun) : "—"}</span>
          <span class="tile-l">{next ? next.name : "nenhum agendamento"}</span>
        </button>
      </div>
      {pending.length ? (
        <section>
          <h2>Precisa de você</h2>
          <div class="stack">
            {pending.map((p) => (
              <PendingCard key={p.pid} p={p} />
            ))}
          </div>
        </section>
      ) : null}
      <section>
        <div class="sec-h">
          <h2>Em andamento</h2>
          <button type="button" class="btn small primary" onClick={() => navigate({ name: "new" })}>
            <Icon n="plus" s={15} /> Novo chat
          </button>
        </div>
        {active.length ? (
          <div class="list">
            {active.map((x) => (
              <SessionRow key={x.sid} s={x} />
            ))}
          </div>
        ) : (
          <Empty title="Nenhum Claude trabalhando agora" />
        )}
      </section>
      <section>
        <div class="sec-h">
          <h2>Recentes</h2>
          <button type="button" class="link" onClick={() => navigate({ name: "sessions" })}>Ver todas</button>
        </div>
        <div class="list">
          {recent.map((x) => (
            <SessionRow key={x.sid} s={x} />
          ))}
        </div>
      </section>
      {s.notes.length ? (
        <section>
          <h2>Avisos</h2>
          <div class="notes">
            {s.notes.slice(0, 8).map((n) => (
              <button type="button" key={n.nid} class={"note " + n.kind} onClick={() => n.sid && openSession(n.sid)}>
                <div class="note-top">
                  <b>{n.title}</b>
                  <span class="muted">{rel(n.ts)}</span>
                </div>
                {n.body ? <span class="note-b">{n.body}</span> : null}
              </button>
            ))}
          </div>
        </section>
      ) : null}
    </div>
  );
}

function Sessions() {
  const s = useStore();
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState("all");
  const [loadingMore, setLoadingMore] = useState(false);
  const all = sessionsSorted();
  const needle = q.trim().toLowerCase();
  const list = all.filter((x) => {
    if (filter === "active" && x.status === "idle" && !x.headless) return false;
    if (filter === "waiting" && x.status !== "waiting") return false;
    if (!needle) return true;
    return (x.title + " " + x.project + " " + x.preview + " " + x.cwd).toLowerCase().includes(needle);
  });
  const more = () => {
    const last = all[all.length - 1];
    if (!last) return;
    setLoadingMore(true);
    req("sessions.list", { before: last.updated, limit: 100 })
      .then((rows) => {
        for (const r of rows) S.sessions.set(r.sid, r);
        changed();
        if (!rows.length) toast("Não há mais sessões");
      })
      .catch(fail)
      .finally(() => setLoadingMore(false));
  };
  return (
    <div class="page">
      <div class="search">
        <input class="input" type="search" placeholder="Buscar por título, projeto ou texto…" value={q} onInput={(e) => setQ(e.currentTarget.value)} />
        <div class="chips">
          {[
            ["all", "Todas"],
            ["active", "Ativas"],
            ["waiting", "Esperando"],
          ].map(([k, l]) => (
            <button type="button" key={k} class={"chip btnchip" + (filter === k ? " on" : "")} onClick={() => setFilter(k)}>
              {l}
            </button>
          ))}
        </div>
      </div>
      {list.length ? (
        <div class="list">
          {list.map((x) => (
            <SessionRow key={x.sid} s={x} active={s.view.sid === x.sid} />
          ))}
        </div>
      ) : (
        <Empty title="Nada encontrado" />
      )}
      {!needle && filter === "all" ? (
        <button type="button" class="btn ghost wide" disabled={loadingMore} onClick={more}>
          {loadingMore ? "Carregando…" : "Carregar mais antigas"}
        </button>
      ) : null}
    </div>
  );
}

function composerHint(s, pend) {
  if (!s) return "";
  if (pend.some((p) => p.kind === "stop")) return "Sua mensagem continua a sessão ao vivo no PC.";
  if (s.headless) return s.status === "running" ? "Vai para a sessão em segundo plano assim que ela puder ler." : "Continua a sessão em segundo plano.";
  if (s.live && s.status === "running") return "O Claude está trabalhando: a mensagem entra na fila e é entregue quando ele parar.";
  if (s.live) return "Sessão aberta no PC e parada: a mensagem continua em segundo plano. Para continuar ao vivo, deixe o modo ausente ligado antes de sair.";
  return "Retoma esta sessão em segundo plano no PC.";
}

function Composer({ sid, session, pend }) {
  const [text, setText] = useState("");
  const [mode, setMode] = useState("");
  const [busy, setBusy] = useState(false);
  const ta = useRef(null);
  const resize = () => {
    const el = ta.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 220) + "px";
  };
  const send = () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    req("send", { sid, text: t, mode: mode || undefined }, 60000)
      .then((r) => {
        setText("");
        requestAnimationFrame(resize);
        const msg = { injected: "Enviado para a sessão", queued: "Na fila: entra quando o Claude parar", resumed: "Retomando em segundo plano", started: "Sessão iniciada" }[r.how];
        toast(msg || "Enviado");
      })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  const showMode = !(session && session.live && !session.headless);
  return (
    <div class="composer">
      <div class="composer-box">
        <textarea
          ref={ta}
          rows={1}
          placeholder="Mensagem para o Claude…"
          value={text}
          onInput={(e) => {
            setText(e.currentTarget.value);
            resize();
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && S.platform !== "android") {
              e.preventDefault();
              send();
            }
          }}
        />
        <button type="button" class="send" disabled={busy || !text.trim()} onClick={send} aria-label="Enviar">
          {busy ? <span class="spin light" /> : <Icon n="send" s={19} />}
        </button>
      </div>
      <div class="composer-meta">
        <span class="muted">{composerHint(session, pend)}</span>
        {showMode ? (
          <select class="mini-select" value={mode} onChange={(e) => setMode(e.currentTarget.value)}>
            <option value="">modo padrão</option>
            {MODES.map(([v, l]) => (
              <option key={v} value={v}>
                {l}
              </option>
            ))}
          </select>
        ) : null}
      </div>
    </div>
  );
}

function SessionView({ sid }) {
  const s = useStore();
  const session = s.sessions.get(sid);
  const data = s.items.get(sid);
  const list = data ? data.list : [];
  const pend = [...s.pending.values()].filter((p) => p.sid === sid);
  const scroller = useRef(null);
  const atBottom = useRef(true);
  const prevHeight = useRef(0);
  const restoring = useRef(false);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    if (restoring.current) {
      el.scrollTop = el.scrollHeight - prevHeight.current;
      restoring.current = false;
      return;
    }
    if (atBottom.current) el.scrollTop = el.scrollHeight;
  }, [list, pend.length]);

  useEffect(() => {
    atBottom.current = true;
  }, [sid]);

  const onScroll = (e) => {
    const el = e.currentTarget;
    atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };

  const older = () => {
    const el = scroller.current;
    prevHeight.current = el ? el.scrollHeight - el.scrollTop : 0;
    restoring.current = true;
    loadOlder(sid).catch(fail);
  };

  return (
    <div class="session">
      <div class="session-h">
        <div class="session-ht">
          <Dot status={session?.status} live={session?.live} />
          <div class="session-title">
            <b>{session ? session.title : "Sessão"}</b>
            <span class="muted">
              {session ? [shortPath(session.cwd), session.model, fmtTokens(session.tokens)].filter(Boolean).join(" · ") : ""}
            </span>
          </div>
        </div>
        <div class="session-actions">
          {session ? <span class={"chip st " + session.status}>{STATUS_LABEL[session.status]}</span> : null}
          {session?.live && !session.headless ? <span class="chip">aberta no PC</span> : null}
          {session?.headless ? (
            <button type="button" class="btn small danger" onClick={() => req("interrupt", { sid }).then(() => toast("Interrompido")).catch(fail)}>
              <Icon n="stop" s={14} /> Parar
            </button>
          ) : null}
        </div>
      </div>
      <div class="scroller" ref={scroller} onScroll={onScroll}>
        <div class="transcript">
          {data && data.more ? (
            <button type="button" class="btn ghost wide" disabled={data.loadingOlder} onClick={older}>
              {data.loadingOlder ? "Carregando…" : "Carregar mensagens anteriores"}
            </button>
          ) : null}
          {!data || (data.loading && !list.length) ? <div class="loading">Carregando conversa…</div> : null}
          {data && data.error ? <Empty title="Não consegui abrir">{data.error}</Empty> : null}
          {data && !data.loading && !list.length && !data.error ? <Empty title="Conversa vazia" /> : null}
          <Transcript list={list} />
          {session?.status === "running" ? (
            <div class="working">
              <span class="spin" /> Claude está trabalhando…
            </div>
          ) : null}
          {pend.map((p) => (
            <PendingCard key={p.pid} p={p} showSession={false} />
          ))}
        </div>
      </div>
      <Composer sid={sid} session={session} pend={pend} />
    </div>
  );
}

function useProjects() {
  const [projects, setProjects] = useState(null);
  useEffect(() => {
    req("projects.list")
      .then(setProjects)
      .catch((e) => {
        setProjects([]);
        fail(e);
      });
  }, []);
  return projects;
}

function FolderPicker({ value, onChange }) {
  const projects = useProjects();
  const known = projects && projects.some((p) => p.cwd === value);
  return (
    <div class="stack">
      <select class="input" value={known ? value : "__custom"} onChange={(e) => onChange(e.currentTarget.value === "__custom" ? "" : e.currentTarget.value)}>
        <option value="__custom">{projects ? "Outra pasta…" : "Carregando projetos…"}</option>
        {(projects || []).map((p) => (
          <option key={p.cwd} value={p.cwd}>
            {p.name} — {shortPath(p.cwd)}
          </option>
        ))}
      </select>
      {!known ? <input class="input" placeholder="C:\caminho\do\projeto" value={value} onInput={(e) => onChange(e.currentTarget.value)} /> : null}
    </div>
  );
}

function NewChat() {
  const [cwd, setCwd] = useState("");
  const [text, setText] = useState("");
  const [mode, setMode] = useState("default");
  const [busy, setBusy] = useState(false);
  const start = () => {
    setBusy(true);
    req("send", { cwd: cwd.trim(), text: text.trim(), mode }, 60000)
      .then((r) => {
        toast("Sessão iniciada");
        navigate({ name: "session", sid: r.sid }, true);
      })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  return (
    <div class="page narrow">
      <h1>Novo chat</h1>
      <p class="muted">Abre uma sessão nova do Claude Code no PC, em segundo plano. Você acompanha e conversa por aqui.</p>
      <label class="field">
        <span>Pasta do projeto</span>
        <FolderPicker value={cwd} onChange={setCwd} />
      </label>
      <label class="field">
        <span>O que o Claude deve fazer</span>
        <textarea class="input" rows={6} value={text} onInput={(e) => setText(e.currentTarget.value)} placeholder="Ex.: rode os testes e corrija o que falhar" />
      </label>
      <label class="field">
        <span>Permissões</span>
        <select class="input" value={mode} onChange={(e) => setMode(e.currentTarget.value)}>
          {MODES.map(([v, l]) => (
            <option key={v} value={v}>
              {l}
            </option>
          ))}
        </select>
      </label>
      <div class="row-end">
        <button type="button" class="btn primary" disabled={busy || !cwd.trim() || !text.trim()} onClick={start}>
          {busy ? "Iniciando…" : "Iniciar"}
        </button>
      </div>
    </div>
  );
}

function pad(n) {
  return String(n).padStart(2, "0");
}

function describeCron(c) {
  if (!c) return "";
  const p = c.trim().split(/\s+/);
  if (p.length !== 5) return `cron ${c}`;
  const [m, h, dom, mon, dow] = p;
  const t = /^\d+$/.test(m) && /^\d+$/.test(h) ? `${pad(h)}:${pad(m)}` : null;
  if (t && dom === "*" && mon === "*") {
    if (dow === "*") return `Todo dia às ${t}`;
    if (dow === "1-5") return `Dias úteis às ${t}`;
    if (/^\d$/.test(dow)) return `Toda ${DOW[+dow % 7].toLowerCase()} às ${t}`;
  }
  const hm = /^\*\/(\d+)$/.exec(h);
  if (m === "0" && hm && dom === "*" && mon === "*" && dow === "*") return `A cada ${hm[1]} h`;
  return `cron ${c}`;
}

function toLocalInput(ms) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function draftFrom(s) {
  const base = { id: "", name: "", kind: "claude", prompt: "", cwd: "", enabled: true, mode: "default", repeat: "once", date: toLocalInput(Date.now() + 3600000), time: "09:00", dow: "1", hours: "2", cron: "" };
  if (!s) return base;
  const d = { ...base, ...s, cron: s.cron || "" };
  if (s.at) d.date = toLocalInput(s.at);
  if (s.cron) {
    const p = s.cron.trim().split(/\s+/);
    const [m, h, dom, mon, dow] = p;
    const timed = p.length === 5 && /^\d+$/.test(m) && /^\d+$/.test(h) && dom === "*" && mon === "*";
    if (timed) d.time = `${pad(h)}:${pad(m)}`;
    if (timed && dow === "*") d.repeat = "daily";
    else if (timed && dow === "1-5") d.repeat = "weekdays";
    else if (timed && /^\d$/.test(dow)) {
      d.repeat = "weekly";
      d.dow = dow;
    } else if (/^\*\/(\d+)$/.test(h) && m === "0") {
      d.repeat = "hours";
      d.hours = h.slice(2);
    } else d.repeat = "cron";
  }
  return d;
}

function ScheduleEditor({ initial, onDone }) {
  const [d, setD] = useState(() => draftFrom(initial));
  const [busy, setBusy] = useState(false);
  const set = (k) => (e) => setD({ ...d, [k]: e.currentTarget.type === "checkbox" ? e.currentTarget.checked : e.currentTarget.value });
  const save = () => {
    const [hh, mm] = (d.time || "09:00").split(":").map(Number);
    const sched = { id: d.id || undefined, name: d.name, kind: d.kind, prompt: d.prompt, cwd: d.cwd, enabled: true, mode: d.mode };
    if (d.repeat === "once") sched.at = new Date(d.date).getTime();
    else if (d.repeat === "daily") sched.cron = `${mm} ${hh} * * *`;
    else if (d.repeat === "weekdays") sched.cron = `${mm} ${hh} * * 1-5`;
    else if (d.repeat === "weekly") sched.cron = `${mm} ${hh} * * ${d.dow}`;
    else if (d.repeat === "hours") sched.cron = `0 */${Math.max(1, Math.min(23, +d.hours || 1))} * * *`;
    else sched.cron = d.cron;
    setBusy(true);
    req("schedules.save", { schedule: sched })
      .then(() => {
        toast("Agendamento salvo");
        onDone();
      })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  return (
    <div class="editor">
      <div class="seg">
        {[
          ["claude", "Tarefa do Claude"],
          ["reminder", "Lembrete"],
        ].map(([k, l]) => (
          <button type="button" key={k} class={d.kind === k ? "on" : ""} onClick={() => setD({ ...d, kind: k })}>
            {l}
          </button>
        ))}
      </div>
      <label class="field">
        <span>Nome</span>
        <input class="input" value={d.name} onInput={set("name")} placeholder="Ex.: Revisar PRs da manhã" />
      </label>
      <label class="field">
        <span>{d.kind === "claude" ? "O que o Claude deve fazer" : "Texto do lembrete"}</span>
        <textarea class="input" rows={4} value={d.prompt} onInput={set("prompt")} />
      </label>
      {d.kind === "claude" ? (
        <>
          <label class="field">
            <span>Pasta</span>
            <FolderPicker value={d.cwd} onChange={(v) => setD({ ...d, cwd: v })} />
          </label>
          <label class="field">
            <span>Permissões</span>
            <select class="input" value={d.mode} onChange={set("mode")}>
              {MODES.map(([v, l]) => (
                <option key={v} value={v}>
                  {l}
                </option>
              ))}
            </select>
          </label>
        </>
      ) : null}
      <label class="field">
        <span>Quando</span>
        <select class="input" value={d.repeat} onChange={set("repeat")}>
          <option value="once">Uma vez</option>
          <option value="daily">Todo dia</option>
          <option value="weekdays">Dias úteis</option>
          <option value="weekly">Toda semana</option>
          <option value="hours">A cada N horas</option>
          <option value="cron">Cron personalizado</option>
        </select>
      </label>
      {d.repeat === "once" ? <input class="input" type="datetime-local" value={d.date} onInput={set("date")} /> : null}
      {["daily", "weekdays", "weekly"].includes(d.repeat) ? (
        <div class="row">
          {d.repeat === "weekly" ? (
            <select class="input" value={d.dow} onChange={set("dow")}>
              {DOW.map((n, i) => (
                <option key={i} value={String(i)}>
                  {n}
                </option>
              ))}
            </select>
          ) : null}
          <input class="input" type="time" value={d.time} onInput={set("time")} />
        </div>
      ) : null}
      {d.repeat === "hours" ? <input class="input" type="number" min="1" max="23" value={d.hours} onInput={set("hours")} /> : null}
      {d.repeat === "cron" ? <input class="input mono" value={d.cron} onInput={set("cron")} placeholder="min hora dia mês dia-da-semana (0=domingo)" /> : null}
      <div class="row-end">
        <button type="button" class="btn ghost" onClick={onDone}>Cancelar</button>
        <button type="button" class="btn primary" disabled={busy} onClick={save}>Salvar</button>
      </div>
    </div>
  );
}

function Schedules() {
  const s = useStore();
  const [editing, setEditing] = useState(null);
  const list = [...s.schedules.values()].sort((a, b) => (a.enabled === b.enabled ? (a.nextRun || 9e15) - (b.nextRun || 9e15) : a.enabled ? -1 : 1));
  if (editing) {
    return (
      <div class="page narrow">
        <h1>{editing.id ? "Editar agendamento" : "Novo agendamento"}</h1>
        <ScheduleEditor initial={editing.id ? editing : null} onDone={() => setEditing(null)} />
      </div>
    );
  }
  const toggle = (x) => req("schedules.save", { schedule: { ...x, enabled: !x.enabled } }).catch(fail);
  return (
    <div class="page">
      <div class="sec-h">
        <h1>Agenda</h1>
        <button type="button" class="btn primary small" onClick={() => setEditing({})}>
          <Icon n="plus" s={15} /> Agendar
        </button>
      </div>
      <p class="muted">Tarefas rodam no PC em segundo plano e avisam aqui quando começam e terminam. Você também pode pedir para qualquer chat do Claude agendar algo.</p>
      {list.length ? (
        <div class="stack">
          {list.map((x) => (
            <div class={"sched" + (x.enabled ? "" : " off")} key={x.id}>
              <div class="sched-h">
                <span class={"badge " + (x.kind === "reminder" ? "reminder" : "claude")}>{x.kind === "reminder" ? "Lembrete" : "Claude"}</span>
                <b>{x.name}</b>
                <Switch on={x.enabled} label="Ativo" onChange={() => toggle(x)} />
              </div>
              <p class="sched-p">{x.prompt}</p>
              <div class="sched-meta">
                <span>{x.cron ? describeCron(x.cron) : "Uma vez"}</span>
                <span>Próxima: {x.enabled ? when(x.nextRun) : "pausado"}</span>
                {x.lastRun ? <span>Última: {when(x.lastRun)} {x.lastStatus ? `(${x.lastStatus})` : ""}</span> : null}
                {x.kind === "claude" ? <span>{shortPath(x.cwd)}</span> : null}
              </div>
              <div class="row-end">
                {x.lastSid ? (
                  <button type="button" class="btn ghost small" onClick={() => openSession(x.lastSid)}>Ver execução</button>
                ) : null}
                <button type="button" class="btn ghost small" onClick={() => req("schedules.run", { sched: x.id }).then(() => toast("Executando")).catch(fail)}>Rodar agora</button>
                <button type="button" class="btn ghost small" onClick={() => setEditing(x)}>Editar</button>
                <button
                  type="button"
                  class="btn danger small"
                  onClick={() => {
                    if (confirm(`Apagar "${x.name}"?`)) req("schedules.delete", { sched: x.id }).catch(fail);
                  }}
                >
                  Apagar
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <Empty title="Nada agendado">Crie um lembrete ou peça para o Claude rodar algo sozinho mais tarde.</Empty>
      )}
    </div>
  );
}

function PowerCard() {
  const [p, setP] = useState(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    req("power.get").then(setP).catch(fail);
  }, []);
  if (!p) return null;
  const lidAwake = p.lidAc === 0;
  const setKeep = (v) =>
    req("config.set", { config: { keepAwake: v } })
      .then(() => setP({ ...p, keepAwake: v }))
      .catch(fail);
  const setLid = (awake) => {
    setBusy(true);
    req("power.lid", { awake })
      .then((r) => {
        setP(r);
        toast(awake ? "Fechar a tampa na tomada não suspende mais" : "Fechar a tampa volta a suspender");
      })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  return (
    <section class="card">
      <h2>Energia</h2>
      <p class="muted">
        Com o PC suspenso nada roda: nem o Claude, nem o Berklin. Para responder pelo celular com o notebook fechado, o PC precisa continuar ligado (a tela pode
        apagar).
      </p>
      <label class="check">
        <input type="checkbox" checked={p.keepAwake} onChange={(e) => setKeep(e.currentTarget.checked)} />
        Impedir a suspensão automática enquanto o Berklin estiver aberto
      </label>
      <div class="row">
        <span class={"chip " + (lidAwake ? "ok" : "bad")}>
          Tampa fechada na tomada: {p.lidAc === 0 ? "não faz nada" : p.lidAc === 1 ? "suspende" : p.lidAc === 2 ? "hiberna" : p.lidAc === 3 ? "desliga" : "?"}
        </span>
        <span class="chip">
          Na bateria: {p.lidDc === 0 ? "não faz nada" : p.lidDc === 1 ? "suspende" : p.lidDc === 2 ? "hiberna" : p.lidDc === 3 ? "desliga" : "?"}
        </span>
      </div>
      <div class="row-end">
        {lidAwake ? (
          <button type="button" class="btn ghost" disabled={busy} onClick={() => setLid(false)}>
            Voltar a suspender ao fechar a tampa
          </button>
        ) : (
          <button type="button" class="btn primary" disabled={busy} onClick={() => setLid(true)}>
            Não suspender ao fechar a tampa (na tomada)
          </button>
        )}
      </div>
    </section>
  );
}

function WindowsSettings() {
  const [cfg, setCfg] = useState(null);
  const [form, setForm] = useState(null);
  const [busy, setBusy] = useState(false);
  const load = () =>
    req("config.get")
      .then((c) => {
        setCfg(c);
        setForm({ pcName: c.pcName, relayUrl: c.relayUrl, relayToken: c.relayToken, autoAwayMin: c.autoAwayMin, autostart: c.autostart, notifyDone: c.notifyDone, defaultMode: c.defaultMode, claudePath: c.claudePath });
      })
      .catch(fail);
  useEffect(() => {
    load();
    const t = setInterval(() => req("config.get").then(setCfg).catch(() => {}), 5000);
    return () => clearInterval(t);
  }, []);
  if (!cfg || !form) return <div class="loading">Carregando…</div>;
  const set = (k) => (e) => setForm({ ...form, [k]: e.currentTarget.type === "checkbox" ? e.currentTarget.checked : e.currentTarget.type === "number" ? +e.currentTarget.value : e.currentTarget.value });
  const save = () => {
    setBusy(true);
    req("config.set", { config: form })
      .then((c) => {
        setCfg(c);
        toast("Salvo");
      })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  const act = (op, msg) => {
    setBusy(true);
    req(op, {}, 90000)
      .then((c) => {
        setCfg(c);
        toast(msg);
      })
      .catch(fail)
      .finally(() => setBusy(false));
  };
  return (
    <div class="page narrow">
      <h1>Ajustes</h1>
      <section class="card">
        <h2>Celular</h2>
        <div class="pair">
          <div class="qr" dangerouslySetInnerHTML={{ __html: cfg.relayUrl ? cfg.pairQr : "" }} />
          <div class="stack">
            <p class="muted">
              {cfg.relayUrl
                ? "Aponte a câmera do celular (ou o Google Lens) para o QR code com o app Berklin instalado. Ou copie o link e cole no app."
                : "Configure o servidor relay abaixo para gerar o QR code de pareamento."}
            </p>
            <div class="row">
              <span class={"chip " + (cfg.relayOnline ? "ok" : "bad")}>{cfg.relayOnline ? "Relay conectado" : cfg.relayUrl ? "Relay desconectado" : "Relay não configurado"}</span>
            </div>
            <div class="row">
              <button type="button" class="btn ghost small" disabled={!cfg.relayUrl} onClick={() => copy(cfg.pairLink)}>Copiar link</button>
              <button
                type="button"
                class="btn danger small"
                onClick={() => {
                  if (confirm("Gerar nova chave? Os celulares pareados vão precisar parear de novo.")) act("pair.rotate", "Nova chave gerada");
                }}
              >
                Nova chave
              </button>
            </div>
          </div>
        </div>
        <label class="field">
          <span>Servidor relay (Pterodactyl)</span>
          <input class="input mono" value={form.relayUrl} onInput={set("relayUrl")} placeholder="ws://IP:PORTA/ws" />
        </label>
        <label class="field">
          <span>Token do relay (RELAY_TOKEN)</span>
          <input class="input mono" value={form.relayToken} onInput={set("relayToken")} />
        </label>
      </section>
      <section class="card">
        <h2>Comportamento</h2>
        <label class="field">
          <span>Nome deste PC</span>
          <input class="input" value={form.pcName} onInput={set("pcName")} />
        </label>
        <label class="field">
          <span>Modo ausente automático após (minutos sem usar o PC, 0 desliga)</span>
          <input class="input" type="number" min="0" max="1440" value={form.autoAwayMin} onInput={set("autoAwayMin")} />
        </label>
        <label class="field">
          <span>Permissões padrão das sessões em segundo plano</span>
          <select class="input" value={form.defaultMode} onChange={set("defaultMode")}>
            {MODES.map(([v, l]) => (
              <option key={v} value={v}>
                {l}
              </option>
            ))}
          </select>
        </label>
        <label class="check">
          <input type="checkbox" checked={form.notifyDone} onChange={set("notifyDone")} /> Avisar quando uma sessão em segundo plano terminar
        </label>
        <label class="check">
          <input type="checkbox" checked={form.autostart} onChange={set("autostart")} /> Abrir com o Windows (na bandeja)
        </label>
        <label class="field">
          <span>Caminho do claude.exe (vazio = automático)</span>
          <input class="input mono" value={form.claudePath} onInput={set("claudePath")} />
        </label>
        <div class="row-end">
          <button type="button" class="btn primary" disabled={busy} onClick={save}>Salvar</button>
        </div>
      </section>
      <PowerCard />
      <section class="card">
        <h2>Integração com o Claude Code</h2>
        <div class="row">
          <span class={"chip " + (cfg.integration.hooks ? "ok" : "bad")}>Hooks {cfg.integration.hooks ? "instalados" : "não instalados"}</span>
          <span class={"chip " + (cfg.integration.mcp ? "ok" : "bad")}>MCP {cfg.integration.mcp ? "instalado" : "não instalado"}</span>
        </div>
        <p class="muted">
          Os hooks ficam em <code>{cfg.integration.settings}</code> e só repassam eventos para este app; com o app fechado eles não fazem nada. O MCP dá ao Claude as
          ferramentas <code>notify_user</code>, <code>ask_user</code> e <code>schedule_task</code>.
        </p>
        <div class="row-end">
          <button type="button" class="btn ghost" disabled={busy} onClick={() => act("integration.remove", "Integração removida")}>Remover</button>
          <button type="button" class="btn primary" disabled={busy} onClick={() => act("integration.install", "Integração instalada")}>
            {cfg.integration.hooks ? "Reinstalar" : "Instalar"}
          </button>
        </div>
      </section>
      <section class="card">
        <h2>Atualizações</h2>
        <p>
          Versão instalada <b>{cfg.version}</b>
          {cfg.update && cfg.update.latest ? <span class="muted"> · última no GitHub: {cfg.update.latest}</span> : null}
        </p>
        {cfg.update && cfg.update.error ? <p class="muted">{cfg.update.error}</p> : null}
        <label class="check">
          <input
            type="checkbox"
            checked={form.autoUpdate ?? cfg.autoUpdate}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setForm({ ...form, autoUpdate: v });
              req("config.set", { config: { autoUpdate: v } }).then(setCfg).catch(fail);
            }}
          />
          Atualizar sozinho (baixa do GitHub e reinicia quando nada estiver esperando você)
        </label>
        <div class="row-end">
          <button
            type="button"
            class="btn ghost"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              req("update.check", {}, 120000)
                .then((u) => {
                  setCfg({ ...cfg, update: u });
                  toast(u.available ? `Versão ${u.latest} disponível` : "Você já está na versão mais nova");
                })
                .catch(fail)
                .finally(() => setBusy(false));
            }}
          >
            Procurar agora
          </button>
          {cfg.update && cfg.update.ready ? (
            <button type="button" class="btn primary" disabled={busy} onClick={() => req("update.apply").then(() => toast("Reiniciando…")).catch(fail)}>
              Atualizar para {cfg.update.latest}
            </button>
          ) : null}
        </div>
      </section>
    </div>
  );
}

function Scanner({ onResult, onClose }) {
  const video = useRef(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    let stream = null;
    let raf = 0;
    let stopped = false;
    let last = 0;
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    const tick = (t) => {
      if (stopped) return;
      raf = requestAnimationFrame(tick);
      const v = video.current;
      if (!v || v.readyState < 2 || t - last < 110) return;
      last = t;
      const scale = Math.min(1, 720 / Math.max(v.videoWidth, v.videoHeight));
      canvas.width = (v.videoWidth * scale) | 0;
      canvas.height = (v.videoHeight * scale) | 0;
      if (!canvas.width || !canvas.height) return;
      ctx.drawImage(v, 0, 0, canvas.width, canvas.height);
      const img = ctx.getImageData(0, 0, canvas.width, canvas.height);
      const code = jsQR(img.data, img.width, img.height, { inversionAttempts: "dontInvert" });
      if (code && code.data) {
        stopped = true;
        onResult(code.data);
      }
    };
    if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      setErr("Este aparelho não liberou a câmera para o app.");
      return undefined;
    }
    navigator.mediaDevices
      .getUserMedia({ video: { facingMode: { ideal: "environment" }, width: { ideal: 1280 } }, audio: false })
      .then((st) => {
        if (stopped) {
          st.getTracks().forEach((tr) => tr.stop());
          return;
        }
        stream = st;
        video.current.srcObject = st;
        video.current.play().catch(() => {});
        raf = requestAnimationFrame(tick);
      })
      .catch((e) => setErr(e && e.name === "NotAllowedError" ? "Permita o uso da câmera para ler o QR code." : "Não consegui abrir a câmera."));
    return () => {
      stopped = true;
      cancelAnimationFrame(raf);
      if (stream) stream.getTracks().forEach((tr) => tr.stop());
    };
  }, []);
  return (
    <div class="scanner">
      <video ref={video} playsInline muted />
      <div class="scan-frame" />
      <div class="scan-ui">
        <p>{err || "Aponte para o QR code em Ajustes no Berklin do PC"}</p>
        <button type="button" class="btn" onClick={onClose}>Cancelar</button>
      </div>
    </div>
  );
}

function UpdateCardAndroid() {
  const s = useStore();
  const n = window.BerklinNative;
  const version = n && n.appVersion ? n.appVersion() : "";
  const u = s.appUpdate;
  return (
    <section class="card">
      <h2>Atualizações</h2>
      <p>
        Versão instalada <b>{version || "—"}</b>
        {u && u.latest ? <span class="muted"> · última no GitHub: {u.latest}</span> : null}
      </p>
      {u && u.error ? <p class="muted">{u.error}</p> : null}
      <p class="muted">O app procura atualizações sozinho a cada 6 horas e avisa quando houver uma pronta para instalar.</p>
      <div class="row-end">
        <button type="button" class="btn ghost" onClick={() => n && n.checkUpdate && (n.checkUpdate(), toast("Procurando atualização…"))}>Procurar agora</button>
        {u && u.available ? (
          <button type="button" class="btn primary" disabled={!u.downloaded} onClick={() => n.installUpdate()}>
            {u.downloaded ? `Instalar ${u.latest}` : "Baixando…"}
          </button>
        ) : null}
      </div>
    </section>
  );
}

function AndroidSettings() {
  const s = useStore();
  const [link, setLink] = useState("");
  const [scanning, setScanning] = useState(false);
  const [cfg, setCfg] = useState(() => nativeConfig());
  const pairWith = (value) => {
    const ok = /^berklin:\/\/pair/i.test(value) && window.BerklinNative.pair(value.trim());
    if (ok) {
      toast("Pareado");
      setLink("");
      setCfg(nativeConfig());
    } else toast("Esse QR code não é de pareamento do Berklin", "error");
  };
  const pair = () => pairWith(link.trim());
  if (scanning) {
    return (
      <Scanner
        onClose={() => setScanning(false)}
        onResult={(value) => {
          setScanning(false);
          pairWith(value);
        }}
      />
    );
  }
  return (
    <div class="page narrow">
      <h1>Ajustes</h1>
      <section class="card">
        <h2>Pareamento</h2>
        <button type="button" class="btn primary wide" onClick={() => setScanning(true)}>
          Escanear QR code do PC
        </button>
        {cfg.paired ? (
          <>
            <p>
              Pareado com <b>{cfg.pcName || s.pc.name}</b>
            </p>
            <p class="muted mono">{cfg.relayUrl}</p>
            <div class="row">
              <span class={"chip " + (s.conn.relay === "online" ? "ok" : "bad")}>Relay {s.conn.relay === "online" ? "conectado" : s.conn.relay === "connecting" ? "conectando" : "offline"}</span>
              <span class={"chip " + (s.conn.agent ? "ok" : "bad")}>PC {s.conn.agent ? "online" : "offline"}</span>
            </div>
          </>
        ) : (
          <p class="muted">Abra o Berklin no PC, vá em Ajustes e escaneie o QR code. Também dá para colar o link de pareamento abaixo.</p>
        )}
        <label class="field">
          <span>Link de pareamento</span>
          <input class="input mono" value={link} onInput={(e) => setLink(e.currentTarget.value)} placeholder="berklin://pair?..." />
        </label>
        <div class="row-end">
          {cfg.paired ? (
            <button
              type="button"
              class="btn danger"
              onClick={() => {
                if (confirm("Desparear deste PC?")) {
                  window.BerklinNative.unpair();
                  setCfg(nativeConfig());
                }
              }}
            >
              Desparear
            </button>
          ) : null}
          <button type="button" class="btn primary" disabled={!link.trim()} onClick={pair}>Parear</button>
        </div>
      </section>
      <section class="card">
        <h2>Notificações em segundo plano</h2>
        <p class="muted">Para o Android não cortar a conexão com o PC, libere o Berklin da otimização de bateria.</p>
        <div class="row-end">
          <button type="button" class="btn ghost" onClick={() => window.BerklinNative.batterySettings()}>Abrir ajuste de bateria</button>
        </div>
      </section>
      <UpdateCardAndroid />
      <AwayCard />
    </div>
  );
}

function Settings() {
  if (S.platform === "android") return <AndroidSettings />;
  if (S.platform === "windows") return <WindowsSettings />;
  return <Empty title="Abra pelo app Berklin" />;
}

const ICONS = {
  home: "M3.5 10.2 12 3.5l8.5 6.7V19a1.5 1.5 0 0 1-1.5 1.5h-4.2v-5.8H9.2v5.8H5A1.5 1.5 0 0 1 3.5 19z",
  chats: "M4.5 5.5h15a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H10l-4.5 3.5v-3.5h-1a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1zM8 10h8M8 13h5",
  cal: "M7.5 3v3M16.5 3v3M3.8 9.5h16.4M5.3 5h13.4a1.5 1.5 0 0 1 1.5 1.5v12.2a1.5 1.5 0 0 1-1.5 1.5H5.3a1.5 1.5 0 0 1-1.5-1.5V6.5A1.5 1.5 0 0 1 5.3 5zM8 13.5h2M14 13.5h2M8 17h2",
  tune: "M4 7h9M17 7h3M4 17h3M11 17h9M15 4.5v5M9 14.5v5",
  plus: "M12 5v14M5 12h14",
  back: "M14.5 5.5 8 12l6.5 6.5",
  send: "M5 12h11M12 6l6 6-6 6",
  stop: "M8 8h8v8H8z",
};

export function Icon({ n, s = 20 }) {
  return (
    <svg class="icon" width={s} height={s} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d={ICONS[n]} />
    </svg>
  );
}

function Logo({ sm }) {
  return (
    <span class={"logo" + (sm ? " sm" : "")}>
      <svg viewBox="0 0 24 24" width={sm ? 16 : 19} height={sm ? 16 : 19} fill="none" stroke="#fff" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 6.5h14a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1h-8.5L6 20v-3.5H5a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1z" />
        <path d="M9 11.5h.01M12 11.5h.01M15 11.5h.01" stroke-width="2.6" />
      </svg>
    </span>
  );
}

const NAV = [
  ["home", "Início", "home"],
  ["sessions", "Sessões", "chats"],
  ["schedules", "Agenda", "cal"],
  ["settings", "Ajustes", "tune"],
];

function ConnBanner() {
  const s = useStore();
  if (s.platform === "android" && !(s.native && s.native.paired) && !nativeConfig().paired) return null;
  if (s.conn.relay === "online" && s.conn.agent) return null;
  const text =
    s.platform === "windows"
      ? "Reconectando ao agente local…"
      : s.conn.relay === "online"
        ? "Relay conectado, mas o PC está offline."
        : s.conn.relay === "connecting"
          ? "Conectando ao relay…"
          : "Sem conexão com o relay.";
  return <div class="banner">{text}</div>;
}

function Sidebar() {
  const s = useStore();
  const pendingCount = s.pending.size;
  const recents = sessionsSorted().slice(0, 40);
  return (
    <aside class="side">
      <div class="brand">
        <Logo />
        <div>
          <b>Berklin</b>
          <span class="muted">{s.pc.name || "…"}</span>
        </div>
        <span class={"conn " + (s.conn.relay === "online" && s.conn.agent ? "ok" : "bad")} />
      </div>
      <nav class="side-nav">
        {NAV.map(([k, l, i]) => (
          <button type="button" key={k} class={s.view.name === k ? "on" : ""} onClick={() => navigate({ name: k })}>
            <Icon n={i} s={19} />
            {l}
            {k === "home" && pendingCount ? <span class="count">{pendingCount}</span> : null}
          </button>
        ))}
        <button type="button" class={"new-btn" + (s.view.name === "new" ? " on" : "")} onClick={() => navigate({ name: "new" })}>
          <Icon n="plus" s={18} />
          Novo chat
        </button>
      </nav>
      <div class="side-list">
        {recents.map((x) => (
          <button type="button" key={x.sid} class={"side-s" + (s.view.sid === x.sid ? " on" : "")} onClick={() => openSession(x.sid)}>
            <Dot status={x.status} live={x.live} />
            <span>{x.title}</span>
          </button>
        ))}
      </div>
    </aside>
  );
}

function TopBar() {
  const s = useStore();
  const v = s.view;
  const titles = { home: s.pc.name || "Berklin", sessions: "Sessões", schedules: "Agenda", settings: "Ajustes", new: "Novo chat" };
  const sub = v.name === "session" ? s.sessions.get(v.sid)?.title || "Sessão" : titles[v.name];
  const showBack = v.name === "session" || v.name === "new";
  return (
    <header class="top">
      {showBack ? (
        <button type="button" class="icon-btn" onClick={goBack} aria-label="Voltar">
          <Icon n="back" s={22} />
        </button>
      ) : (
        <Logo sm />
      )}
      <b class="top-title">{sub}</b>
      {s.away.on ? <span class="chip warnchip">ausente</span> : null}
      <span class={"conn " + (s.conn.relay === "online" && s.conn.agent ? "ok" : "bad")} />
    </header>
  );
}

function BottomNav() {
  const s = useStore();
  return (
    <nav class="bottom">
      {NAV.map(([k, l, i]) => (
        <button type="button" key={k} class={s.view.name === k ? "on" : ""} onClick={() => navigate({ name: k }, true)}>
          <span class="ni">
            <Icon n={i} s={22} />
          </span>
          <span>{l}</span>
          {k === "home" && s.pending.size ? <span class="count">{s.pending.size}</span> : null}
        </button>
      ))}
    </nav>
  );
}

function useWide() {
  const mq = useMemo(() => window.matchMedia("(min-width: 900px)"), []);
  const [wide, setWide] = useState(mq.matches);
  useEffect(() => {
    const f = () => setWide(mq.matches);
    mq.addEventListener("change", f);
    return () => mq.removeEventListener("change", f);
  }, [mq]);
  return wide;
}

function Toast() {
  const s = useStore();
  if (!s.toast) return null;
  return <div class={"toast " + s.toast.kind}>{s.toast.text}</div>;
}

function Main() {
  const s = useStore();
  const v = s.view;
  if (s.platform === "android" && !nativeConfig().paired && v.name !== "settings") return <AndroidSettings />;
  switch (v.name) {
    case "session":
      return <SessionView key={v.sid} sid={v.sid} />;
    case "sessions":
      return <Sessions />;
    case "schedules":
      return <Schedules />;
    case "settings":
      return <Settings />;
    case "new":
      return <NewChat />;
    default:
      return <Home />;
  }
}

function App() {
  const s = useStore();
  const wide = useWide();
  const inSession = s.view.name === "session";
  return (
    <div class={"app " + (wide ? "wide" : "narrow-l") + (inSession ? " in-session" : "")}>
      {wide ? <Sidebar /> : <TopBar />}
      <main class="main">
        <ConnBanner />
        <Main />
      </main>
      {!wide && !inSession ? <BottomNav /> : null}
      <Toast />
    </div>
  );
}

boot();
render(<App />, document.getElementById("app"));
