import { Component } from "preact";
import { useMemo } from "preact/hooks";
import { Marked } from "marked";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import powershell from "highlight.js/lib/languages/powershell";
import javascript from "highlight.js/lib/languages/javascript";
import typescript from "highlight.js/lib/languages/typescript";
import json from "highlight.js/lib/languages/json";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import java from "highlight.js/lib/languages/java";
import kotlin from "highlight.js/lib/languages/kotlin";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import xml from "highlight.js/lib/languages/xml";
import sql from "highlight.js/lib/languages/sql";
import yaml from "highlight.js/lib/languages/yaml";
import diff from "highlight.js/lib/languages/diff";
import go from "highlight.js/lib/languages/go";
import markdown from "highlight.js/lib/languages/markdown";
import ini from "highlight.js/lib/languages/ini";

const LANGS = { bash, powershell, javascript, typescript, json, python, rust, java, kotlin, csharp, css, xml, sql, yaml, diff, go, markdown, ini };
for (const [k, v] of Object.entries(LANGS)) hljs.registerLanguage(k, v);
hljs.registerAliases(["sh", "shell", "zsh", "console"], { languageName: "bash" });
hljs.registerAliases(["ps1", "pwsh", "ps"], { languageName: "powershell" });
hljs.registerAliases(["js", "jsx", "mjs", "cjs"], { languageName: "javascript" });
hljs.registerAliases(["ts", "tsx"], { languageName: "typescript" });
hljs.registerAliases(["py"], { languageName: "python" });
hljs.registerAliases(["rs"], { languageName: "rust" });
hljs.registerAliases(["kt", "kts"], { languageName: "kotlin" });
hljs.registerAliases(["cs", "c#"], { languageName: "csharp" });
hljs.registerAliases(["html", "htm", "svg", "xaml", "csproj"], { languageName: "xml" });
hljs.registerAliases(["yml"], { languageName: "yaml" });
hljs.registerAliases(["md"], { languageName: "markdown" });
hljs.registerAliases(["toml", "properties", "env"], { languageName: "ini" });

export function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

export function highlight(text, lang) {
  const l = (lang || "").toLowerCase();
  if (l && hljs.getLanguage(l) && text.length < 120000) {
    try {
      return hljs.highlight(text, { language: l, ignoreIllegals: true }).value;
    } catch {}
  }
  return esc(text);
}

function codeBlock(text, lang, label) {
  return `<pre class="code"><div class="code-h"><span>${esc(label || lang || "texto")}</span><button type="button" data-copy>copiar</button></div><code class="hljs">${highlight(text, lang)}</code></pre>`;
}

const md = new Marked({
  gfm: true,
  breaks: false,
  renderer: {
    html({ text }) {
      return esc(text);
    },
    link({ href, tokens }) {
      const inner = this.parser.parseInline(tokens);
      const ok = /^(https?:|mailto:)/i.test(href || "");
      return ok ? `<a href="${esc(href)}" data-ext="1">${inner}</a>` : inner;
    },
    image({ href, text }) {
      return esc(`[imagem: ${text || href}]`);
    },
    code({ text, lang }) {
      const l = (lang || "").split(/\s/)[0];
      return codeBlock(text, l);
    },
  },
});

export function mdHtml(text) {
  try {
    return md.parse(text || "");
  } catch {
    return `<p>${esc(text)}</p>`;
  }
}

export function memo(fn) {
  return class Memo extends Component {
    shouldComponentUpdate(next) {
      for (const k in next) if (next[k] !== this.props[k]) return true;
      for (const k in this.props) if (!(k in next)) return true;
      return false;
    }
    render(p) {
      return fn(p);
    }
  };
}

export const Markdown = memo(({ text, cls }) => <div class={"md " + (cls || "")} dangerouslySetInnerHTML={{ __html: mdHtml(text) }} />);

