# Plan: umux v1.7.5 — umux Core (Always-On device)

> Source PRD: [`umux-v1.7.5-prd.md`](./umux-v1.7.5-prd.md) — stories #141–#143, discovery 2026-09-20.
> Slot: after v1.7.0 (shipped 2026-09-30), before v1.8.0. HITL order: **macOS + Windows first, Ubuntu after**.

## Architectural decisions

Durable decisions that apply across all phases:

- **Holder = the daemon.** umux Storestation (`umux-storestation`) is the single holder of the
  sleep block. The Core flag is **daemon-owned state, persisted by the daemon and re-asserted on
  daemon start** — the app must never be the component that re-applies it, because the whole point
  is "every window closed". Daemon stop (graceful or crash) releases the block by construction
  (the assertion lives inside the daemon process / its file descriptors).
- **Protocol, not a fork.** Core rides the EXISTING Storestation socket protocol: one new
  request op (set Core on/off) + new fields on the status response. No second socket, no second
  daemon, no new pid file.
- **Best-effort per platform, never silent.** Each platform backend answers with
  `held: yes/no` plus, when the OS refuses or limits the guarantee, a per-platform
  **instruction string** (Windows/Linux lid-close action, macOS on battery). The instruction
  surfaces in BOTH faces: the Core section in Settings and `umux status --json`.
- **Platform backends (all no-admin, per PO decision Q14):**
  - macOS: caffeinate-style system power assertion (no admin; reliable on AC, battery may still
    force sleep — documented, shown in status).
  - Windows: `SetThreadExecutionState` (system-required, continuous) on a daemon thread — stops
    idle sleep without admin; the lid-close action stays a power-plan setting → instructions.
  - Linux: session-level idle/sleep inhibit (logind inhibitor or the desktop session's
    equivalent); the lid switch belongs to logind (root) → instructions.
- **Data model.** Daemon-side persisted record: `core: { enabled: bool, held: bool,
  instruction?: string }`. App-side: nothing new — the Settings switch is a client that sends the
  op and mirrors the status (same pessimistic persist-after-success flow as the Storestation
  toggles). `umux status --json` gains `sleepPrevented` (+ the instruction when not held).
- **Screen may sleep.** Only the system/machine is held. No display assertion anywhere.
- **Trigger = the manual switch only.** No session-based logic, no activity heuristics (PO
  decision, rejected alternative in the PRD).
- **Naming.** "umux Core" is THIS feature (Settings section "Core"); the daemon stays
  "umux Storestation" (section "Storestation"). Two sections, one daemon.

---

## Phase 1: Core on macOS — daemon state, assertion, status (tracer bullet)

**User stories**: #142 (macOS part), #143

### What to build

The full vertical path on macOS with NO UI: the daemon persists a Core flag, re-asserts it on
start, gains the set-Core op on the socket, holds a caffeinate-style system power assertion while
Core is ON, and reports the truth in the status response — which `umux status --json` surfaces as
`sleepPrevented` (+ the battery instruction when the guarantee is limited). Demoable entirely from
the terminal: run the daemon, flip Core with a client call, watch `pmset -g assertions`.

### Assumptions carried in

- v1.7.0 daemon skeleton stands: socket prepare/single-instance, op dispatch, persistence dir,
  graceful shutdown (story 110) — Core slots into that op table, no protocol redesign.
- `umux status --json` already reads the daemon's status response; it only learns new fields.

### Out of scope for this phase

- No Settings UI (Phase 2), no Windows/Linux backends (Phases 3–4).
- No autostart interaction changes — Core resumes from daemon-persisted state on ANY daemon start.
- No notifications (decided in discovery).

### Acceptance criteria

- [ ] Core ON → `pmset -g assertions` shows the umux system-sleep assertion; toggle OFF or daemon
  stop → assertion gone — [command: `pmset -g assertions | grep -i umux` flips with the toggle]
- [ ] Core ON survives a daemon restart: stop the daemon, start it again → assertion is back
  without any client call — [observable: `pmset -g assertions` after restart]
- [ ] The set op on a second daemon / a stopped daemon fails with the existing conflict/error
  codes, not a hang — [test: storestation integration test]
- [ ] `umux status --json` reports `sleepPrevented: true/false` in sync with the assertion, and
  the battery instruction string on macOS when relevant — [command: `umux status --json`]

---

## Phase 2: Settings — the Core section and its switch

**User stories**: #141, #143

### What to build

The user-facing face: a new "Core" section in Settings, in the established full-page pattern —
one full-width plain-text entry button ("Core (Always-On device)") that opens its own screen with
the switch, the daemon status line and (when present) the platform instruction. The switch uses
the SAME pessimistic flow as the Storestation toggles: toggle ON ensures the daemon is running
(spawn if absent — works with every window closed), the op must succeed before the setting
persists; toggle OFF releases the block. Screen keeps its own sub-view navigation (Back/Escape),
footnote stays last.

