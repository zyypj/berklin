# Berklin protocol

Berklin lets a phone (Android) and the PC itself (Windows app) watch and drive the Claude Code sessions running on one Windows PC.

```
 Claude Code ──hooks──▶ berklin-hook.exe ──HTTP 127.0.0.1──▶ Agent (inside Berklin.exe on the PC)
 ~/.claude/projects/*.jsonl ──file watch──────────────────▶ Agent
 Windows UI ◀──ws://127.0.0.1 (plain JSON)──▶ Agent
 Android app ◀──ws(s)://relay (AES-GCM blobs)──▶ Relay (Pterodactyl, Node) ◀──▶ Agent
```

The relay never sees plaintext. Every app message is JSON encrypted end to end with a key that only the PC and the paired phones know.

## 1. Pairing

The PC generates a random 32-byte key `K` once and shows a QR code / copyable link:

```
berklin://pair?u=<urlencoded relay ws url>&t=<urlencoded relay token>&k=<base64url K, no padding>&n=<urlencoded PC name>
```

Example: `berklin://pair?u=ws%3A%2F%2F1.2.3.4%3A25565%2Fws&t=abc&k=q0d...&n=TUF`

The phone stores `u`, `t`, `k`, `n` and a random device id `dev` (16 hex chars) of its own.

Room id: `room = lowercase hex( SHA-256( UTF8("berklin-room:") || K ) )`, first 32 hex chars.

## 2. Relay transport

Connect: `<u>?room=<room>&role=client&dev=<dev>&t=<t>` (the agent uses `role=agent`, `dev=pc`).
The relay closes with code 4001 when the token is wrong, 4002 on bad params.

Frames:

| direction | WS frame | content |
|---|---|---|
| client → relay | binary | `payload` |
| relay → agent | binary | `[u8 n][n bytes ascii dev][payload]` |
| agent → relay | binary | `[u8 n][n bytes ascii dev][payload]`, `n = 0` means every client in the room |
| relay → client | binary | `payload` |
| relay → client | text | `{"relay":"agent","online":true\|false}` (sent on connect and whenever the agent connects/leaves) |
| relay → agent | text | `{"relay":"client","dev":"<dev>","online":true\|false}` |

`payload = nonce(12 random bytes) || AES-256-GCM(key=K, nonce, plaintext)` where the GCM output is ciphertext followed by the 16-byte tag. No AAD. `plaintext` is UTF-8 JSON (one app message).

The relay sends WS pings every 25 s. Clients should also ping every 25-30 s and reconnect with backoff (1 s → 30 s max) on any close.

On the Windows UI the same app messages travel unencrypted as WS **text** frames over `ws://127.0.0.1:<port>/ws?t=<localToken>`.

## 3. App messages

Requests (client → agent): `{"id": <int>, "op": "<op>", ...args}`
Responses (agent → client): `{"re": <id>, "ok": true, "data": <any>}` or `{"re": <id>, "ok": false, "err": "<message>"}`
Events (agent → client, unsolicited): `{"ev": "<event>", ...fields}`

Every client must send `hello` first after each (re)connect.

### Ops

| op | args | data |
|---|---|---|
| `hello` | `dev`, `name`, `platform` (`android`/`windows`) | `Snapshot` |
| `sessions.list` | `limit?`, `before?` (ms, `updated` cursor) | `Session[]` |
| `session.open` | `sid`, `limit?` (default 150) | `{sid, items: Item[], more: bool}` and subscribes this client to `items` events for `sid` |
| `session.older` | `sid`, `before` (item `i`), `limit?` | `{items: Item[], more: bool}` |
| `session.close` | `sid` | `null` (unsubscribe) |
| `send` | `sid?`, `cwd?`, `text`, `mode?` (permission mode for new/resumed headless runs) | `{sid, how}` with `how` ∈ `injected` (delivered into the live session), `queued` (session busy, delivered when Claude stops), `resumed` (headless resume), `started` (new headless session) |
| `interrupt` | `sid` | `null` (kills a headless run started by Berklin) |
| `pending.resolve` | `pid` and one of: `allow` (bool) + `message?`; `answers` (`{questionText: answerText}`); `text` (string, for `stop`); `release: true` (give the request back to the PC screen) | `null` |
| `away.set` | `on` (bool) | `Away` |
| `schedules.save` | `schedule` (`Schedule` without computed fields; no `id` = create) | `Schedule` |
| `schedules.delete` | `sched` (schedule id; `id` is always the request id) | `null` |
| `schedules.run` | `sched` | `null` |
| `projects.list` | — | `{cwd, name, last}[]` |
| `notes.ack` | `ids` (string[]) | `null` |

