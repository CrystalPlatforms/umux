# Plan: umux v1.7.6 — Windows Stability & UX Quickupdate

> Source PRD: `plans/umux-v1.7.6-prd.md` · GitHub issue #99
> Carved 2026-10-10. Ship order: bug phases (1–5) first, feature phases (6–8) second — PO decision.

## Architectural decisions

Durable decisions that apply across all phases:

- **Architecture style**: unchanged two-process Tauri model. The daemon stays the
  owner of the session engine; the desktop app is its client when Storestation
  is ON. The local socket protocol (versioned hello handshake) does not change —
  both sides already announce their versions.
- **Daemon delivery (Windows)**: revised 2026-10-15 (PO decision after P3
  discovery measurements). The daemon binary STAYS console-subsystem — a
  GUI-subsystem build was implemented and measured first, and Ctrl+C under a
  live PowerShell prompt never reaches even an attached daemon (PSReadLine's
  raw input mode), which breaks the "Ctrl+C stops a terminal-run daemon"
  requirement irreparably. Zero windows at login is delivered by the
  autostart artifact wrapping the daemon in `conhost --headless` (verified
  on Windows 11 26300); app-side spawns get no-window creation flags.
  Console-less launches keep the CLI verbs panic-free through an
  output-routing decision: valid handles → print, attachable parent console
  → print through it, neither → discard.
- **Version gate**: ONE choke point in the daemon-client driver decides
  compatibility from the hello handshake. Action for an older/incompatible
  daemon: graceful stop → spawn the bundled daemon → re-verify. Last resort:
  a Storestation error card in Settings. The app must never hard-crash on
  daemon-provided data.
- **Autostart**: dev builds refuse to install or modify the OS autostart
  artifact; release builds self-heal it to point at the installed daemon at
  startup. Artifacts stay as they are (Run key / LaunchAgent / systemd unit).
- **App-side process spawns** (daemon launch) carry no-window/no-console
  creation flags on Windows.
- **UI modules**: one shared overlay-scrollbar module (styles + scroll-flash
  behavior) reused across regions; tab multi-selection as a pure reducer with
  UI wiring on top; app version read through the app's own version API.
- **Quality gates**: `npm test` and `cargo test` stay at the documented
  baselines (known flaky lists excluded) after every phase.

---

## Phase 1: Reproduce & fix the startup crash

**User stories**: 3 (no crash with a daemon alive), PRD decision "repro-first".

### What to build

With a deliberately older daemon answering on the socket (the stale dev-build
daemon still on the PO's machine, or a protocol-level stand-in), launch the
release app with backtraces enabled and capture the exact panic behind the
observed abort. Fix that specific defect so the app boots and stays alive with
an old daemon running — even if session features still misbehave until Phase 2
teaches it to replace the daemon. Record the captured backtrace in the phase
notes so Phase 2 verifies its gate against a known crash, not a hypothesis.

### Assumptions carried in

- The PO's Windows machine is the reproduction environment (deterministic
  repro already observed: daemon alive → app dies ~7 s after launch).
- The Event Log entries from 2026-10-10 (0xc0000409, faulting module
  umux-app.exe) are the same failure mode being reproduced.

### Out of scope for this phase

- Auto-replacement of the old daemon (Phase 2).
- Any Settings UI or error card (Phase 2).
- Console-window work (Phases 3–4).

### Acceptance criteria

- [ ] Panic reproduced and captured with a backtrace before the fix — [observable: backtrace log recorded in the phase notes]
- [ ] With an older daemon running, the app starts and stays alive: 10 consecutive launches, zero aborts — [observable: no new 0xc0000409 entries in Event Log]
- [ ] Frontend and backend suites at known baselines — [command: `npm test` and `cargo test` pass at documented flaky baseline]

---

## Phase 2: Version gate & automatic daemon replacement

**User stories**: 4 (auto-replace older daemon), 5 (graceful error instead of crash).

### What to build

At the single driver choke point, compare the daemon version from the hello
handshake against the app version. Older or incompatible → stop the running
daemon gracefully, spawn the bundled daemon, re-verify the handshake; surface
the swap silently if it succeeds. If replacement is impossible, show a clear
Storestation error card in Settings — the app process itself never aborts. In
the same pass, audit every boot-path consumer of daemon-provided data and
replace hard failures with typed errors. Include the message content for an
older APP meeting a newer daemon (error card, no downgrade attempt).

### Assumptions carried in

- Phase 1 removed the crash mode; this phase converts it into self-healing.
- The hello handshake already carries both versions; the protocol does not
  change.

