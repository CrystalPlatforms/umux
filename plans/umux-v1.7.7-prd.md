# umux v1.7.7 — (COMPLETED)

> Proposal [#108](https://github.com/CrystalPlatforms/umux/issues/108) + PO decisions 2026-10-19 (recorded [on #108](https://github.com/CrystalPlatforms/umux/issues/108#issuecomment-6260586732)); version **v1.7.7** assigned by the PO (2026-10-26). Extract of the master PRD ([`umux-prd.md`](./umux-prd.md) wins on conflict); stories **#144–#148**.
> Builds entirely on v1.7.5 umux Core ([`umux-v1.7.5-prd.md`](./umux-v1.7.5-prd.md), issues #93–#97, shipped). Reference project (design only, **zero code reuse**): [krishhgg/Insomnia](https://github.com/krishhgg/Insomnia) (MIT) — a macOS menu-bar app for timed awake sessions. umux takes ONLY the time dimension; **no menu bar** (PO decision).
> Merged document (2026-10-26): the former standalone `umux-core-session-prd.md` (what & why) and `umux-core-session-plan.md` (phased plan) were consolidated into this one file.

**Status:** in implementation — Phase 1 (daemon ops, expiry, status, CLI — #111 #112) and the macOS Core retirement + Settings slice (Phases 3–4 — #113 #114) are merged to `main`; Phase 2 (Windows/Linux parity verification) and the full HITL walk remain.

## What it is

A **timed awake session** on top of the v1.7.5 Core state machine: `umux core-session start <czas>` holds the OS power assertion (the same three backends as Core — macOS IOKit / Windows `SetThreadExecutionState` / Linux logind inhibitor, **unchanged**) for a bounded window of max **24 h**. `extend` adds time mid-session; `stop` ends it early; **expiry auto-releases** the hold with no client call and fires a desktop notification. Status exposes the session; Settings shows the countdown and the end state.

On **macOS**, Core's persistent switch is **retired**: the Core section in Settings becomes the **Core Session** switch with a **BETA** badge and an info line about the Insomnia inspiration. `umux core on` is refused at the daemon level with a pointer to `core-session start`. On **Windows/Linux**, Core stays exactly as shipped and a session start is refused while Core is ON.

## Why

v1.7.5 Core is a persistent ON/OFF toggle: the machine stays awake until someone remembers to flip it off. Insomnia's genuinely new idea is the **time dimension** — a long-running agent session gets an awake window with an end time. The hard parts are already done (three native backends, one status schema, honest instruction strings); a timed session is a daemon-owned deadline on top, not a new backend.

## User stories (master PRD numbering)

- **144.** As a user, I want to start a timed awake session (`umux core-session start <czas>`) so the machine stays awake for a bounded window (max 24 h per session) instead of indefinitely, with nothing to remember to switch off.
- **145.** As a user, I want to extend or stop a session mid-flight (`core-session extend` / `stop`) and have expiry auto-release the hold with a desktop notification (same family as the update notification) and a visible end state in Settings, so the machine never stays awake longer than I asked.
- **146.** As a developer, I want the session to be daemon-owned state persisted with an absolute end time and restored after a daemon restart with the remaining window (never silently converting into an infinite hold), so crashes and restarts can neither extend nor fake the hold.
- **147.** As a user on macOS, I want Core's persistent switch replaced by Core Session (BETA) with an Insomnia info line, my old Core ON flag cleaned at update, and `umux core on` refused at the daemon with a pointer to `core-session start`, so there is exactly one honest way to keep the machine awake there.
- **148.** As a user on Windows and Linux, I want Core unchanged and a session start refused while Core is ON with a clear message, so the two holds never fight or stack silently.

## Decisions (PO 2026-10-19, signed off — recorded on #108)

| #   | Temat            | Decyzja                                                                                                                                                                                                                                                                                                       |
| --- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Limit sesji      | **24 h** maks. na jedną sesję                                                                                                                                                                                                                                                                                 |
| 2   | Wygaśnięcie      | Widoczny stan w Settings („sesja zakończona o HH:MM") **+ powiadomienie desktopowe** w prawym dolnym rogu — ten sam mechanizm/styl co powiadomienie o nowej aktualizacji. **Nadpisuje cięcie „no notifications" z v1.7.5** — świadoma decyzja PO, tylko dla wygaśnięcia sesji (start/extend bez powiadomień). |
| 3   | Nazwa            | **Core Session** — w UI „Core Session", w kodzie i CLI podkomenda **`core-session`** (z myślnikiem): `umux core-session start <czas>` / `extend` / `stop`                                                                                                                                                     |
| 4   | Źródło pomysłu   | Tylko technologia/koncepcja Insomnii — zero kopiowania kodu, **żadnego menu baru**                                                                                                                                                                                                                            |
| 5   | macOS — Settings | Przełącznik Core **znika**; w jego miejscu przełącznik Core Session z badge **BETA** + info o Insomnii w sekcji                                                                                                                                                                                               |
| 6   | macOS — CLI      | `umux core on` na macOS → odmowa z podpowiedzią (`umux core-session start`); `core off` pozostaje do obsługi starego stanu                                                                                                                                                                                    |
| 7   | macOS — migracja | Flaga Core ON **czyszczona przy aktualizacji** + jasne info w statusie/Settings, że Core został zastąpiony przez Core Session (beta)                                                                                                                                                                          |
| 8   | Windows/Linux    | Core bez zmian; `core-session start` przy Core ON → **odmowa z komunikatem** (Core już trzyma maszynę obudzoną bez limitu)                                                                                                                                                                                    |

### Technical pins (from the same decision session)

- **Absolute deadline, not remaining time**: the persisted state is `endsAt` (unix seconds). Restore computes `endsAt − now`; a deadline in the past after a restart means _expired_ — no assertion, recorded in status.
- **Additive flag shape**: `storestation.core.json` gains `"session": {...}` alongside `"enabled"`; an old daemon reads only `enabled` (ignores the rest), a new daemon reads an old file as "no session". Corrupt file still reads as OFF (unchanged v1.7.5 rule).
- **One gate**: the macOS refusal of `core on` lives at the daemon (`core.set` op), not only in the CLI — no client can bypass it.
- **Cap = total session span**: `extend` cannot push `endsAt` beyond 24 h from session start.
- **CLI grammar**: durations as `<n><s|m|h>` (e.g. `90m`, `2h`); `--json` mirrors the status fields, consistent with the existing CLI discipline.
- **Status shape (additive)**: inside the existing `core` object of `umux status --json`: `session: {active, endsAt, remainingSecs, lastEndedAt, lastEndReason}`. `endsAt`/`lastEndedAt` are unix seconds; `lastEndReason` is `"expired" | "stopped" | null`. `sleepPrevented` keeps its meaning (held = Core ON or session active); the `core` object's contract keys (`enabled`, `held`, `instruction`) never change.
- **Expiry notification is daemon-side** (a session must outlive every window — that is the point of Core): the smallest per-platform path, no new heavy dependencies; visual family of the app's update notification.
- **Timer**: daemon-side; expiry drops the existing `PowerAssertion` value (inheriting the crash-release guarantee by construction) and records `lastEndReason: "expired"`. The platform backends (#93–#96) are untouched.

## Architectural decisions (implementation)

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

## Implementation plan

### Phase 1: Core Session core — daemon ops, expiry timer, status, CLI (tracer bullet, macOS)

**User stories**: 144, 145 (backend half), 146, 148 (refusal half)

#### What to build

The full vertical path with NO UI, demoable from the terminal on macOS: the daemon accepts `core.session.start/extend/stop`, holds the existing system-sleep assertion for the window, auto-releases at `endsAt` (timer tick), records the ended state, and reports everything in the status response — which `umux status --json` surfaces inside `core.session` (additive; `sleepPrevented` semantics unchanged: held = Core ON **or** session active). The CLI learns `umux core-session start/extend/stop` (durations `<n>{s|m|h}`, `--json` mirror). Persistence is additive in the flag file; daemon-start restore re-asserts a live session with the remaining window and treats a past deadline as expired — never an infinite hold. Refusals: `core.session.start` while Core ON (any platform), extend past the 24 h span from `startedAt`.

#### Assumptions carried in

- v1.7.5 Core stands (`CoreState`, flag persistence, `core.set` op, status `core` object, contract tests in `src-tauri/cli/tests/core_lifecycle.rs`). The session slots beside it; no backend changes.
- The daemon serve loop already wakes on a regular tick; the expiry check rides that (or a dedicated thread locking the same mutex) — no scheduler infrastructure.

#### Out of scope for this phase

- No Settings UI (Phase 4), no macOS Core retirement gate/migration (Phase 3), no Windows/Linux hardware walkthrough (Phase 2).
- No expiry notification yet (Phase 4 wires delivery; this phase only records `lastEndReason: "expired"`).
- No notifications on start/extend (never, per PRD).

#### Acceptance criteria

- [ ] `umux core-session start 90m` → assertion held (`pmset -g assertions` shows umux); `umux status --json` shows `core.session.active=true`, `endsAt` ≈ now+90m, `remainingSecs` counting down — [command: `pmset -g assertions`; command: `umux status --json`]
- [ ] `extend 30m` moves `endsAt` +30m; an extend that would push the span past 24 h from `startedAt` is refused with an enumerated error and changes nothing — [test: storestation tests]
- [ ] `stop` releases the assertion immediately and records `lastEndReason: "stopped"`, `lastEndedAt` — [command: `pmset -g assertions`; test: storestation tests]
- [ ] Expiry auto-releases with no client call: assertion gone, `active=false`, `lastEndReason: "expired"` — [test: storestation timer test with a short TTL]
- [ ] Daemon restart mid-session re-asserts with the remaining window; restart after the deadline asserts nothing and records the expiry — [observable: `pmset -g assertions` after restart; test: restore paths]
- [ ] `core-session start` while Core is ON is refused with an enumerated code; state unchanged — [test: storestation tests]
- [ ] Flag compatibility both ways: a new-format file read by the v1.7.5-shaped reader yields `enabled` only; an old file read by the new reader yields "no session"; a corrupt file still reads OFF — [test: storestation tests]
- [ ] `umux status --json` exposes `core.session` with exactly the pinned field names; the `core` object's `enabled`/`held`/`instruction` keys unchanged — [test: `src-tauri/cli/tests/core_lifecycle.rs` contract]

### Phase 2: Windows + Linux parity — same state machine, per-platform hold verification

**User stories**: 144, 145 (backend half), 146, 148

#### What to build

The Phase 1 session machine verified live on the other two platforms — no new state logic: on Windows the session's hold appears as the Core execution-state request (`powercfg /requests`), on Linux as the logind inhibitor fd; expiry and early stop release them. The refusal matrix, cap, restore semantics and status shape are asserted identical across platforms (the state machine is shared code; this phase proves it). Platform gaps (e.g. a backend refusing mid-session) surface as the honest instruction string, never silence.

#### Assumptions carried in

- Phase 1's session machine, ops, CLI and status shape stand unchanged.
- The three assertion backends (#93–#96) are untouched; the session only owns _when_ they are taken and dropped.

#### Out of scope for this phase

- No Settings UI, no macOS gate/migration.
- No notification delivery (Phase 4).

#### Acceptance criteria

- [ ] Windows: `core-session start` → `powercfg /requests` shows the umux request; expiry releases it; `active=false`, `lastEndReason: "expired"` — [command: `powercfg /requests`; test: storestation tests (Windows-gated)]
- [ ] Linux: `core-session start` → logind inhibitor held (visible via `loginctl`/`systemd-inhibit --list`); expiry releases the fd — [command: `systemd-inhibit --list`; test: storestation tests (Linux-gated)]
- [ ] Refusal matrix + 24 h cap + restore semantics behave identically on all three platforms; the daemon status `core.session` shape is byte-identical (same field names) — [test: storestation tests; command: `umux status --json` diffed across platforms]
- [ ] Existing Core lifecycle tests on Windows/Linux still pass untouched — [test: `src-tauri/cli/tests/core_lifecycle.rs`]

### Phase 3: macOS Core retirement — daemon gate, flag migration, CLI refusal

**User stories**: 147

#### What to build

macOS stops offering persistent Core, enforced at the single gate: the daemon's `core.set(true)` refuses on macOS with an enumerated code whose message names `core-session start`; `core.set(false)` stays allowed (releases any stale hold). The migration: on macOS daemon start, a legacy `enabled: true` flag is persisted back as `false` — no assertion — and the honest channel (`core.instruction`) explains that Core was replaced by Core Session (beta). The CLI: `umux core on` on macOS surfaces the daemon refusal (with the pointer); `umux core off` succeeds. Windows/Linux `core on|off` are untouched. The `core` status object keeps its contract keys everywhere.

#### Assumptions carried in

- Phase 1's session machine stands — the refusal's pointer (`core-session start`) resolves to a working command.
- The flag document is additive-safe (Phase 1), so rewriting `enabled` does not disturb a live session record.

#### Out of scope for this phase

- No Settings changes (Phase 4 owns the macOS section swap).
- No behavior change on Windows/Linux.

#### Acceptance criteria

- [ ] macOS: daemon `core.set(true)` → enumerated refusal naming `core-session`; `core.set(false)` succeeds and releases — [test: storestation tests (macOS-gated)]
- [ ] macOS: `umux core on` CLI → refusal with the `core-session start` hint; `umux core off` succeeds — [command: `umux core on` on macOS]
- [ ] macOS: daemon start with legacy `enabled: true` → flag persisted as `false`, no assertion held, status `core.instruction` explains the Core Session (beta) replacement — [test: restore path; command: `umux status --json`]
- [ ] Windows/Linux: `core on|off` behavior unchanged; all existing Core lifecycle tests pass — [test: `src-tauri/cli/tests/core_lifecycle.rs`]
- [ ] The `core` object in `umux status --json` keeps `enabled`/`held`/`instruction` on every platform — [test: CLI contract test]

### Phase 4: Settings — Core Session section (BETA on macOS), countdown, end state, expiry notification

**User stories**: 145 (UX half), 147 (Settings half)

#### What to build

Settings' Core section becomes platform-aware. On macOS the Core switch is gone; the section is **Core Session**: a switch (BETA badge) that starts a session (default length offered in the dialog), an Insomnia info line ("inspirowane projektem Insomnia — tylko technologia, bez menu baru"), a live countdown from `endsAt`, and the visible end state („sesja zakończona o HH:MM" from `lastEndedAt`) after expiry/stop. On Windows/Linux the existing Core switch stays and session controls (start/extend/stop) appear beside it with the same countdown/end state. Expiry fires a desktop notification emitted by the daemon (bottom-right corner, the update-notification family) — including with every umux window closed. All controls drive the same daemon ops as the CLI; every new interactive element carries spring press feedback (#101).

#### Assumptions carried in

- Phases 1–3 stand: ops, status shape, macOS gate and migration are all live — the UI only renders and drives them.
- Settings already mirrors daemon status for Core (v1.7.5 pattern: the section mirrors, never owns the state).

#### Out of scope for this phase

- No new daemon state, no schema changes.
- No menu bar, no schedules, no per-session notification preferences.

#### Acceptance criteria

- [ ] macOS: Settings shows no Core switch; the Core Session switch carries a BETA badge and the Insomnia info line — [test: `src/SettingsDialog.test.tsx`; observable: Settings]
- [ ] The countdown renders from `core.session.endsAt` and ticks; after expiry/stop the section shows the ended state with the time — [test: SettingsDialog.test.tsx]
- [ ] Expiry with every window closed still produces a desktop notification in the update-notification family, bottom-right — [HITL walkthrough; automated part: the daemon emits the expiry notification — test: storestation tests]
- [ ] Start/extend/stop from Settings hit the same daemon ops as the CLI (one source of truth) and reflect refusals (Core ON, cap) as visible errors, never silence — [test: SettingsDialog.test.tsx]
- [ ] Every new interactive element has spring press feedback (#101) — [observable: Settings]
- [ ] `npm test` green; the landing/README copy untouched by this phase — [command: `npm test`]

---

## Out of scope

- Menu bar (Insomnia's UI form) — PO decision.
- Schedules / time-of-day windows, remote wake, wake-on-LAN, battery intelligence (master PRD out-of-scope list stands).
- Lid-close blocking (unchanged: instruction strings only).
- Notifications on session start or extend (only expiry notifies).
- Any code reuse from Insomnia itself — reference project only.
- Touching the v1.7.5 platform backends or the Linux notification behavior.

## Rejected alternatives

- **Persisting remaining time instead of an absolute `endsAt`** — rejected: an uncomputable/negative remainder after downtime has no honest meaning; an absolute deadline degenerates correctly (past = expired).
- **"Session wins, Core resumes after" coexistence** — rejected: the daemon would have to remember the prior Core state and re-assert it on expiry (extra state machine); a refusal is one line and cannot silently stack holds.
- **Leaving a migrated macOS Core flag ON in the background** — rejected: without the switch, nobody could ever turn it off from Settings; a trap, not a migration.
- **A backstop recovery agent (Insomnia's LaunchAgent)** — unnecessary: umux assertions are process-owned (fd/thread/kernel lifetime), not sticky global settings; daemon death releases by construction.

## Validation list (HITL, after implementation)

Walked by Adam on real hardware after Phase 4, macOS → Windows → Ubuntu order (same as v1.7.5):

- [ ] macOS (AC power): `umux core-session start 90m` → `pmset -g assertions` shows the umux hold; expiry auto-releases it, a desktop notification appears, Settings shows the ended state.
- [ ] macOS: `extend` moves the countdown; `stop` releases immediately; start is refused while a session runs past the 24 h cap on extend.
- [ ] macOS: daemon restart mid-session re-asserts the remaining window; restart after the deadline asserts nothing and records the expiry.
- [ ] macOS: after updating from a Core-ON state, the flag reads OFF, `umux core on` is refused with a pointer to `core-session start`, and Settings shows no Core switch — only Core Session (BETA) + Insomnia info.
- [ ] Windows: session holds (`powercfg /requests`), expiry releases + notification; Core switch unchanged; session refused while Core ON.
- [ ] Ubuntu: session holds (logind inhibitor), expiry releases + notification; same refusal rule.
- [ ] `umux status --json` shows identical session field names on all three platforms.