const EXT = {
  js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript", ts: "typescript", tsx: "typescript",
  rs: "rust", py: "python", java: "java", kt: "kotlin", kts: "kotlin", cs: "csharp", json: "json", css: "css",
  scss: "css", html: "xml", htm: "xml", xml: "xml", xaml: "xml", csproj: "xml", sql: "sql", yml: "yaml", yaml: "yaml",
  md: "markdown", sh: "bash", ps1: "powershell", psm1: "powershell", go: "go", toml: "ini", ini: "ini", properties: "ini",
};

export function langOf(path) {
  const m = /\.([A-Za-z0-9]+)$/.exec(path || "");
  return m ? EXT[m[1].toLowerCase()] || "" : "";
}

export function shortPath(p) {
  if (!p) return "";
  const parts = String(p).split(/[\\/]/).filter(Boolean);
  return parts.length > 3 ? "…/" + parts.slice(-3).join("/") : parts.join("/");
}

function lineDiff(a, b) {
  const A = (a || "").split("\n");
  const B = (b || "").split("\n");
  let s = 0;
  while (s < A.length && s < B.length && A[s] === B[s]) s++;
  let ea = A.length - 1;
  let eb = B.length - 1;
  while (ea >= s && eb >= s && A[ea] === B[eb]) {
    ea--;
    eb--;
  }
  const head = A.slice(Math.max(0, s - 2), s).map((t) => [" ", t]);
  const tail = A.slice(ea + 1, Math.min(A.length, ea + 3)).map((t) => [" ", t]);
  const a2 = A.slice(s, ea + 1);
  const b2 = B.slice(s, eb + 1);
  const n = a2.length;
  const m = b2.length;
  let mid;
  if (n * m > 250000) {
    mid = [...a2.map((t) => ["-", t]), ...b2.map((t) => ["+", t])];
  } else {
    const dp = new Uint16Array((n + 1) * (m + 1));
    for (let i = n - 1; i >= 0; i--)
      for (let j = m - 1; j >= 0; j--)
        dp[i * (m + 1) + j] = a2[i] === b2[j] ? dp[(i + 1) * (m + 1) + j + 1] + 1 : Math.max(dp[(i + 1) * (m + 1) + j], dp[i * (m + 1) + j + 1]);
    mid = [];
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
      if (a2[i] === b2[j]) {
        mid.push([" ", a2[i]]);
        i++;
        j++;
      } else if (dp[(i + 1) * (m + 1) + j] >= dp[i * (m + 1) + j + 1]) mid.push(["-", a2[i++]]);
      else mid.push(["+", b2[j++]]);
    }
    while (i < n) mid.push(["-", a2[i++]]);
    while (j < m) mid.push(["+", b2[j++]]);
  }
  return [...head, ...mid, ...tail];
}

function Diff({ a, b, lang }) {
  const lines = useMemo(() => lineDiff(a, b), [a, b]);
  return (
    <pre class="diff">
      {lines.map(([t, l], i) => (
        <div key={i} class={t === "+" ? "add" : t === "-" ? "del" : "ctx"}>
          <span class="sign">{t}</span>
          <span dangerouslySetInnerHTML={{ __html: highlight(l, lang) || " " }} />
        </div>
      ))}
    </pre>
  );
}

function Code({ text, lang, label }) {
  return <div dangerouslySetInnerHTML={{ __html: codeBlock(text || "", lang, label) }} />;
}

export function toolSummary(name, input) {
  const i = input || {};
  switch (name) {
    case "Bash":
    case "PowerShell":
      return i.description || String(i.command || "").split("\n")[0];
    case "Read":
    case "Edit":
    case "Write":
    case "MultiEdit":
    case "NotebookEdit":
      return shortPath(i.file_path || i.notebook_path);
    case "Grep":
      return `"${i.pattern || ""}"${i.path ? " em " + shortPath(i.path) : ""}`;
    case "Glob":
      return i.pattern || "";
    case "WebFetch":
      return i.url || "";
    case "WebSearch":
      return i.query || "";
    case "Task":
    case "Agent":
      return i.description || "";
    case "TodoWrite":
      return `${(i.todos || []).length} tarefas`;
    case "AskUserQuestion":
      return (i.questions || [])[0]?.question || "";
    default:
      return "";
  }
}