### Out of scope for this phase

- Console-window work (Phases 3–4).
- Remote/SSH daemon scenarios (v2.5.0 scope).

### Acceptance criteria

- [ ] Gate logic accepts same-version and rejects older / newer-major with a typed outcome — [test: driver-crate unit tests over fixture handshakes]
- [ ] With a stale daemon running, app start → within 15 s the responding daemon reports the app's version — [observable: daemon status output after app start]
- [ ] Simulated incompatible daemon → Settings shows the error card; app process alive after ≥ 5 min of interaction — [observable + HITL]
- [ ] Boot-path audit: no daemon-data consumer on a startup path can hard-crash — [observable: audit checklist recorded; driver unit tests cover malformed replies]

---

## Phase 3: Windowless daemon

**User stories**: 1 (never any console window), 2 (terminal commands still print).

### What to build

Keep the daemon binary console-subsystem (revised 2026-10-15, PO decision —
the original GUI-subsystem approach was built and measured; a live PowerShell
prompt swallows Ctrl+C so the terminal stop requirement is unreachable in
that design). Terminal UX stays exactly v1.7.5: the shell waits on `run`,
Ctrl+C stops it, banners and error objects print as today. What P3 adds is
the output-routing decision for console-less launches (app-side
CREATE_NO_WINDOW spawn, conhost --headless Run-key host — both wired in
phase 4): valid handles → print as today, attachable parent console → print
through it, neither → park the dead streams on the NUL device so prints
never panic. Verify the headless-conhost hosting on the PO's machine so
phase 4's artifact rewrite stands on proven ground.

### Assumptions carried in

- Windows Terminal is the default terminal host on the PO's machine (the case
  the old hide-mechanism missed).
- The installer ships the daemon beside the app (unchanged layout).

### Out of scope for this phase

- The app-side spawn flags and autostart changes (Phase 4) — until then, a
  login launch may still use an old autostart entry.
- macOS/Linux behavior (daemon already runs fine there; verbs unchanged).

### Acceptance criteria

- [ ] Binary stays console-subsystem (PE header inspection shows subsystem 3); terminal `run` blocks the shell like v1.7.5 — [observable: PE header + HITL]
- [ ] Zero windows at login moves to phase 4's `conhost --headless` artifact; P3 verifies the hosting works (daemon alive under `conhost --headless`, no window process with a visible MainWindowHandle) — [command]
- [ ] From a terminal: `run` prints its banner there and the shell waits on it; `stop` and `core on|off` print expected text and error objects — [HITL + command]
- [ ] Output-routing decision covered by unit tests for all three branches — [test: daemon-crate unit tests]
- [ ] Ctrl+C stops a terminal-run daemon cleanly; `stop` stops a headless one — [command + HITL]

---

## Phase 4: Silent app-side spawn & self-healing autostart

**User stories**: 6 (dev-proof autostart), 7 (self-heal wrong entry), 8 (no window flash from the app).

### What to build

