# PRD: umux Core Session — timed awake sessions

> Proposal [#108](https://github.com/CrystalPlatforms/umux/issues/108) + PO decisions 2026-10-19 (recorded [on #108](https://github.com/CrystalPlatforms/umux/issues/108#issuecomment-6260586732)). Extract of the master PRD ([`umux-prd.md`](./umux-prd.md) wins on conflict); stories **#144–#148**.
> Builds entirely on v1.7.5 umux Core (`plans/umux-v1.7.5-prd.md`, issues #93–#97, shipped). Reference project (design only, **zero code reuse**): [krishhgg/Insomnia](https://github.com/krishhgg/Insomnia) (MIT) — a macOS menu-bar app for timed awake sessions. umux takes ONLY the time dimension; **no menu bar** (PO decision).
> **Version slot: open — the number is assigned by the PO.** This document deliberately carries no version.

**Status:** planned — decisions locked 2026-10-19, implementation not started.

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

| # | Temat | Decyzja |
|---|---|---|
| 1 | Limit sesji | **24 h** maks. na jedną sesję |
| 2 | Wygaśnięcie | Widoczny stan w Settings („sesja zakończona o HH:MM") **+ powiadomienie desktopowe** w prawym dolnym rogu — ten sam mechanizm/styl co powiadomienie o nowej aktualizacji. **Nadpisuje cięcie „no notifications" z v1.7.5** — świadoma decyzja PO, tylko dla wygaśnięcia sesji (start/extend bez powiadomień). |
| 3 | Nazwa | **Core Session** — w UI „Core Session", w kodzie i CLI podkomenda **`core-session`** (z myślnikiem): `umux core-session start <czas>` / `extend` / `stop` |
| 4 | Źródło pomysłu | Tylko technologia/koncepcja Insomnii — zero kopiowania kodu, **żadnego menu baru** |
| 5 | macOS — Settings | Przełącznik Core **znika**; w jego miejscu przełącznik Core Session z badge **BETA** + info o Insomnii w sekcji |
| 6 | macOS — CLI | `umux core on` na macOS → odmowa z podpowiedzią (`umux core-session start`); `core off` pozostaje do obsługi starego stanu |
| 7 | macOS — migracja | Flaga Core ON **czyszczona przy aktualizacji** + jasne info w statusie/Settings, że Core został zastąpiony przez Core Session (beta) |
| 8 | Windows/Linux | Core bez zmian; `core-session start` przy Core ON → **odmowa z komunikatem** (Core już trzyma maszynę obudzoną bez limitu) |

### Technical pins (from the same decision session)

- **Absolute deadline, not remaining time**: the persisted state is `endsAt` (unix seconds). Restore computes `endsAt − now`; a deadline in the past after a restart means *expired* — no assertion, recorded in status.
- **Additive flag shape**: `storestation.core.json` gains `"session": {...}` alongside `"enabled"`; an old daemon reads only `enabled` (ignores the rest), a new daemon reads an old file as "no session". Corrupt file still reads as OFF (unchanged v1.7.5 rule).
- **One gate**: the macOS refusal of `core on` lives at the daemon (`core.set` op), not only in the CLI — no client can bypass it.
- **Cap = total session span**: `extend` cannot push `endsAt` beyond 24 h from session start.
- **CLI grammar**: durations as `<n><s|m|h>` (e.g. `90m`, `2h`); `--json` mirrors the status fields, consistent with the existing CLI discipline.
- **Status shape (additive)**: inside the existing `core` object of `umux status --json`: `session: {active, endsAt, remainingSecs, lastEndedAt, lastEndReason}`. `endsAt`/`lastEndedAt` are unix seconds; `lastEndReason` is `"expired" | "stopped" | null`. `sleepPrevented` keeps its meaning (held = Core ON or session active); the `core` object's contract keys (`enabled`, `held`, `instruction`) never change.
- **Expiry notification is daemon-side** (a session must outlive every window — that is the point of Core): the smallest per-platform path, no new heavy dependencies; visual family of the app's update notification.
- **Timer**: daemon-side; expiry drops the existing `PowerAssertion` value (inheriting the crash-release guarantee by construction) and records `lastEndReason: "expired"`. The platform backends (#93–#96) are untouched.

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

- [ ] macOS (AC power): `umux core-session start 90m` → `pmset -g assertions` shows the umux hold; expiry auto-releases it, a desktop notification appears, Settings shows the ended state.
- [ ] macOS: `extend` moves the countdown; `stop` releases immediately; start is refused while a session runs past the 24 h cap on extend.
- [ ] macOS: daemon restart mid-session re-asserts the remaining window; restart after the deadline asserts nothing and records the expiry.
- [ ] macOS: after updating from a Core-ON state, the flag reads OFF, `umux core on` is refused with a pointer to `core-session start`, and Settings shows no Core switch — only Core Session (BETA) + Insomnia info.
- [ ] Windows: session holds (`powercfg /requests`), expiry releases + notification; Core switch unchanged; session refused while Core ON.
- [ ] Ubuntu: session holds (logind inhibitor), expiry releases + notification; same refusal rule.
- [ ] `umux status --json` shows identical session field names on all three platforms.
