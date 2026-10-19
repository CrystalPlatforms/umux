# Plan: umux Core Session (timed awake sessions)

> Source PRD: [`umux-core-session-prd.md`](./umux-core-session-prd.md) (proposal #108 + PO decisions 2026-10-19); master PRD [`umux-prd.md`](./umux-prd.md) wins on conflict. Stories #144–#148. Version slot: open (PO's call).

## Architectural decisions

Durable decisions that apply across all phases:

- **Holder**: the daemon (umux Storestation) is the single holder of the session, exactly as for Core — the session is a new state domain inside the daemon's Core state machine (`CoreState` grows a session), NOT a separate service. One lock (`state.core`), existing ops table gains `core.session.*` ops.
- **Data model**: the flag document `storestation.core.json` grows additively:
  `{"enabled": bool, "session": {"startedAt": <unix s>, "endsAt": <unix s>} | null}` plus a non-persisted ended record surfaced in status (`lastEndedAt`, `lastEndReason`). Old daemons read only `enabled`; corrupt files still read as OFF.
- **Deadline semantics**: absolute `endsAt` (unix seconds), never remaining time. Restore: `endsAt − now ≤ 0` → expired, no assertion, recorded. Expiry drops the existing `PowerAssertion` value — the crash-release guarantee is inherited by construction (kernel/fd/thread lifetime), never re-implemented.
- **Refusal matrix (one gate at the daemon)**: `core.session.start` refused while `enabled: true` (all platforms — story 148); `core.set(true)` refused on macOS (story 147) with an enumerated code naming `core-session`; `core.set(false)` allowed everywhere.
- **Cap**: total session span ≤ 24 h from `startedAt`; `extend` clamps/refuses at the cap with an enumerated error.
- **Key entities / names**: UI "Core Session"; code + CLI subcommand `core-session`; status object `core.session` with fields `active`, `endsAt`, `remainingSecs`, `lastEndedAt`, `lastEndReason` (`"expired" | "stopped" | null`). Field names byte-identical on all platforms.
- **CLI grammar**: `umux core-session start <n>{s|m|h}` / `extend <n>{s|m|h}` / `stop`; `--json` mirrors the status fields. `umux core on|off` keeps its name; on macOS `core on` surfaces the daemon refusal.
- **Notification**: expiry only (never start/extend), emitted daemon-side so it fires with every window closed; smallest per-platform path, no new heavy dependencies; visual family of the app's update notification, bottom-right corner.
- **Integrations**: none new. The three platform backends (#93–#96) and the Linux notification behavior are untouched. Settings talks to the same daemon ops as the CLI (single source of truth).
- **Standing rule**: every new interactive element ships with spring press feedback (#101).

---

## Phase 1: Core Session core — daemon ops, expiry timer, status, CLI (tracer bullet, macOS)

**User stories**: 144, 145 (backend half), 146, 148 (refusal half)

### What to build

The full vertical path with NO UI, demoable from the terminal on macOS: the daemon accepts `core.session.start/extend/stop`, holds the existing system-sleep assertion for the window, auto-releases at `endsAt` (timer tick), records the ended state, and reports everything in the status response — which `umux status --json` surfaces inside `core.session` (additive; `sleepPrevented` semantics unchanged: held = Core ON **or** session active). The CLI learns `umux core-session start/extend/stop` (durations `<n>{s|m|h}`, `--json` mirror). Persistence is additive in the flag file; daemon-start restore re-asserts a live session with the remaining window and treats a past deadline as expired — never an infinite hold. Refusals: `core.session.start` while Core ON (any platform), extend past the 24 h span from `startedAt`.

### Assumptions carried in

- v1.7.5 Core stands (`CoreState`, flag persistence, `core.set` op, status `core` object, contract tests in `src-tauri/cli/tests/core_lifecycle.rs`). The session slots beside it; no backend changes.
- The daemon serve loop already wakes on a regular tick; the expiry check rides that (or a dedicated thread locking the same mutex) — no scheduler infrastructure.

### Out of scope for this phase

- No Settings UI (Phase 4), no macOS Core retirement gate/migration (Phase 3), no Windows/Linux hardware walkthrough (Phase 2).
- No expiry notification yet (Phase 4 wires delivery; this phase only records `lastEndReason: "expired"`).
- No notifications on start/extend (never, per PRD).

### Acceptance criteria

- [ ] `umux core-session start 90m` → assertion held (`pmset -g assertions` shows umux); `umux status --json` shows `core.session.active=true`, `endsAt` ≈ now+90m, `remainingSecs` counting down — [command: `pmset -g assertions`; command: `umux status --json`]
- [ ] `extend 30m` moves `endsAt` +30m; an extend that would push the span past 24 h from `startedAt` is refused with an enumerated error and changes nothing — [test: storestation tests]
- [ ] `stop` releases the assertion immediately and records `lastEndReason: "stopped"`, `lastEndedAt` — [command: `pmset -g assertions`; test: storestation tests]
- [ ] Expiry auto-releases with no client call: assertion gone, `active=false`, `lastEndReason: "expired"` — [test: storestation timer test with a short TTL]
- [ ] Daemon restart mid-session re-asserts with the remaining window; restart after the deadline asserts nothing and records the expiry — [observable: `pmset -g assertions` after restart; test: restore paths]
- [ ] `core-session start` while Core is ON is refused with an enumerated code; state unchanged — [test: storestation tests]
- [ ] Flag compatibility both ways: a new-format file read by the v1.7.5-shaped reader yields `enabled` only; an old file read by the new reader yields "no session"; a corrupt file still reads OFF — [test: storestation tests]
- [ ] `umux status --json` exposes `core.session` with exactly the pinned field names; the `core` object's `enabled`/`held`/`instruction` keys unchanged — [test: `src-tauri/cli/tests/core_lifecycle.rs` contract]

---

## Phase 2: Windows + Linux parity — same state machine, per-platform hold verification

**User stories**: 144, 145 (backend half), 146, 148

### What to build

The Phase 1 session machine verified live on the other two platforms — no new state logic: on Windows the session's hold appears as the Core execution-state request (`powercfg /requests`), on Linux as the logind inhibitor fd; expiry and early stop release them. The refusal matrix, cap, restore semantics and status shape are asserted identical across platforms (the state machine is shared code; this phase proves it). Platform gaps (e.g. a backend refusing mid-session) surface as the honest instruction string, never silence.

### Assumptions carried in

- Phase 1's session machine, ops, CLI and status shape stand unchanged.
- The three assertion backends (#93–#96) are untouched; the session only owns *when* they are taken and dropped.

### Out of scope for this phase

- No Settings UI, no macOS gate/migration.
- No notification delivery (Phase 4).

### Acceptance criteria

- [ ] Windows: `core-session start` → `powercfg /requests` shows the umux request; expiry releases it; `active=false`, `lastEndReason: "expired"` — [command: `powercfg /requests`; test: storestation tests (Windows-gated)]
- [ ] Linux: `core-session start` → logind inhibitor held (visible via `loginctl`/`systemd-inhibit --list`); expiry releases the fd — [command: `systemd-inhibit --list`; test: storestation tests (Linux-gated)]
- [ ] Refusal matrix + 24 h cap + restore semantics behave identically on all three platforms; the daemon status `core.session` shape is byte-identical (same field names) — [test: storestation tests; command: `umux status --json` diffed across platforms]
- [ ] Existing Core lifecycle tests on Windows/Linux still pass untouched — [test: `src-tauri/cli/tests/core_lifecycle.rs`]

---

## Phase 3: macOS Core retirement — daemon gate, flag migration, CLI refusal

**User stories**: 147

### What to build

macOS stops offering persistent Core, enforced at the single gate: the daemon's `core.set(true)` refuses on macOS with an enumerated code whose message names `core-session start`; `core.set(false)` stays allowed (releases any stale hold). The migration: on macOS daemon start, a legacy `enabled: true` flag is persisted back as `false` — no assertion — and the honest channel (`core.instruction`) explains that Core was replaced by Core Session (beta). The CLI: `umux core on` on macOS surfaces the daemon refusal (with the pointer); `umux core off` succeeds. Windows/Linux `core on|off` are untouched. The `core` status object keeps its contract keys everywhere.

### Assumptions carried in

- Phase 1's session machine stands — the refusal's pointer (`core-session start`) resolves to a working command.
- The flag document is additive-safe (Phase 1), so rewriting `enabled` does not disturb a live session record.

### Out of scope for this phase

- No Settings changes (Phase 4 owns the macOS section swap).
- No behavior change on Windows/Linux.

### Acceptance criteria

- [ ] macOS: daemon `core.set(true)` → enumerated refusal naming `core-session`; `core.set(false)` succeeds and releases — [test: storestation tests (macOS-gated)]
- [ ] macOS: `umux core on` CLI → refusal with the `core-session start` hint; `umux core off` succeeds — [command: `umux core on` on macOS]
- [ ] macOS: daemon start with legacy `enabled: true` → flag persisted as `false`, no assertion held, status `core.instruction` explains the Core Session (beta) replacement — [test: restore path; command: `umux status --json`]
- [ ] Windows/Linux: `core on|off` behavior unchanged; all existing Core lifecycle tests pass — [test: `src-tauri/cli/tests/core_lifecycle.rs`]
- [ ] The `core` object in `umux status --json` keeps `enabled`/`held`/`instruction` on every platform — [test: CLI contract test]

---

## Phase 4: Settings — Core Session section (BETA on macOS), countdown, end state, expiry notification

**User stories**: 145 (UX half), 147 (Settings half)

### What to build

Settings' Core section becomes platform-aware. On macOS the Core switch is gone; the section is **Core Session**: a switch (BETA badge) that starts a session (default length offered in the dialog), an Insomnia info line ("inspirowane projektem Insomnia — tylko technologia, bez menu baru"), a live countdown from `endsAt`, and the visible end state („sesja zakończona o HH:MM" from `lastEndedAt`) after expiry/stop. On Windows/Linux the existing Core switch stays and session controls (start/extend/stop) appear beside it with the same countdown/end state. Expiry fires a desktop notification emitted by the daemon (bottom-right corner, the update-notification family) — including with every umux window closed. All controls drive the same daemon ops as the CLI; every new interactive element carries spring press feedback (#101).

### Assumptions carried in

- Phases 1–3 stand: ops, status shape, macOS gate and migration are all live — the UI only renders and drives them.
- Settings already mirrors daemon status for Core (v1.7.5 pattern: the section mirrors, never owns the state).

### Out of scope for this phase

- No new daemon state, no schema changes.
- No menu bar, no schedules, no per-session notification preferences.

### Acceptance criteria

- [ ] macOS: Settings shows no Core switch; the Core Session switch carries a BETA badge and the Insomnia info line — [test: `src/SettingsDialog.test.tsx`; observable: Settings]
- [ ] The countdown renders from `core.session.endsAt` and ticks; after expiry/stop the section shows the ended state with the time — [test: SettingsDialog.test.tsx]
- [ ] Expiry with every window closed still produces a desktop notification in the update-notification family, bottom-right — [HITL walkthrough; automated part: the daemon emits the expiry notification — test: storestation tests]
- [ ] Start/extend/stop from Settings hit the same daemon ops as the CLI (one source of truth) and reflect refusals (Core ON, cap) as visible errors, never silence — [test: SettingsDialog.test.tsx]
- [ ] Every new interactive element has spring press feedback (#101) — [observable: Settings]
- [ ] `npm test` green; the landing/README copy untouched by this phase — [command: `npm test`]

---

## HITL validation

Walked by Adam on real hardware after Phase 4, per the PRD's validation list (macOS → Windows → Ubuntu order, same as v1.7.5).