Add the no-window/no-console creation flags to the app-side daemon spawn, so
enabling Storestation (or Core) from Settings flashes nothing. Make autostart
installation refuse in dev builds with a clear message (same spirit as the
existing dev skip for single-instance). At release startup, verify the
platform autostart artifact resolves to the installed daemon and rewrite it
when it does not — silent, logged self-heal that repairs already-poisoned
machines (including the PO's Run key pointing into the dev tree) on first
launch of the fixed build.

### Assumptions carried in

- Phase 3's console-subsystem daemon binary is what the (re)written artifact
  points at — wrapped in `conhost --headless` on Windows (P3 verified the
  hosting works windowlessly).
- Autostart mechanisms stay: Run key on Windows, LaunchAgent on macOS, systemd
  user unit on Linux.

### Out of scope for this phase

- New autostart mechanisms (no scheduled tasks, no services).
- UI for the self-heal (it is silent; Settings already shows daemon status).

### Acceptance criteria

- [ ] Storestation OFF→ON toggled 5× in Settings → zero window flashes — [HITL: count = 0]
- [ ] Dev build + autostart toggle → clear refusal message; the OS artifact is unchanged — [HITL once + test: dev-guard unit test]
- [ ] Pre-seeded wrong Run key → after one release launch the key resolves to the installed daemon path — [observable: registry query artifact]
- [ ] macOS/Linux autostart generators unchanged and green — [test: existing generator unit tests pass on all platforms]

---

## Phase 5: CLI trust repair & the Storestation full-restore walk

**User stories**: 9 (CLI version + attach trustworthy), 10 (full session restore on Windows).

### What to build

Close out the bug package. Machine-level: remove the stale cargo-installed CLI
copy on the PO's machine (documented one-liner in the HITL steps) so PATH
resolves to the bundled CLI. Product-level: when `attach` cannot find a
usable app or daemon, its failure output names the lookup command so any user
can discover a shadowing copy. Then run the definitive Storestation HITL walk
on Windows: daemon enabled → work in terminals → close every umux window →
reopen → everything comes back.

### Assumptions carried in

- Phases 1–4 landed: no crash, gate self-heals, no windows, autostart repaired.
- The bundled CLI already supports `attach` and `--version` (it reports its
  real version).

### Out of scope for this phase

- Installer PATH management (rejected in the PRD).
- v1.8.0 live-CLI socket parity.

### Acceptance criteria

- [ ] On the PO's machine `where umux` resolves to the bundled copy and `umux --version` equals the installed version — [command]
- [ ] `umux attach` with the daemon running focuses or starts the app — [command + HITL]
- [ ] `attach` failure output (daemon or app unresolvable) includes the lookup hint — [test: CLI output test]
- [ ] Full restore: 2 workspaces × 2 tabs with live shells → close all windows → reopen → all four panels return with their sessions — [HITL script]

---

## Phase 6: Overlay scrollbars

**User stories**: 11 (bars appear only while scrolling, everywhere except the terminal).

### What to build

One shared module: scrollbar styles (thin, rounded, transparent track) plus a
scroll-flash behavior (thumb visible only while the region is scrolling,
fading out shortly after; also shown on region hover). Apply it to the sidebar
list, the Settings card, the tab bar's horizontal scroll, the cmux import
preview, and dialogs. Follow the existing frontend test pattern of asserting
the style guarantees (jsdom cannot measure scrollbars). The terminal pane's
own scrollbar is explicitly untouched.

### Assumptions carried in

- WebView2 (Chromium) honors the custom scrollbar styling.
- Bug phases are done; this starts the feature package (PO's ship order).

### Out of scope for this phase

- Restyling the xterm/terminal scrollbar.
- Any layout redesign of the affected regions.

### Acceptance criteria

- [ ] Style guarantees asserted in frontend tests (margins, overflow, scrollbar styling) — [test: vitest, same pattern as the existing viewport assertions]
- [ ] HITL: scrolling each of the five regions shows the bar only during scrolling and it fades after; hovering shows it; the terminal scrollbar behaves exactly as before — [HITL script]

---

## Phase 7: Tab multi-select

**User stories**: 12 (Ctrl/Shift+click selection), 13 (close selected), 14 (drag group reorder), 15 (move to another workspace).

### What to build

A pure, unit-testable selection reducer over the tab list: toggle one
(Ctrl+click), range select (Shift+click), clear (Esc or plain click), group
reorder preserving relative order, and move-to-workspace. Wire the UI: the
context menu gains "close selected" and "move to workspace ▸" when ≥ 2 tabs
are selected; dragging a selection moves the whole group while keeping its
internal order. Per-tab working directory and live session state survive
moves exactly as a single-tab move does today.

### Assumptions carried in

- The existing tab drag infrastructure can carry a multi-item payload (or is
  extended minimally to do so).
- Single-tab move semantics (cwd + session preservation) already work.

### Out of scope for this phase

- Cross-workspace selection (selection lives within one workspace's tab bar).
- Select-all shortcuts; spring/accent work (v2.1.0 scope).

### Acceptance criteria

- [ ] Reducer unit tests: toggle, range, clear, group reorder preserving relative order, move between workspaces — [test: vitest]
- [ ] HITL: Ctrl+click three tabs; Shift+click extends the range; Esc clears; dragging the group reorders it with internal order kept — [HITL script]
- [ ] HITL: "move to workspace ▸" lands all selected tabs in the target with sessions/cwd intact; "close selected" closes exactly the selection — [HITL script]

---

## Phase 8: Version label in Settings

**User stories**: 16 (app version visible next to the update status).

### What to build

Read the app version through the app's own version API and render it beside
the updater status row — in both the "up to date" and "update available"
states (e.g. "umux is up to date (1.7.6)").

### Assumptions carried in

- The updater row already renders both states; this only adds the version.

### Out of scope for this phase

- Changelog display, update channel UI, or any updater behavior change.

### Acceptance criteria

- [ ] Component test: the row renders the provided version string in both states — [test: vitest]
- [ ] HITL: Settings shows the installed version next to the update status — [observable]
