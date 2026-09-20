# umux Storestation — CLI Surface & Socket Protocol (design doc)

**Status:** design output of `/agent-cli` (design mode), 2026-09-19. Feeds [`umux-v1.7.0-plan.md`](./umux-v1.7.0-plan.md) (phases 1, 2, 4, 5) and becomes the protocol reference for the v1.8.0 live CLI (stories #53–#55, #87).
**Scope:** the v1.7.0 CLI additions (`umux status`, `umux sessions list`, `umux attach`, `umux agent-context`, the daemon binary `umux-storestation`) and protocol **v1** of the umux Storestation local socket.

## Summary

A small, strictly additive extension of the existing `umux` CLI plus a versioned local-socket protocol owned by umux Storestation. The new surface passes all Tier-1 agent-native principles from day one (canonical `--json`, enumerated errors, bounded responses, non-interactive defaults, safe retries). Known Friction in the *inherited* v1.6.x surface (no `--json`, non-standard verbs, no introspection) is deliberately deferred to v1.8.0 — no breaking changes in v1.7.0.

## Constraints

- Rust + clap derive; existing crate `cli` (package `umux`); new workspace member for the daemon binary `umux-storestation`; shared engine crate (`session_core`) per the plan.
- Transport: one local per-user socket; its name/path derives from the config directory, so `UMUX_CONFIG_DIR` isolates store + socket + sessions together for test instances (e.g. `~/.umux-test`).
- Consumers: humans, shell scripts, AI agents (Claude Code driving umux headlessly) — non-interactive and structured output are mandatory on data commands.
- v1.8.0 grows the op catalog to full app parity; the v1.7.0 daemon must reject unknown ops cleanly so that growth is purely additive.
- No breaking changes to existing v1.6.x commands.

## Command classes

| Command | Class |
|---|---|
| `umux status` | read |
| `umux sessions list` | read |
| `umux attach` | bootstrap (launches GUI; idempotent via single-instance) |
| `umux agent-context` | read |
| `umux-storestation run` | bootstrap (foreground service) |
| `umux-storestation stop` | mutate (idempotent) |

## Surface sketch

```
umux (existing CLI binary — additions only)
  status            [--json]                    Storestation health; offline is a state (exit 0), not an error
  sessions list     [--json] [--limit N]        live sessions owned by Storestation
  attach            [--json] [--dry-run]        launch/focus the desktop app bound to Storestation
  agent-context                                 machine-readable self-description (schema 1)
  --config-dir <path>                           global; precedence: flag > UMUX_CONFIG_DIR > platform default

umux-storestation (new daemon binary)
  run                [--config-dir]             serve foreground until stop; single instance enforced
  stop               [--json] [--config-dir]    graceful shutdown: kills owned shells, cleans socket
  --version
```

Existing v1.6.x commands unchanged (`list export notify import new rm rename split config`, `--desk/--term` untouched — live commands never read the store directly).

## Exit codes (both binaries; documented in `--help` and `agent-context`)

| Code | Meaning |
|---|---|
| 0 | success — including "Storestation offline" answers from `status` / `sessions list` (offline is a state) |
| 2 | usage error (clap default) |
| 3 | Storestation required but not reachable (`attach`; future live commands) |
| 4 | conflict — Storestation already running (`umux-storestation run`) |
| 5 | internal / unexpected error |

## Output contracts

`umux status --json` (Storestation running):

```json
{
  "cliVersion": "1.7.0",
  "protocol": 1,
  "storestation": {
    "running": true, "version": "1.7.0", "pid": 4212,
    "uptimeSeconds": 3600, "sessions": 2, "attachedClients": 1,
    "dataDir": "C:\\Users\\adam\\AppData\\Roaming\\umux"
  }
}
```

`umux status --json` (Storestation off): `{ "cliVersion": "1.7.0", "protocol": 1, "storestation": { "running": false, "staleSocket": false } }` — **exit 0**.

`umux sessions list --json`:

```json
{
  "storestation": { "running": true },
  "sessions": [
    { "id": "<uuid>", "title": "pwsh — ~/proj", "workspaceId": "…", "tabId": "…", "panelId": "…",
      "cwd": "…", "shell": "…", "cols": 120, "rows": 40, "attachedClients": 1, "createdAt": "…" }
  ],
  "truncated": false
}
```

Storestation off → `"sessions": []` + `"storestation": {"running": false}` — exit 0 (the `storestation` block makes "no sessions" vs "daemon off" unambiguous).

`umux attach --json`: `{ "launched": true, "appPid": 8123 }` or `{ "launched": false, "reason": "alreadyRunning", "focused": true }`. Storestation off → exit 3 with the error object below.

Error object (CLI `--json` **and** protocol, same shape):

```json
{ "code": "storestationNotRunning", "message": "umux Storestation is not running.",
  "next": ["Enable it in Settings → Storestation", "or run: umux-storestation run"], "retryable": false }
```

## Error catalog (machine codes; v1.8.0 extends additively — clients treat unknown codes as generic)

`storestationNotRunning` · `storestationAlreadyRunning` · `staleSocket` · `protoTooNew` · `protoTooOld` · `unknownOp` · `badParams` · `sessionNotFound` · `limitInvalid` · `ioError`

(`badParams` joined at phase 2 with the session ops: a syntactically valid request whose params fail validation answers `badParams`, not `unknownOp`.)

## `agent-context` skeleton

```json
{
  "schema": 1,
  "cli": "umux", "cliVersion": "1.7.0",
  "protocol": 1, "daemon": "umux-storestation",
  "env": { "configDir": "UMUX_CONFIG_DIR", "precedence": "flag > env > default" },
  "exitCodes": { "0": "ok / Storestation offline state", "2": "usage", "3": "storestation unreachable", "4": "already running", "5": "internal" },
  "errors": ["storestationNotRunning", "…"],
  "commands": [
    { "name": "status", "class": "read", "json": true,
      "notes": ["exits 0 when Storestation is offline — offline is a state, not an error"] },
    { "name": "sessions list", "class": "read", "json": true, "limitDefault": 100 },
    { "name": "attach", "class": "bootstrap", "json": true, "dryRun": true },
    { "name": "agent-context", "class": "read", "json": "always" }
  ]
}
```

## Socket protocol v1

**Transport**
- Unix (macOS/Linux): UDS at `<config_dir>/storestation.sock`, perms `0600`; stale file unlinked on daemon start.
- Windows: named pipe `\\.\pipe\umux-storestation-<hash>` where `<hash>` = short stable hash of the canonical config dir path (so `UMUX_CONFIG_DIR` gives test isolation for free).
- One socket; multiple concurrent client connections (CLI one-shot, desktop app persistent).

**Framing** — every frame: `u32 LE length` + payload; payload starts with a 1-byte type tag:
- `0x01` control — JSON envelope (UTF-8)
- `0x02` data — binary: `sessionIdLen:u16 | sessionId:utf8 | bytes` (frame cap 64 KiB)

**Handshake** (first control frame, client→server): `{"v":1,"op":"hello","params":{"client":"cli|desktop|tui","clientVersion":"1.7.0"}}` → `{"id":…,"ok":true,"result":{"proto":1,"daemonVersion":"1.7.0","daemonPid":…}}`. Major mismatch → `{"ok":false,"error":{"code":"protoTooNew"|"protoTooOld",…}}`, then close.

**Control envelope** — request `{"id":<u64>,"op":"<resource.verb>","params":{…}}`; response `{"id":<u64>,"ok":true,"result":{…}}` | `{"id":<u64>,"ok":false,"error":{code,message,next,retryable}}`. Fields camelCase (consistent with the store JSON). Ops dotted `resource.verb`.

**Op catalog**

Implemented in v1.7.0:
- `storestation.status` — daemon health (used by `umux status`)
- `sessions.list` — `limit` (default 100, max 1000) → sessions + `truncated`; a session entry is `{ id, title, workspaceId, tabId, panelId, cwd, shell, cols, rows, attachedClients, createdAt }` (the three ids and `title` may be null; `createdAt` is unix epoch seconds)
- `sessions.create` (client-generated UUIDv4 id; optional shell, cwd, cols/rows — default 80x24 — and title/workspaceId/tabId/panelId) → the session's summary. IDEMPOTENT by id: an existing id returns that session's summary and spawns nothing (create-by-key groundwork for v1.8.0 retries). Missing shell/cwd fall back to the daemon's defaults (`$SHELL` → `/bin/sh`; `$HOME`)
- `session.write` — `{ id, data }` where `data` is BASE64-encoded bytes (binary-safe inside the JSON envelope)
- `session.resize` — `{ id, cols, rows }`
- `session.kill` — `{ id }` → kills the child, drops the record, notifies subscribers
- `session.subscribe` — `{ id, subscriber? }` → opens this connection's push stream of `0x02` data frames for the session plus lifecycle events; result `{ subscribed: true, attachedClients: N }`. The optional `subscriber` token labels the attachment
- `session.unsubscribe` — `{ id, subscriber }` → detaches exactly that attachment; the session itself keeps living (a closed panel detaches, it does not kill)
- `session.status` — `{ id }` → the live lookups the desktop driver needs per panel: `{ busy, childPid, foregroundPid, cwd, exitCode }` (phase 2 addition so Storestation ON keeps close-confirmation, agent-status presence, the cwd snapshot and the ports tooltip at full parity)
- `storestation.shutdown` — idempotent graceful stop (used by `umux-storestation stop`); kills every owned shell first

**Event envelope (phase 2)** — lifecycle events are control frames WITHOUT an `id` (they are not responses): `{"event":"session.exit","session":"<id>","exitCode":<code|null>}` and `{"event":"session.title","session":"<id>","title":"<text>"}`. Exactly one `session.exit` per session, even when a kill and the end-of-stream race. Titles are noticed by a READ-ONLY scan of the stream for OSC 0/2 sequences — the daemon never rewrites a byte (byte-identical rule; the umux OscParser still runs client-side).

Defined but unimplemented (daemon answers `unknownOp` + protocol level until then): future v1.8.0+ ops (e.g. workspace/tab management at app parity).

Unknown op / bad params → enumerated error; connection stays open.

**Idempotency & mutation boundaries** — `storestation.shutdown` safe twice; session ids client-generated (create-by-key is idempotent for v1.8.0 retries); `attach` has `--dry-run` and the app gets `tauri-plugin-single-instance` so re-attach focuses instead of duplicating.

**Bounds & timeouts** — lists: default limit 100 (max 1000, `truncated:true` beyond); data frames ≤ 64 KiB; client defaults: connect 3 s, request 10 s, attach app-launch wait 15 s.

## Findings (self-audit, sorted by severity)

### Friction — P2/P6: existing v1.6.x commands (`list`, `export`, `new`, `rm`, …)
**Observed:** text-only output; verbs `new`/`rm`/`rename` deviate from create/delete/update; `list` overloads "workspaces".
**Expected:** canonical `--json` on data commands; standard verb vocabulary.
**Fix:** v1.7.0 sets the pattern on all *new* commands; v1.8.0 retrofits `--json` + verb aliases to old ones without removal. *(Becomes Blocker at v1.8.0 if skipped.)*

### Friction — P7: no machine-readable introspection today
**Observed:** only human `--help`; surface reaches 12+ commands across two binaries in v1.7.0.
**Fix:** ship `umux agent-context` (schema 1) in plan phase 1; add a parity test asserting it matches `--help` so the layers cannot drift.

### Friction — P3: existing errors are human-only prose
**Fix:** new commands implement the error catalog (`code`/`message`/`next`/`retryable`) from day one; old commands migrate in v1.8.0.

### Optimization — P5: existing `list` is unbounded
**Fix (when touched, v1.8.0):** `--limit`, default 100.

### Optimization — P9: config dir selectable via env only
**Fix:** global `--config-dir` flag on both binaries; precedence flag > env > default; documented in `agent-context`.

### Optimization — P6: two binaries, one vocabulary
**Note:** keep flags, exit codes and error codes identical across `umux` and `umux-storestation` (`stop --json` mirrors the convention).

## Recommendations

1. Ship every new command with `--json`, the exit-code catalog and the error object from day one (plan phases 1–2) — cheapest now, expensive to retrofit.
2. `umux agent-context` lands in phase 1 with a parity test against `--help`.
3. Protocol v1 must answer `unknownOp` for unimplemented ops — this is what makes v1.8.0 purely additive.
4. Add `tauri-plugin-single-instance` in the attach phase so `umux attach` is idempotent (second run focuses the existing window; eliminates the store double-writer risk).
5. Defer the old-command retrofit (`--json`, verb aliases, limits) to v1.8.0 — no breaking changes in v1.7.0.

## Architectural backstop

By v1.8.0 the CLI approaches ~20 commands across two binaries with two output modes. Generate `agent-context` (and optionally JSON schemas) from a single command-table source rather than hand-maintaining the layers; revisit when the count passes ~20.