const TOOL_ICON = {
  Bash: "›_", PowerShell: "›_", Read: "◱", Edit: "✎", MultiEdit: "✎", Write: "✚", Grep: "⌕", Glob: "⌕",
  WebFetch: "◎", WebSearch: "◎", Task: "◇", Agent: "◇", TodoWrite: "☑", AskUserQuestion: "?",
};

function Todos({ todos }) {
  return (
    <ul class="todos">
      {(todos || []).map((t, i) => (
        <li key={i} class={t.status}>
          <span class="chk">{t.status === "completed" ? "✓" : t.status === "in_progress" ? "●" : "○"}</span>
          {t.content || t.activeForm}
        </li>
      ))}
    </ul>
  );
}

function ToolBody({ name, input, res }) {
  const i = input || {};
  const out = res && res.text;
  switch (name) {
    case "Edit":
      return <Diff a={i.old_string} b={i.new_string} lang={langOf(i.file_path)} />;
    case "MultiEdit":
      return (
        <div>
          {(i.edits || []).map((e, k) => (
            <Diff key={k} a={e.old_string} b={e.new_string} lang={langOf(i.file_path)} />
          ))}
        </div>
      );
    case "Write":
      return <Code text={i.content} lang={langOf(i.file_path)} label={shortPath(i.file_path)} />;
    case "Bash":
    case "PowerShell":
      return (
        <div>
          <Code text={i.command} lang={name === "Bash" ? "bash" : "powershell"} label={name} />
          {out ? <pre class={"out" + (res.err ? " err" : "")}>{out}</pre> : null}
        </div>
      );
    case "TodoWrite":
      return <Todos todos={i.todos} />;
    case "Read":
      return out ? <pre class="out">{out}</pre> : null;
    default:
      return (
        <div>
          {input && Object.keys(i).length ? <Code text={JSON.stringify(input, null, 2)} lang="json" label="entrada" /> : null}
          {out ? <pre class={"out" + (res.err ? " err" : "")}>{out}</pre> : null}
        </div>
      );
  }
}

const OPEN_BY_DEFAULT = new Set(["Edit", "MultiEdit", "Write", "TodoWrite"]);

export const ItemView = memo(({ it, res }) => {
  switch (it.k) {
    case "user":
      return (
        <div class="it user">
          <div class="bubble">{it.text}</div>
        </div>
      );
    case "text":
      return (
        <div class="it text">
          <Markdown text={it.text} />
        </div>
      );
    case "think":
      return (
        <details class="it think">
          <summary>Pensamento</summary>
          <Markdown text={it.text} cls="dim" />
        </details>
      );
    case "info":
      return <div class="it info">{it.text}</div>;
    case "tool": {
      const err = res && res.err;
      return (
        <details class={"it tool" + (err ? " err" : "") + (res ? "" : " live")} open={OPEN_BY_DEFAULT.has(it.name) || err}>
          <summary>
            <span class="ti">{TOOL_ICON[it.name] || "◆"}</span>
            <b>{it.name}</b>
            <span class="ts">{toolSummary(it.name, it.input)}</span>
            {res ? null : <span class="spin" />}
          </summary>
          <div class="tb">
            <ToolBody name={it.name} input={it.input} res={res} />
          </div>
        </details>
      );
    }
    default:
      return null;
  }
});

export function Transcript({ list }) {
  const results = useMemo(() => {
    const m = new Map();
    for (const it of list) if (it.k === "result" && it.tid) m.set(it.tid, it);
    return m;
  }, [list]);
  return list.map((it) => (it.k === "result" ? null : <ItemView key={it.i} it={it} res={it.k === "tool" ? results.get(it.tid) : undefined} />));
}
