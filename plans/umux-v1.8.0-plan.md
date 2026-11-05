# Plan: umux v1.8.0 — Live CLI + Local Socket

> Source PRD: [`umux-v1.8.0-prd.md`](./umux-v1.8.0-prd.md) (master PRD wins on conflict; protocol reference: [`umux-storestation-cli-protocol.md`](./umux-storestation-cli-protocol.md)).
> Carve 2026-11-02. PO decisions locked at carve: **no QuickType** (JSON Schema contract + wire/contract tests, hand-written TS types), **full 1:1 parity including system actions** (update, reset, open, window), **offline commands keep the `--desk`/`--term` requirement**.

## Architectural decisions

Durable decisions that apply across all phases:

- **One socket, one server — "whoever is alive" (PO 2026-10-27):** the desktop app serves the protocol when Storestation is OFF (or the daemon is absent); the daemon serves when it is ON. Same path/pipe (`storestation.sock` / hash pipe), same framing, same hello. The CLI probes and talks to whichever socket answers; `umux status --json` reports `server: "app" | "daemon"` (+ `serverVersion`) — additive fields (story 10).
- **Daemon as router when ON:** the daemon executes session and Core ops itself (today's behavior, unchanged) and **forwards app-domain ops to the registered desktop client** when the app is connected (protocol v1 additive extension: an executor-registration op + forwarded request/response control frames). No registered app → app-domain ops answer the new additive error `appNotRunning` (exit 3, `next` names `umux attach`).
- **App gateway = bridge, not a second brain:** the app's socket server (in the app process, reusing the storestation crate's protocol/transport/socketpath/client code) forwards requests over a Tauri event bridge to the frontend; the frontend executes the **pure state ops** it already uses for UI actions (workspaces/groups/tabs/panes/settings modules, split-tree ops) and persists via the existing save path. Terminal I/O ops hit the in-process `pty_*`/`ssh_*` commands directly. The live state keeps ONE owner: the frontend.
- **Single-writer guarantee (story 7):** while the app or daemon is alive, the CLI never writes the store files — every mutation rides the gateway; the offline fallback (nothing alive) writes saved definitions exactly like today and **probes both sockets first**.
- **One command family (story 6):** canonical `resource.verb` names; every v1.6.x short name stays as an **alias of the same command**; `create`/`delete`/`update` join as aliases beside `new`/`rm`/`rename` (story 8). Global commands (`status`, `attach`, `agent-context`, `notify`) keep their names.
- **One command table (story 9):** a single in-code table (canonical name, aliases, class, json support, notes) drives CLI dispatch AND `agent-context`; the existing `--help`↔`agent-context` parity test extends to the whole surface. No help/doc generation tooling beyond that.
- **No QuickType (PO 2026-11-02):** the protocol doc gains a JSON Schema description of the new ops; correctness is enforced by wire contract tests (raw framed clients, both serving sides) and CLI↔socket answer-equality tests; TS types stay hand-written.
- **Three-worlds routing (PO 2026-10-27):** layout/layout-adjacent commands route: app alive → live through the app socket; daemon-only → **refuse exit 3** with `appNotRunning` + `umux attach` hint (session and Storestation/Core commands work — daemon domain); nothing alive → offline behavior **with the `--desk`/`--term` requirement kept exactly as today** (PO 2026-11-02). Commands that inherently need a live target (`tabs send`, open/window actions) always require one — exit 3.
- **Handover rules (delegated to /carve by the PRD, pinned here):**
  - The app binds the socket only when a daemon probe fails (same lazy pattern as today's session-driver connect; bind failure → run client-only).
  - Storestation toggled ON: the app **stops its gateway server first**, then spawns/verifies the daemon (Windows `first_pipe_instance` forbids double-bind; Unix would orphan the socket file otherwise).
  - Toggled OFF: after the daemon's shutdown is confirmed, the app re-binds and serves.
  - Daemon crashes while ON: the app detects the dead client, takes over the socket, keeps in-process session fallback (today's behavior). **No automatic hand-back** — the daemon returns only through the explicit ON flow (deterministic, no two-server races).
  - `umux-storestation stop` against an app-served socket: honest answer (`stopped: false`, reason names the app as the server); it never kills the app.
  - Dev builds serve too (the socket is not single-instance; the single-instance plugin stays release-only and untouched).
- **Exit codes unchanged:** 0 ok / 2 usage / 3 required-but-unreachable / 4 conflict / 5 internal. Offline remains a state where a valid offline answer exists. New error codes are **additive**: `appNotRunning` first; clients treat unknown codes as generic.
- **Security unchanged:** local per-user transport only — Unix UDS `0600`, Windows per-user named pipe; no auth, no token, no network listener, ever.
- **Settings split by domain:** app-domain keys (notifications, agent status, session restore, ports tooltip, tab branch/folder display, default shell, sidebar width) execute in the app (UI toggles and CLI share one path); daemon-domain switches (Storestation enable/autostart, Core, Core Session) execute on the serving side and stay refused (exit 3) when nothing alive, as today.
- **sessions ops stay daemon-domain in semantics:** when the app serves the socket they map onto the app's live panels (list; write/resize/kill/subscribe/unsubscribe/status by panel mapping); `sessions.create` outside the daemon answers with guidance (the app spawns terminals through the UI, not headless).
- **Standing rule #101:** any interactive element added anywhere ships with spring press feedback (expected: none — this release adds no new UI controls).
- **Key entities (names pinned):** workspaceId / tabId / panelId (existing uuids), group tree + order arrays (store shape unchanged), `server`, `serverVersion`, `appNotRunning`, canonical catalog `workspaces·groups·tabs·panes·settings·sessions·storestation·core·core-session·update·reset·open·window` + globals (`status`, `attach`, `agent-context`, `notify`).

---

## Phase 1: One command table + v1.6.x retrofit (offline)

**User stories**: 6, 8, 9 (+ offline half of 1)

### What to build

The CLI's full command surface becomes one in-code table: canonical `resource.verb` names with the old short names registered as aliases of the same commands (`list`→`workspaces list`, `new`→`workspaces create`, `rm`→`workspaces delete`, `rename`→`workspaces rename`, `split`→`panes split`, `config get|set`→`settings get|set`, `export`/`import` under `workspaces`, `notify` stays global). `create`/`delete`/`update` work as aliases beside `new`/`rm`/`rename`. Nothing is removed and no behavior changes for old spellings. The retrofit: old store commands gain `--json` (same doc/error-object shapes as the live commands already use), the structured error catalog (`code`/`message`/`next`/`retryable` — emitted in `--json` mode instead of prose), and `--limit` (default 100) on `list` with a `truncated` marker. `agent-context` is generated from the table and the help-parity test covers the whole surface. Dispatch routes through the table.

### Assumptions carried in

- `store_core` and the store file formats are untouched; offline persistence behaves exactly as today (plain write, `--desk`/`--term` required).
- The existing error object shape (`protocol.rs`) and the existing `agent-context` parity test are the patterns to extend, not replace.

### Out of scope for this phase

- No live/daemon/app changes at all (no socket, no probe, no gateway).
- No exit-code changes (unknown workspace/setting still exits 1; missing flag still 2).
- Commands that don't exist yet (send, panes resize, update, …) are NOT declared in the table yet — the table grows per phase.

### Acceptance criteria

- [ ] Every old store command accepts `--json` and emits the doc/error-object shapes used by the live commands today — [test: `src-tauri/cli/tests/` retrofit suite]
- [ ] Alias equivalence: `umux new X`, `umux workspaces create X`, and future-proof `create` variants produce identical offline outcomes (store diff) — [test: alias equivalence suite]
- [ ] `umux list --limit 2` truncates with a `truncated` marker; default limit 100 — [test: list limit suite]
- [ ] `agent-context` is generated from the command table and matches `--help` and the real dispatch for the whole surface — [test: extended `agent_context_parity.rs`]
- [ ] Existing scripts keep exit codes and prose behavior without `--json` — [observable: run the documented v1.6.x script examples; test: no-`--json` outputs unchanged]

---

## Phase 2: App serves the socket — tracer (status + live read)

**User stories**: 3 (start), 10, 7 (foundation)

### What to build

The desktop app gains a gateway server on the SAME socket path: at startup (and lazily) it probes the daemon; if absent it binds and serves, if present it stays a client. Handshake works unchanged; the hello result gains `server: "app" | "daemon"` (additive). The app implements two read-only ops: `storestation.status` (app variant: the app's own health + `server` fields; Storestation block reports the daemon as not running when it is) and `workspaces.list` — the full live state (workspaces, groups, tabs, panels, order, agent states, colors, pins) **plus a `saved` section** with the on-disk definitions (story 1's two sections). The bridge: request → frontend executes the read against its live state → response. The CLI probes app-then-daemon: `umux status --json` reports `server`/`serverVersion`; `umux workspaces list --json` prints live+saved when the app is alive, saved-only offline (byte-identical behavior to today when nothing is alive). Stale-socket cleanup follows the existing daemon rules.

### Assumptions carried in

- Phase 1's command table and `workspaces list` canonical command exist.
- The storestation crate's protocol/transport/socketpath/client code is reused as-is (framing, `0600`, hello, `unknownOp`).
- The daemon never writes the store and the app's gateway inherits that rule — reads only in this phase.

### Out of scope for this phase

- No mutations through the socket; no daemon-side routing/forwarding; no handover flows beyond "probe first, bind if absent".
- `tabs send` and every other live command still don't exist.

### Acceptance criteria

- [ ] Raw framed client (wire test pattern): hello against the app → `{server:"app", proto:1, …}`; `workspaces.list` returns live+saved sections — [test: `storestation/tests/` gateway wire suite]
- [ ] With the app open: `umux status --json` → `server:"app"`, exit 0; `serverVersion` equals the app version — [command: `umux status --json`]
- [ ] With the app open: `umux workspaces list --json` matches the UI state (same workspaces/groups/tabs/panels, same order) — [command: diff against a UI-captured fixture; test: structure contract]
- [ ] With the daemon healthy (ON): the app does NOT bind — it registers as a client; `umux status --json` → `server:"daemon"` unchanged behavior — [test: app-side unit test; command: status while daemon ON]
- [ ] App closed: `umux status`/`workspaces list` behave byte-identically to today (offline state, exit 0) — [test: offline CLI suite]
- [ ] Normal terminal output stays byte-identical (parser passthrough rule untouched) — [test: existing OscParser suites]

---

## Phase 3: Three-worlds routing + live layout mutations

**User stories**: 1, 2, 6, 7

### What to build

The CLI routes layout commands through the three worlds: app alive → live through the app socket; daemon-only → exit 3 `appNotRunning` naming `umux attach` (daemon-domain commands keep working); nothing alive → offline with the `--desk`/`--term` requirement — and the offline mutation path **probes both sockets first and refuses to write files while anything is alive** (single-writer, story 7). The gateway implements the full layout mutation set through the bridge — the frontend executes the same pure ops the UI uses, so the UI updates instantly and persistence goes through the existing save path: workspaces (create, rename, delete, close, pin/unpin, color, move-to-group, move-to-new-group, reorder, open/activate), groups (create, rename, delete, subtree delete, unpack, pin/unpin, color, reorder/nest, collapse/expand), tabs (create, close incl. multi, rename, pin/unpin, color, reorder, move-to-workspace, separate), panes (split h/v, close). Tab creation honors the chosen shell / inherited cwd semantics like the UI. Live results echo the new entity ids so automation can chain commands.

### Assumptions carried in

- Phase 2's gateway, bridge and read path stand; this phase adds the mutate half of the same bridge.
- The UI's pure op modules cover every listed action already (they do — the inventory shows no gap in the layout domain).
- `panes split` (canonical) behaves exactly like today's `split` alias offline (same workspace-scoped semantics).

### Out of scope for this phase

- No daemon forwarding (Phase 4) — in the daemon-only world layout commands refuse.
- No `tabs send`, no pane resize/zoom/focus, no settings mutations (Phase 5a); no system actions (Phase 5b).

### Acceptance criteria

- [ ] Three worlds for `umux workspaces create "projekt"`: app closed → saved definition appears at next launch; app open → appears live in the UI without restart; daemon-only → exit 3, error code `appNotRunning`, hint names `umux attach` — [test: CLI routing unit tests; HITL: three-worlds walkthrough]
- [ ] Single-writer: with the app running, a mutation leaves `workspaces.json` mtime untouched and executes live — [test: mtime assertion in CLI integration test]
- [ ] Every layout mutation through the gateway produces the same state outcome as the matching UI action (pure-op parity) — [test: op-mapping parity suite]
- [ ] Live mutation responses echo entity ids (created/reordered targets addressable by the next command) — [test: gateway suite]
- [ ] Offline mutations keep requiring `--desk`/`--term`; probes prevent offline writes while the app or daemon is alive — [test: CLI integration tests]
- [ ] Existing alias suite (`new`, `rm`, `rename`, `split`) behaves exactly as before offline — [test: backward-compat suite]

---

## Phase 4: Daemon ON — handover + proxy routing

**User stories**: 6, 10 (completion)

### What to build

The protocol grows additively so the daemon can serve the whole catalog without owning the app's state: a desktop client registers itself as a gateway executor (hello with executor capability, or a `gateway.register` op — chosen at implementation); app-domain requests arriving at the daemon are wrapped into forward frames to the registered executor's connection, and the executor's answers are relayed back to the CLI connection (request id mapping server-side). No executor registered → `appNotRunning` (the daemon-only refusal). Session, Core and Core Session ops keep executing in the daemon exactly as today; Storestation enable/autostart and Core switches stay daemon-domain. The app registers as executor whenever it is connected to the daemon. Handover flows per the pinned rules: ON toggle (app releases the socket BEFORE the daemon starts), OFF toggle (app re-binds after the daemon's shutdown is confirmed), daemon crash (app takes over the socket + in-process fallback, no automatic hand-back), and `umux-storestation stop` answering honestly when the app serves. `umux status` reports `server:"daemon"` with the daemon block; `serverVersion` = daemon version (the existing version gate keeps comparing against whoever answers).

### Assumptions carried in

- Phase 2's app-server and Phase 3's mutations stand — Phase 4 only changes WHO serves and adds the forward path; op semantics are untouched.
- The app's daemon-client connection (persistent client) already exists and survives alongside a gateway executor role.

### Out of scope for this phase

- No new CLI commands; no settings/send/system actions (Phases 5a/5b).
- No automatic socket hand-back to a recovering daemon (pinned: explicit ON flow only).
- TUI/remote exposure (out of release scope).

### Acceptance criteria

- [ ] Daemon wire tests: app-domain op with no registered executor → `appNotRunning`; with an executor → forwarded, answer relayed byte-equivalent to the app's own answer — [test: `storestation/tests/` forward suite]
- [ ] Storestation ON with the app open: `umux workspaces create` works live (through the proxy); `umux status --json` → `server:"daemon"` — [command: full ON walkthrough]
- [ ] Toggle OFF → the app serves again (`server:"app"`); toggle ON → daemon serves (`server:"daemon"`); repeated toggles are race-free — [command: toggle loop; test: handover unit tests]
- [ ] SIGKILL the daemon while ON → the app takes over the socket; the CLI keeps working (`server:"app"`); sessions fall back in-process as today — [observable: status before/after; test: takeover unit test]
- [ ] `umux-storestation stop` while the app serves → honest `{stopped:false}`-style answer naming the app; the app stays alive — [command; test: CLI suite]
- [ ] Existing session/core/reattach/lifecycle suites stay green (no regression in the daemon's own domain) — [test: existing `storestation/tests/` + `cli/tests/` suites]

---

## Phase 5a: Live I/O — send, panes, settings, sessions

**User stories**: 2 (send), 4, 5 (partial)

### What to build

The inherently-live commands and the settings surface: `tabs send <tab> <text>` writes through the same path as keystrokes (the live stream stays byte-identical; the command itself is not terminal output); requires a live target — offline or daemon-only → exit 3 with the attach hint. Pane interaction: `panes resize` (explicit ratio/number replacing the mouse drag), `panes zoom`, `panes focus`. Settings: `settings get`/`set` for every app-domain key the UI exposes (notifications, agent status, session restore, ports tooltip, tab branch/folder display, default shell incl. custom, sidebar width) — the CLI mutation and the UI toggle take the same path, so outcomes can't diverge; `settings open` opens `settings.json` in the system editor. Daemon-domain switches route to the serving side (Storestation enable/autostart, `core on|off`, `core-session` — the existing commands keep their behavior and refusals). Sessions when the app serves: `sessions list` returns the app's live panels; `session.write/resize/kill/subscribe/unsubscribe/status` map onto live panels by id; `sessions.create` outside the daemon answers with guidance instead of spawning anything headless.

### Assumptions carried in

- Phases 2–4 stand: the bridge, routing, proxy and handover are all live; this phase fills the op catalog on both serving sides.
- The `--json`/error-catalog patterns from Phase 1 apply to every new command from day one.

### Out of scope for this phase

- No system actions (Phase 5b); no docs.
- No pane max/balance (doesn't exist in the UI; not in the inventory).

### Acceptance criteria

- [ ] `umux tabs send <tab> "echo hi"` with the app open → the text lands in that panel's shell and executes — [HITL; test: op-mapping suite]
- [ ] `tabs send` offline / daemon-only → exit 3 error object naming the reason and `umux attach` — [test: CLI suite]
- [ ] `panes resize/zoom/focus` reproduce the drag/button outcomes (persisted ratio, zoom state, focused pane) — [test: parity suite; HITL]
- [ ] Every app-domain settings key: `settings set` == the UI toggle outcome (store diff equality); unknown key → enumerated error (today's behavior preserved) — [test: settings parity suite]
- [ ] Daemon-domain commands offline keep exit 3 (existing behavior); through the daemon they behave as today — [test: existing `core_lifecycle.rs` untouched + new routing cases]
- [ ] `sessions list` when the app serves matches the app's live panels; subscribe/write/kill against a panel behave like their daemon counterparts — [test: gateway suite; command: live walkthrough]
- [ ] Terminal output passthrough remains byte-identical (the send path adds no mutations) — [test: existing OscParser/session suites]

---

## Phase 5b: System actions + parity fixture

**User stories**: 4, 5 (completion)

### What to build

The system-action commands (full 1:1 parity, PO 2026-11-02): `umux update check` and `umux update install` through the gateway (same updater as Settings; install honestly reports the relaunch boundary — the answer describes what will happen, the app performs it), `umux reset` (live: through the gateway — reset + relaunch; offline: removes both store files after an explicit `--yes`; without it → enumerated refusal), `umux open folder <target>` / `umux open port <tab> <port>` / `umux open settings` (the same system openers the UI uses), `umux window close|minimize|toggle-maximize` (live-only, exit 3 otherwise), and the inspect reads: `tabs ports` and `tabs branches` (the tooltip/label data). Then the parity lock: the UI inventory (Appendix) becomes a data fixture in tests — every UI action maps to a command + op; a check fails when the UI gains an action without a mapping. The contract test asserts CLI answers equal raw-socket answers op-by-op. `agent-context` reaches its final shape (whole catalog, examples included).

### Assumptions carried in

- Phases 1–5a stand; every mechanism (routing, proxy, bridge, settings) is proven — this phase adds the last commands and the enforcement tests.
- The updater/reset/system-open paths exist as app-internal commands today; the gateway exposes them without reimplementing them.

### Out of scope for this phase

- No docs content (Phase 6); no new UI.

### Acceptance criteria

- [ ] Parity fixture: every inventoried UI action maps to a command + op; adding a UI action without a mapping fails the suite — [test: parity fixture suite driven by the Appendix inventory]
- [ ] Contract test: for every op, the CLI's answer equals a raw-socket client's answer (same doc, both serving sides) — [test: CLI↔socket equality suite]
- [ ] `umux update check` with the app open mirrors the Settings row result; `update install` reports the relaunch honestly — [command; HITL once]
- [ ] `umux reset` offline without `--yes` refuses with an enumerated error; with `--yes` it removes both store files; live reset goes through the gateway and relaunches — [command; test: CLI suite]
- [ ] `umux open port <tab> <port>` opens the browser; `open folder` opens the file manager; `open settings` opens the editor — [command; HITL once]
- [ ] `umux window close|minimize|toggle-maximize` work live; offline → exit 3 — [command; test]
- [ ] `tabs ports`/`tabs branches` return the same data the UI tooltip/labels show — [test: parity suite]

---

## Phase 6: Documentation

**User stories**: 11

### What to build

README gains a "Live CLI" section: the three-worlds model in plain words, the exit codes, `agent-context`-based agent examples (a script that inspects, a script that drives layout, an agent steering a terminal), and the socket security note. The landing Docs section grows the same content in landing shape. The protocol reference doc (`umux-storestation-cli-protocol.md`) is refreshed: the additive extension (executor registration/forward frames), the new op catalog, `appNotRunning`, and the `server`/`serverVersion` hello fields. **Landing changes are gated: shown to the PO for acceptance first, pushed to `main` only after explicit OK** (standing landing rule — pushing IS publishing).

### Assumptions carried in

- Phases 1–5b stand: the documented surface is shipped and verified; docs describe what exists (no aspirational copy).
- Docs describe the latest released version (standing docs rule).

### Out of scope for this phase

- No standalone docs site (future work, out of release scope).

### Acceptance criteria

- [ ] README "Live CLI" section exists and every example runs verbatim on the PO's machine — [observable; command: copy-paste walkthrough]
- [ ] Landing Docs CLI content drafted and shown to the PO; pushed only after explicit OK — [observable: PO acceptance recorded before push]
- [ ] Protocol doc updated with the additive extension + new catalog; no contradictions with the shipped wire tests — [observable: doc review against test names]
- [ ] `agent-context` examples in docs parse as valid JSON — [command: doc example extraction script]

---

## Appendix: Parity inventory (fixture source, PO 2026-10-27 full-1:1 decision)

The UI action inventory drawn at carve (2026-11-02, from the shipped v1.7.5–v1.7.7 UI). Grouped by area; each row: UI action → canonical command. This table is the seed of the Phase 5b fixture; implementation re-verifies each row against the actual UI before declaring parity.

### Workspaces
| UI action (entry point today) | Command |
|---|---|
| Create ("+" / menu / Ctrl+Shift+N) | `workspaces create` |
| Switch/activate (row click, Ctrl+Shift+←/→) | `workspaces open` |
| Rename (context menu → inline) | `workspaces rename` |
| Delete (× / menu, confirm dialog) | `workspaces delete` |
| Close, keep definition (batch only today) | `workspaces close` |
| Pin/unpin (menu) | `workspaces pin` / `workspaces unpin` |
| Reorder (DRAG-ONLY) | `workspaces reorder` |
| Move into existing group (drag/menu picker) | `workspaces move --group <g>` |
| Move to NEW group (menu picker) | `workspaces move --new-group <name>` |
| Set/clear color (menu swatches, 8 palette) | `workspaces color <c>` / `workspaces color clear` |
| Batch multi-select (Ctrl+click) | addressed by multi-id args on the verbs |
| Batch pin/unpin/close/delete | `workspaces pin/unpin/close/delete` with multi-id args |
| Inspector: agent chips, folder lines, branch labels, cwd | covered by `workspaces list` / `tabs branches` reads |

### Groups
| UI action | Command |
|---|---|
| Create / rename / delete (+subtree confirm) | `groups create` / `groups rename` / `groups delete` |
| Unpack (dissolve) | `groups unpack` |
| Pin/unpin | `groups pin` / `groups unpin` |
| Reorder / nest (DRAG-ONLY) | `groups move` |
| Collapse/expand | `groups collapse` / `groups expand` |
| Set/clear color | `groups color` |

### Tabs
| UI action | Command |
|---|---|
| Create ("+" / Ctrl+Shift+T) | `tabs create` |
| Create in chosen shell (chevron picker) | `tabs create --shell <s>` |
| Switch (click; NO keyboard cycling today) | `tabs open` |
| Close (× / menu; busy confirm) | `tabs close` (multi-id for bulk) |
| Rename (double-click / menu) | `tabs rename` |
| Pin/unpin | `tabs pin` / `tabs unpin` |
| Reorder (DRAG-ONLY) | `tabs reorder` |
| Move to workspace (menu picker, batch) | `tabs move --workspace <w>` |
| Color | `tabs color` |
| Separate split into tabs (menu) | `tabs separate` |
| Fullscreen chrome-hide (menu / Esc) | `tabs fullscreen on` / `off` |
| Send text to terminal | `tabs send` |
| Ports tooltip (hover → copy/open) | `tabs ports` + `open port` |
| Branch labels (auto) | `tabs branches` |

### Panes
| UI action | Command |
|---|---|
| Split h/v (Ctrl+Shift+H/E, menu) | `panes split --horizontal/--vertical` (alias: `split`) |
| Resize (divider DRAG-ONLY) | `panes resize --ratio <n>` |
| Close (× / Ctrl+Shift+W) | `panes close` |
| Focus (click/typing) | `panes focus` |
| Zoom/unzoom (button / Ctrl+Shift+Z) | `panes zoom` |

### Settings (app-domain keys)
| UI toggle | Command |
|---|---|
| notifications / agent status / session restore / ports tooltip / tab branch / tab folders | `settings set <key> <bool>` (six keys) |
| Default shell (picker + custom dialog) | `settings set default-shell <s>` / auto |
| Sidebar width (DRAG persists) | `settings set sidebar-width <n>` |
| Open settings.json (footnote) | `settings open` |
| Daemon-domain (Storestation on/off/autostart, Core, Core Session) | existing `storestation`/`core`/`core-session` commands, routed to the serving side |

### Sessions / live I/O
| UI action | Command |
|---|---|
| Live panels inspect | `sessions list` (app-served mapping) |
| Attach to panel stream | `session subscribe` op (CLI exposes via sessions verbs) |
| Kill a live panel's shell | `session kill` op via `sessions` verbs |

### System actions (full-1:1, PO 2026-11-02)
| UI action | Command |
|---|---|
| Check for updates / Download & restart | `update check` / `update install` |
| Reset umux (two-click arm) | `umux reset` (offline: requires `--yes`) |
| Folder line double-click → file manager | `open folder <target>` |
| Port click → browser | `open port <tab> <port>` |
| Window minimize/maximize/close (header menu) | `window minimize` / `window toggle-maximize` / `window close` |
| Notification bell (mute) | `settings set notifications-enabled` (same switch) |
| cmux import wizard | offline `import cmux` stays; live import rides the gateway (plan applies through the app) |
| EmptyState "Create workspace" | `workspaces create` |
| Config-fallback banner dismiss | transient UI state — not persistable, no command (documented in the fixture as UI-only) |

**UI-only exceptions recorded in the fixture** (expressible as data but meaningless without a screen): inline rename inputs, drag ghost/insertion-line visuals, sidebar drag handle itself (width is the persisted counterpart), splash/banner transient states. Everything else must resolve to a command.
