# Plan: umux v1.7.0 — umux Storestation (+ landing-page revamp)

> Source PRD: [`umux-v1.7.0-prd.md`](./umux-v1.7.0-prd.md) (extract; master [`umux-prd.md`](./umux-prd.md) wins on conflict)
> CLI/protocol spec: [`umux-storestation-cli-protocol.md`](./umux-storestation-cli-protocol.md) — the command/wire contract for phases 1, 2, 4, 5 and the reference for the v1.8.0 live CLI.
> Carved 2026-09-19. Scope decisions from planning with Adam: 10 phases approved; landing = "docs + sections"; v1.7.0 CLI stays minimal (status/attach/list) because the full live-CLI expansion is v1.8.0 itself.

## Architectural decisions

Durable decisions that apply across all phases:

- **Architecture style — one session engine, three faces.** PTY/session logic moves into a shared workspace crate (`session_core`) linked by (a) the desktop app in-process (Storestation OFF) and (b) the new headless daemon binary `umux-storestation` (Storestation ON), which serves it over a local socket. The desktop app and CLI become socket *clients* through a daemon-client driver implementing the same `SessionCore` interface as the in-process driver; the UI never knows which driver is active.
- **Storestation OFF is additive zero-change.** With the daemon never enabled the app behaves exactly like v1.6.x — guarded by the existing suites plus a manual parity checklist (phase 3).
- **Transport & isolation.** One local per-user socket (named pipe on Windows / UDS `0600` on macOS+Linux) whose name/path derives from the config directory, so `UMUX_CONFIG_DIR` isolates store + socket + sessions together for test instances (e.g. `~/.umux-test`). Protocol v1 per the design doc: length-prefixed frames (control JSON / binary data), versioned handshake, camelCase envelopes, `resource.verb` ops, unknown-op rejection.
- **Data model.** Storestation keeps a runtime **session registry** (session id ↔ workspace/tab/panel ids, title, cwd, shell, cols/rows, createdAt, attached clients) and a bounded **scrollback ring** per session. `workspaces.json`/`settings.json` stay app-owned; only additive settings fields appear: `storestation { daemonEnabled: false, autostartEnabled: false }`.
- **Ownership when Storestation ON.** Storestation owns the live sessions and their shells; the app rebinds panels to registry-matched sessions at startup. Shell lifetime is bound to the daemon by OS primitives (Windows job object kill-on-close / Unix process groups + SIGHUP): a clean stop kills everything, a crash leaves no orphans, and stale socket/pid/registry files are detected and cleaned on the next start.
- **Parsing stays client-side.** The daemon streams raw bytes untouched; OscParser runs in the attached view (byte-identical rule preserved). Notifications fire in attached views only — daemon-side notifications when nothing is attached are out of scope (candidate follow-up).
- **CLI/agent contract.** New commands ship with canonical `--json`, the exit-code catalog (0/2/3/4/5), the error object `{code, message, next, retryable}`, `umux agent-context`, and global `--config-dir` (flag > env > default). Existing v1.6.x commands unchanged; their retrofit (`--json`, verb aliases) is v1.8.0.
- **Autostart mechanisms (decided at /carve):** Windows HKCU Run key · macOS LaunchAgent plist · Linux systemd user unit — always under user toggles, off by default.
- **Packaging.** `umux-storestation` rides the existing sidecar mechanism (`externalBin` + build script) into every installer and updates with the app via the existing updater; zero-cost unsigned policy unchanged. `tauri-plugin-single-instance` is added in phase 5 so `umux attach` is idempotent.
- **Process.** Development on the long-lived **`core` branch**, releases tagged from `main`; HITL order **macOS + Windows first, Ubuntu after** (PO decision 2026-09-12). Known pre-existing failures on `main` (2 flaky WorkspaceShell, 2 Readme.test.ts, 14 cargo shell-tests) are the **baseline** — phase gates compare against it; they are not regressions to fix here.
- **Housekeeping before phase 1 (Adam's call — no git actions without consent):** `main` carries uncommitted changes and is 1 commit behind `origin/main`; the `core` branch does not exist yet and is created from an up-to-date `main`.

**Story coverage:** #105 → P2, P4, P5 · #106 → P5 · #107 → P3, P4 · #108 → P4, P7 · #109 → P7 · #110 → P2, P6 · #111 → P1, P2, P5 (completes at v1.8.0) · #112 → out of scope (v1.9.0; protocol ready for it) · #113 → P8 · #136 → P9.

---

## Phase 1: Daemon skeleton — `umux-storestation`, local socket, `umux status`

**User stories**: #105 (foundation), #111 (partial), #113 (groundwork)

### What to build

New workspace crate with the headless `umux-storestation` binary: resolves the config dir, creates the per-user socket, serves the protocol v1 handshake plus `storestation.status` and `storestation.shutdown`, enforces single instance, cleans stale socket/pid leftovers on start, and shuts down gracefully on Ctrl+C / `stop`. The `umux` CLI gains `status [--json]`, `agent-context`, and the global `--config-dir` flag; the exit-code catalog and error-object shape land now. Offline behavior of every existing command is unchanged.

### Assumptions carried in

- `store_core::paths` already centralizes config-dir resolution and `UMUX_CONFIG_DIR` — reused for socket location.
- The CLI integration-test pattern (`CARGO_BIN_EXE_*` + tempdir + env) exists in `src-tauri/cli/tests/` — replicated for the daemon.

### Out of scope for this phase

- No real sessions/PTYs (`sessions.list` returns empty). No Settings UI, no autostart, no installer bundling, no app changes.

### Acceptance criteria

- [ ] Daemon serves a temp-dir instance; `umux status --json` reports `{running:true, protocol:1, daemonVersion}` — [test: `src-tauri/cli/tests/storestation_lifecycle.rs`]
- [ ] Second `umux-storestation run` against the live socket exits 4 with `storestationAlreadyRunning` (message names the running pid) — [test: same file]
- [ ] `umux-storestation stop` → daemon exits, socket+pid files removed; subsequent `umux status` → `{running:false}` with exit 0 — [test]
- [ ] Bogus leftover socket/pid file → `umux status` reports `{running:false, staleSocket:true}` exit 0; next daemon start cleans it — [test]
- [ ] `umux status` with stdin closed and no daemon: exits promptly, never hangs, same output shape offline — [test: DEVNULL-stdin run]
- [ ] `umux agent-context` emits schema-1 JSON; a parity test asserts it matches the `--help` surface (commands, flags, exit codes) — [test: `agent_context_parity`]
- [ ] Handshake: client `hello` with a newer protocol major → `protoTooNew` error and a clean close — [test: `src-tauri/storestation/tests/protocol.rs`]
- [ ] HITL (Windows): run `umux-storestation` in a terminal → `umux status` shows running + version; `umux-storestation stop` stops it; double start refuses — [command: manual script]

---

## Phase 2: Shared session engine — Storestation owns real shells

**User stories**: #105 (Storestation owns PTYs), #110 (clean-stop half), #111 (partial — `sessions list`)

### What to build

Extract the PTY/session logic into the shared crate `session_core` (the app links it unchanged — a behavior-identical refactor). The daemon gains the real session ops over the socket: `sessions.create` (client-generated id, shell, cwd, size), `session.write`, `session.resize`, `session.kill`, `session.subscribe` (binary output frames + lifecycle events), `sessions.list` — implemented in v1.7.0 because the desktop daemon-client driver (phase 4) is their client; only `sessions list` is exposed as a CLI command now (create/kill/write become CLI commands at v1.8.0). A per-session bounded scrollback ring starts capturing from birth (replay is consumed in phase 5). Clean stop: daemon shutdown kills every owned shell — OS primitives bind child lifetime to the daemon.

### Assumptions carried in

- Phase 1 daemon/socket/handshake are stable; `portable-pty` stays the PTY backend (ConPTY on Windows).
- Session ids are client-generated UUIDv4 (idempotent create-by-key groundwork for v1.8.0 retries).

### Out of scope for this phase

- No desktop app wiring (phase 4), no scrollback replay (phase 5), no crash-hardening specifics (phase 6). The CLI does NOT gain create/kill commands (v1.8.0).

### Acceptance criteria

- [ ] `session_core` builds; Rust + frontend suites match the known baseline (refactor changes no behavior) — [test: `cargo test` + `npm test` vs baseline]
- [ ] Integration: create a session, write `echo umux-probe`, output frames contain it, kill the session → child process gone — [test: `src-tauri/storestation/tests/sessions_live.rs`]
- [ ] Two live shells + `umux-storestation stop` → zero descendant processes within 5 s — [test + command: manual process audit]
- [ ] `umux sessions list --json` lists live sessions (id/cwd/shell/size/attachedClients); `--limit` truncates with `truncated:true` — [test]
- [ ] Scrollback ring is bounded (cap respected, oldest dropped) — [test: `session_core` unit]
- [ ] Unknown op over the socket → `unknownOp` error; connection stays open — [test]

---

## Phase 3: SessionCore seam in the app — in-process driver, zero behavior change

**User stories**: #107 (guard — Storestation OFF = exactly today)

### What to build

Introduce the `SessionCore` driver trait inside the app's invoke layer; today's logic becomes the in-process driver. The commands (`pty_*`, `ssh_*`, cwd snapshot) delegate through the seam. No UI change, no persistence change — the point is that phase 4 swaps drivers without touching feature code.

### Assumptions carried in

- Phase 2's `session_core` exists; the trait shape mirrors the command surface already in the invoke layer.

### Out of scope for this phase

- No daemon-client driver, no Settings section, no daemon spawning.

### Acceptance criteria

- [ ] All app session commands route through the driver trait (single seam; no direct engine calls outside the in-process driver) — [observable: code audit]
- [ ] Rust + frontend suites match the baseline — [test]
- [ ] Manual parity checklist passes: open/write/resize/close panel, unlimited splits, agent-status chips, OSC notification fires, SSH panel connects, session restore on relaunch, cmux import works — [observable: HITL checklist, Windows]

---

## Phase 4: Daemon-client driver + Storestation section in Settings — the survival demo

**User stories**: #105, #107 (verified), #108 (daemon + status parts), #111 (partial)

### What to build

The second driver: every session op proxied over the socket (subscription = push frames → the same reader pipeline: OscParser → notifications → terminal). Settings gains the **Storestation section**, styled like the import wizard: daemon on/off (default off) + live status (running/stopped, version, session count). Toggle ON spawns the bundled `umux-storestation` if absent and switches **new** sessions to Storestation; toggle OFF stops the daemon — with a confirmation dialog when live sessions exist (they die cleanly, #110 semantics). On startup with Storestation ON, the app asks `sessions.list` and **rebinds** panels to registry-matched live sessions instead of spawning fresh shells (matching by workspace/tab/panel ids). Sessions already open when toggling are not migrated — they stay where they are.

### Assumptions carried in

- Phases 1–3: daemon, session ops, driver seam.
- Bundled-binary path resolution reuses the CLI sidecar conventions (real bundling is verified in phase 8; until then a dev `umux-storestation` on PATH is acceptable).

### Out of scope for this phase

- No scrollback replay (reattached panels start empty — fixed in phase 5), no `umux attach` (phase 5), no autostart row (phase 7), no installer bundling (phase 8). SSH sessions stay app-side even with Storestation ON.

### Acceptance criteria

- [ ] Storestation section renders; daemon toggle + status persist across restarts — [test: `src/SettingsDialog.test.tsx` extended]
- [ ] Storestation OFF → behavior identical to the baseline (suites + checklist) — [test + observable]
- [ ] Storestation ON → a new panel is Storestation-owned: `umux sessions list` shows it with matching ids — [command]
- [ ] Driver-level survival: connect, create a session, write, disconnect the client, reconnect, subscribe → same session id, output continues — [test: `storestation/tests/reattach.rs`]
- [ ] THE DEMO (HITL, Windows then macOS): Storestation ON, agent running in a panel, close every umux window → `umux status` still lists the session and output grows; reopen umux → the panel rebinds to the same session (scrollback empty — expected until phase 5) — [observable]
- [ ] Toggle OFF with live sessions → confirmation dialog naming the count; after confirm, the daemon stops with zero orphan shells — [test: dialog] + [observable: process audit]
- [ ] Byte-identical rule: the daemon passes bytes untouched; rendered output identical with either driver (OSC passthrough fixtures) — [test]

---

## Phase 5: `umux attach` + scrollback replay + single instance

**User stories**: #106, #111 (partial)

### What to build

Scrollback replay: on rebind/attach the client writes the ring-buffer prefix before consuming live frames (strict replay→live ordering, no gaps, no duplication). CLI `umux attach [--json] [--dry-run]`: launches (or focuses) the desktop app bound to Storestation; Storestation offline → exit 3 `storestationNotRunning` with enumerated next steps. The app gains `tauri-plugin-single-instance` so a second launch focuses the existing window (attach becomes idempotent; the store double-writer risk disappears).

### Assumptions carried in

- Phase 4 rebind works; the phase 2 ring buffer holds history.
- App-executable lookup follows the installer layouts (the CLI sits beside the app binary in NSIS/deb; macOS via bundle path / `open`) — exact resolution decided at implementation and recorded in the design doc.

### Out of scope for this phase

- No interactive/terminal CLI attach (TUI territory, v1.9.0 story #112); no remote/multi-machine anything.

### Acceptance criteria

- [ ] Replay ordering: write a known byte pattern, disconnect, reconnect, subscribe → first frames equal the recorded prefix, then live bytes, no gap or duplication — [test]
- [ ] `umux attach --json` contract: `{launched:true, appPid}` / `{launched:false, reason:"alreadyRunning", focused:true}` / exit-3 error object offline — [test]
- [ ] `attach --dry-run` prints the resolved app path and launches nothing — [test]
- [ ] A second `umux attach` / app launch focuses the existing window instead of spawning a duplicate — [test: single-instance plugin] + [observable]
- [ ] HITL (Windows, then macOS): close the app mid-agent, `umux attach` → window opens, panel shows history + live output — [observable]

---

## Phase 6: Lifecycle hardening — crash leftovers

**User stories**: #110 (crash half)

### What to build

Hard-kill resilience: owned shells die with the daemon via OS primitives (job object kill-on-close on Windows; process groups + SIGHUP on Unix). The next start detects and cleans stale registry/pid/socket state; clients during downtime get a clean, bounded connection-refused (never a hang). A corrupted registry is tolerated — it rebuilds as sessions re-register; store files are untouched.

### Assumptions carried in

- The phase 2 clean-stop audit as the baseline for the crash variant.

### Out of scope for this phase

- No session adoption/resurrection after a crash (sessions die with the daemon by design); no daemon auto-restart loop.

### Acceptance criteria

- [ ] Kill -9 the daemon with 2 live shells → both child processes exit within 5 s (Windows job object; Unix group signal) — [test: platform-gated]
- [ ] After a crash: the next start cleans stale socket/pid/registry markers; `umux status` is healthy immediately after — [test]
- [ ] A client during daemon absence fails bounded with `storestationNotRunning`, no hang (connect timeout 3 s) — [test]
- [ ] HITL: `taskkill /F` (Windows) / `kill -9` (macOS) while an agent runs → no orphans in Task Manager / Activity Monitor; restart clean — [command: audit script]

---

## Phase 7: Autostart at login

**User stories**: #108 (completed — autostart row), #109

### What to build

The autostart toggle in the Storestation section. Mechanisms: Windows HKCU `Run` key (no-window launch), macOS LaunchAgent plist in `~/Library/LaunchAgents`, Linux systemd user unit. Autostart launches `umux-storestation run` headless from the installed location. Disabling removes the mechanism fully. Per-platform artifact generation is unit-tested; enabling is HITL-verified per OS (macOS + Windows first, Ubuntu after).

### Assumptions carried in

- The installed `umux-storestation` location is stable from phase 8 bundling (dev fallback: current exe path).

### Out of scope for this phase

- No autostart of the desktop app (daemon only); no admin/elevated paths; off by default everywhere.

### Acceptance criteria

- [ ] Unit tests generate a valid Run-key value / LaunchAgent plist / systemd unit per platform — [test: `autostart` units]
- [ ] Toggle persists; enable→disable removes the mechanism fully — [test: settings store]
- [ ] HITL per platform: enable, reboot/relogin → daemon running with no window (`umux status` green); disable → absent after reboot — [command]
- [ ] The Storestation section shows all three controls (daemon, autostart, status) — [test: SettingsDialog] — completes #108

---

## Phase 8: Storestation inside the installers + updates

**User stories**: #113

### What to build

Extend the sidecar build script to produce `umux-storestation` for every target (universal macOS = both arches + lipo, like the CLI) and add it to `externalBin` — every installer (NSIS, dmg, deb, AppImage, rpm) ships it beside `umux`. The NSIS PATH hook picks it up automatically; Linux follows the existing CLI PATH conventions; macOS PATH is documented via the existing `install.sh`, extended for the daemon binary. The updater is untouched — a normal app update replaces the daemon binary too; versions stay locked to the workspace version.

### Assumptions carried in

- Phases 1–7 tolerate the packaged layout (dev builds unaffected).

### Out of scope for this phase

- No code signing (zero-cost policy); no standalone daemon installer or independent daemon updates (Storestation updates only with the app); Linux HITL waits for Windows/macOS to pass (order).

### Acceptance criteria

- [ ] CI release artifacts contain `umux-storestation` for all five targets — [observable: release.yml artifacts]
- [ ] Fresh install on Windows (NSIS) and macOS (dmg): Settings → Storestation ON works with zero extra downloads — [command: HITL, primary platforms]
- [ ] Update flow (old → new via the updater): daemon binary replaced; `umux status` reports the new version — [observable: HITL]
- [ ] Uninstall removes the daemon binary and (NSIS hook) the PATH entry — [observable]

---

## Phase 9: Landing-page revamp — docs + sections

**User stories**: #136

### What to build

Expand the static `landing/` one-pager (no framework, no build step) with: a **docs section** (per-platform install incl. the SmartScreen/Gatekeeper notes, build from source, a feature tour distilled from the README), a **short roadmap/changelog block**, and a **"what's next" ecosystem teaser** (Bridge/PWA/NativeApps mention). Same deployment: Cloudflare Pages serves `landing/` from `main` — zero-cost, unchanged. The HTML-parsing test is extended.

### Assumptions carried in

- The v1.7.0 feature set is final enough to document (this phase lands after 8; copy is adjusted at release if needed).

### Out of scope for this phase

- No multi-page docs site (possible in a later version), no search, no i18n, no analytics.

### Acceptance criteria

- [ ] All sections render; internal anchors resolve; no dead external links — [test: `landing/index.test.ts` extended]
- [ ] Docs content matches shipped reality (install steps validated against the actual installers) — [observable: HITL walkthrough]
- [ ] The deploy stays on the free tier / no new infrastructure — [observable]

---

## Phase 10: Integration & release from `core`

**User stories**: all (#105–#113, #136)

### What to build

Merge `core` → `main`; bump the version to 1.7.0 across the workspace, `tauri.conf.json` and `package.json`; the README gains the Storestation section (daemon, autostart, `attach`, the CLI live-commands table, unchanged zero-cost workarounds); PRD status updates (master + extract). Full HITL sweep in PO order: macOS + Windows first, Ubuntu last (including reboot/autostart and the updater flow). The release happens only on Adam's explicit ship command.

### Acceptance criteria

- [ ] The PRD validation list from the extract passes on macOS + Windows, then Ubuntu — [observable: HITL protocol]
- [ ] README/PRD status synced; landing live — [observable]
- [ ] Tag + release from `main` after Adam's go — [command]