### Assumptions carried in

- Phase 1's daemon op + status fields exist and answer truthfully.
- The Settings sub-view pattern (entry button → dedicated screen → Back/Escape) ships in this
  same tree — Core reuses it verbatim, including the always-last footnote.

### Out of scope for this phase

- No Windows/Linux backends — the switch already works on macOS; other platforms show the
  honest status/instruction their backend reports (added in Phases 3–4).
- No new autostart coupling: Core does not force the daemon autostart switch (PO scope cut).

### Acceptance criteria

- [ ] The Core switch is OFF by default and mirrors the daemon-reported state after the dialog
  opens — [test: SettingsDialog component test]
- [ ] Toggling ON with no daemon running spawns it, the op succeeds, the switch persists —
  the assertion exists with every umux window closed — [observable: `pmset -g assertions`]
- [ ] Toggling OFF releases the block; a failed op (daemon unreachable) leaves the persisted
  state untouched and shows the error — [test: component test with a failing op]
- [ ] The instruction string from the daemon renders in the Core screen when present, and the
  screen hides nothing silently — [test: component test]

---

## Phase 3: Windows — SetThreadExecutionState backend

**User stories**: #142 (Windows part)

### What to build

The same daemon op on Windows: a dedicated daemon thread holding
`SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` while Core is ON (no admin, screen
may sleep). The lid-close action is a power-plan setting the daemon cannot touch — the backend
reports the honest state plus the lid instruction string, which flows to Settings and
`umux status --json`. Cross-compilation must stay green (the known cfg-trap: local macOS
`cargo check` does not build the Windows-gated code — CI does).

### Assumptions carried in

- Phase 1's Core state machine and status fields are platform-agnostic; only the assertion
  backend is cfg-gated.
- The v1.6.2 cfg-trap lesson: Windows-gated code is validated by CI builds, not local cargo check.

### Out of scope for this phase

- No power-plan changes, no admin elevation (PO decision Q14).
- No Ubuntu backend yet (Phase 4).

### Acceptance criteria

- [ ] Windows build compiles in CI with the new cfg(windows) backend — [observable: CI run green]
- [ ] Core ON keeps the machine awake through the idle timeout with the lid open; OFF releases —
  [observable: Windows machine, HITL]
- [ ] The lid-close instruction renders in Settings and `umux status --json` — [observable: HITL]
- [ ] The assertion thread releases on daemon stop (no orphaned execution state) — [observable:
  HITL, `powercfg /requests` shows no umux hold after stop]

---

## Phase 4: Ubuntu — session idle-sleep inhibit backend

**User stories**: #142 (Linux part)

### What to build

The Linux backend: a session-level inhibit against idle-suspend while Core is ON (logind
inhibitor taken by the daemon within the user session, or the desktop session's inhibit
equivalent — whichever the reference GNOME/Wayland setup honors). The lid switch belongs to
logind/root: the backend reports the lid instruction string instead of touching it, and the
block's lifetime stays tied to the daemon (the inhibitor fd dies with the process).

### Assumptions carried in

- Phase 1's op/status surface unchanged; this is the third cfg-gated backend.
- Reference environment: Ubuntu/Wayland + GNOME (the project's reference platform), Adam's
  machine is the HITL gate.

### Out of scope for this phase

- No lid handling, no logind config changes, no root (PO decision Q14).
- No X11-specific paths (X11 sessions are untested project-wide).

### Acceptance criteria

- [ ] Core ON inhibits GNOME's idle suspend; OFF releases — [observable: HITL on Ubuntu/Wayland,
  `systemd-inhibit --list` / GNOME settings behavior]
- [ ] The inhibitor dies with the daemon (kill -9 included) — [observable: HITL]
- [ ] The lid instruction renders in Settings and `umux status --json` — [observable: HITL]

---

## Phase 5: Consistency pass + full HITL validation

**User stories**: #141–#143 (closure)

### What to build

The wrap-up slice: every platform's status/instruction shape checked against one schema
(`sleepPrevented` + instruction, same field names everywhere), the PRD's validation list walked
end-to-end on real machines (macOS AC + battery, Windows, Ubuntu), and the status JSON contract
documented where the existing `umux status` output contract lives. No new features.

### Assumptions carried in

- Phases 1–4 are merged; only verification and shape-fixing remain.

### Out of scope for this phase

- No remote wake / schedules / battery intelligence (PRD out-of-scope list).
- No landing/README copy changes unless the validation pass exposes a wrong claim.

### Acceptance criteria

- [ ] The PRD validation list is checked off item by item on the real platforms —
  [observable: the checklist in `umux-v1.7.5-prd.md`]
- [ ] `umux status --json` field names are identical across the three platforms —
  [command: run on each platform and diff the Core-related keys]
- [ ] Full `npm test` + `cargo check` green on the tree — [command: `npm test && cargo check`]