### Events

| ev | fields |
|---|---|
| `session` | `s: Session` (upsert) |
| `items` | `sid`, `items: Item[]` (append; only to subscribed clients) |
| `pending` | `p: Pending` (new or updated) |
| `pending.done` | `pid` |
| `note` | `n: Note` |
| `schedule` | `s: Schedule` (upsert) |
| `schedule.gone` | `id` |
| `away` | `a: Away` |

### Types

```ts
Snapshot = { pc: {name: string}, away: Away, sessions: Session[], pending: Pending[], schedules: Schedule[], notes: Note[] }
Away     = { on: boolean, auto: boolean }
Session  = { sid, title, cwd, project, updated /*ms*/, status: "running"|"waiting"|"idle", live: boolean, headless: boolean, preview: string, model?: string, tokens?: number }
Item     = { i /*int, index inside the session*/, k: "user"|"text"|"think"|"tool"|"result"|"info", ts /*ms*/, text?: string, name?: string /*tool name*/, input?: any /*tool input*/, tid?: string /*tool_use_id, links result→tool*/, err?: boolean }
Pending  = { pid, sid, kind: "permission"|"question"|"stop", ts, title /*session title*/, cwd, tool?: string, input?: any,
             questions?: {question: string, header: string, multiSelect: boolean, options: {label: string, description: string}[]}[],
             message?: string /*for stop: Claude's last message*/ }
Note     = { nid, ts, kind: "needs"|"done"|"schedule"|"info"|"error", title, body, sid?: string, pid?: string }
Schedule = { id, name, kind: "claude"|"reminder", prompt, cwd, at?: number /*ms, one-shot*/, cron?: string /*"min hour dom mon dow", local time*/,
             enabled: boolean, mode: string /*permission mode*/, nextRun?: number, lastRun?: number, lastStatus?: string, lastSid?: string }
```

## 4. Notifications (client behaviour)

* `pending` with `kind = permission` → notification "<title>: <tool> precisa de permissão" with actions **Permitir** / **Negar** → `pending.resolve {pid, allow}`.
* `pending` with `kind = question`: if there is exactly one question, `multiSelect = false` and ≤ 3 options → one action per option label → `pending.resolve {pid, answers: {[question]: label}}`. Always add a **Responder** inline-reply action (free text → same `answers` map, used for the first question).
* `pending` with `kind = stop` → "<title> terminou" + **Responder** inline-reply → `pending.resolve {pid, text}`.
* `pending.done` → cancel that notification.
* `note` → plain notification (dedupe by `nid`), then `notes.ack`. Tapping any notification opens the app on `sid`.
* Unacked notes are re-sent in `Snapshot.notes` after reconnect, so clients must dedupe by `nid`.

## 5. Shared UI and the Android bridge

The UI is one web bundle in `ui/dist` (`index.html`, `app.js`, `app.css`, no ES modules, works from `file://`).

* Windows: served by the Tauri app; the UI connects itself to `ws://127.0.0.1:<port>/ws?t=<token>` (port/token injected as `window.__BERKLIN_LOCAL = {port, token}`).
* Android: loaded from `file:///android_asset/ui/index.html` in a WebView. The native foreground service owns the single relay socket and the crypto; the UI talks to it through a JS bridge.

Bridge, native object `window.BerklinNative` (`addJavascriptInterface`):

| method | returns | meaning |
|---|---|---|
| `send(json: String)` | void | send one app message (request) to the agent |
| `config()` | String JSON `{paired, pcName, relayUrl}` | pairing state |
| `pair(link: String)` | boolean | store a `berklin://pair?...` link and (re)connect |
| `unpair()` | void | forget pairing, disconnect |
| `batterySettings()` | void | open the "ignore battery optimizations" screen for the app |

Native → JS (via `evaluateJavascript`), each argument a JSON string:

* `window.__berklin.onMessage(json)` — every decrypted response/event from the agent
* `window.__berklin.onStatus(json)` — `{relay: "connecting"|"online"|"offline", agent: boolean}`
* `window.__berklin.onOpen(sid)` — the user tapped a notification for session `sid`

Request ids: the UI uses ids `< 1_000_000_000`; the native service uses ids `>= 1_000_000_000` for its own requests (its own `hello`, `pending.resolve` from notification actions, `notes.ack`). Responses go to whoever owns the id; events go to both.

The native service sends its own `hello` on every connect and keeps notifications in sync from the snapshot (`pending`, `notes`) and the events, even when the UI is closed. When the UI starts it sends its own `hello` through the bridge.
