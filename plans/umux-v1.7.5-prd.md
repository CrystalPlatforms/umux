# PRD: umux v1.7.5 — umux Core (Always-On device)

> Extract of the master PRD ([`umux-prd.md`](./umux-prd.md) wins on conflict); stories #141–#143.
> Discovered 2026-09-20 in an /ask session with the PO. Same day, the v1.7.0 daemon was **rebranded umux Storestation** and the name **umux Core** was assigned to this feature.

**Status:** planned — slot after v1.7.0 ships, before v1.8.0 (PO decision 2026-09-20).

## What it is

An optional **"Core (Always-On device)" switch** in Settings (**off by default**). When ON, the daemon holds an OS power assertion so the **machine — not the screen** — stays awake for as long as Core holds it, **even with every umux window closed and zero sessions live**. The block releases on daemon stop / toggle-off.

Naming: "umux Core" from v1.7.5 on means THIS feature. The v1.7.0 daemon is **umux Storestation** (settings section "Storestation"; Core gets its own "Core" section in Settings — two sections, one daemon).

## Why

Agents and long jobs outlive attention: the user closes the lid or walks away, the OS suspends the machine, and everything stops. umux already keeps sessions alive across app close (v1.7.0 Storestation); Always-On closes the remaining gap — the OS itself putting the computer to sleep. Screen lock is explicitly NOT a problem (locking the screen stops nothing).

## User stories (master PRD numbering)

- **141.** As a user, I want a "Core (Always-On device)" switch in Settings (off by default), so my machine stays awake for as long as umux Core holds it — even with every umux window closed.
- **142.** As a developer, I want Always-On to be best-effort per platform — an OS power assertion wherever possible, clear instructions where the OS refuses (Windows/Linux lid-close action, macOS on battery) — with the state visible in `umux status --json` (`sleepPrevented`), so the limits are never silent.
- **143.** As a user, I want the daemon to keep only the machine awake (the screen may sleep) and to release the block on stop or toggle-off, so enabling Core never changes anything beyond the machine staying awake.

## Decisions (discovery 2026-09-20, PO signed off)

- **Trigger:** the manual switch only — no session-based logic, no activity heuristics. ON = hold continuously, even at zero sessions.
- **Holder:** the daemon (umux Storestation) is the single holder — toggling Core ON spawns the daemon if absent; works with every window closed (the whole point of the feature).
- **Guarantees: best-effort + instructions, NO admin/root paths** (PO decision Q14): hold an OS assertion wherever possible; where the OS refuses, show instructions instead of failing silently:
  - macOS: power assertion (`caffeinate`-style, no admin) — reliable on AC power; on battery macOS may still force sleep (documented, shown in status).
  - Windows: `SetThreadExecutionState` stops idle sleep without admin; the **lid-close action** is a power-plan setting — umux shows instructions (no silent failure).
  - Linux: idle-sleep inhibit via the desktop session; the **lid switch** belongs to logind (root) — instructions shown.
- **Screen may sleep** — only the system is held awake.
- **UX:** the switch in the Core section of Settings + a `sleepPrevented` field in `umux status --json`. No notifications (decided).
- **Platforms/HITL order:** macOS + Windows first, Ubuntu after (same as v1.7.0).
- **Remote shutdown** ("turn my machine off from the phone") is a future Ecosystem capability, out of scope here.

## Out of scope

- Wake-on-LAN / any remote wake, schedules (time windows), auto-resume after a forced sleep (wake-time session rebind is v1.7.0 scope and already planned).
- Admin/root elevation; changing OS power settings automatically.
- Any battery-level intelligence.

## Rejected alternatives

- **Session-based holding** (hold only while a session/agent is active) — rejected 2026-09-20: complexity without a stated need; the switch is explicit user intent.
- **App-process holder** — rejected 2026-09-20: dies with the windows; the scenario is "close everything, machine stays on".
- **Admin/root power-plan changes for full guarantees** — rejected 2026-09-20: umux does not modify the user's system power configuration; instructions instead.

## Validation list (HITL, after implementation)

- [ ] macOS: Core ON on AC → close the lid → machine stays awake (`pmset -g assertions` shows the umux assertion); Core OFF / daemon stop → assertion gone.
- [ ] macOS on battery: the documented limit is visible (`sleepPrevented` state / instruction), no silent failure.
- [ ] Windows: Core ON keeps the machine awake with the lid open; lid-close shows the instruction path; toggle-off releases.
- [ ] Ubuntu: idle-sleep inhibit works in GNOME; lid instruction path shown.
- [ ] `umux status --json` reports the Always-On state in every case above.
