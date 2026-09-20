# umux v1.7.0 — PRD (umux Storestation + landing-page revamp)

**Status:** planned — the next build target. Discovered 2026-09-04 (the "umux Ecosystem" /ask + /blueprint session) as part of v2.0; **rescheduled 2026-09-12 (PO decision)** to be v1.7.0 — the first ecosystem piece to ship. Built on branch **`core`**, released from `main` like any 1.x version.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Scope:** the optional umux Storestation daemon (stories #105–#113) plus the landing-page revamp (story #136 — docs section and more; exact scope decided at v1.7.0 planning).
**HITL order (PO decision 2026-09-12):** macOS and Windows first, Ubuntu after.

## Problem Statement

Closing the umux window kills every session: agents stop mid-task, long jobs die, and there is nothing to reattach to — the work existed only as long as the window did. The desktop app, the future TUI, and the CLI live as separate islands with separate lifetimes instead of views on the same running work. (And on the web, umux's landing page is a one-pager — a visitor who wants to *learn* the tool has to dig through the repo.)

## Solution

**umux Storestation** — an optional background daemon, **off by default**, that owns the terminal sessions:

- With Storestation on, closing every window leaves agents and long jobs alive; **`umux attach`** brings any view back.
- A dedicated **Storestation section in Settings** (daemon on/off, autostart on/off, live status) styled like the import wizard.
- Optional **autostart at login**, so a reboot never orphans the setup.
- The **CLI drives Storestation headlessly** while no window is open.
- Storestation ships **inside the normal installers** and updates with the app — enabling it never means installing something extra.

With Storestation OFF, the desktop app behaves exactly like today — the daemon is pure opt-in. Beside the daemon, this release **expands the landing page** (docs and more — scope at planning).

## User Stories

*(story numbers match the master PRD; #105–#113 formed the Storestation block of the old v2.0 ecosystem scope)*

- **105.** As a developer, I want Storestation to keep my terminal sessions alive after I close umux Desktop, so that a closed window never kills a running agent or long job.
- **106.** As a developer, I want `umux attach` to reattach the desktop app or the CLI to Storestation's living sessions, so that coming back costs seconds and loses nothing.
- **107.** As a user, I want Storestation OFF to leave the desktop app behaving exactly as today (sessions end when the app closes), so that the daemon is pure opt-in.
- **108.** As a user, I want a dedicated Storestation section in Settings — daemon on/off, autostart on/off, and running status — styled like the import wizard, so that all ecosystem controls live in one obvious place.
- **109.** As a user, I want Storestation to start automatically at login when autostart is enabled, so that the ecosystem reaches my machine even after a reboot.
- **110.** As a developer, I want stopping Storestation to terminate its shells cleanly (no orphan processes) and a crashed Storestation's leftovers to be detected and cleaned on the next start, so that the daemon never litters my system.
- **111.** As a developer, I want the CLI's commands to work against Storestation while no window is open, so that scripts and agents can drive umux headlessly. *(The full live surface completes at v1.8.0.)*
- **112.** As a developer, I want umux Terminal (TUI) to be able to attach to the same Storestation sessions as the desktop app, so that both are interchangeable views on the same work. *(Store separation (v1.9.0) untouched; this story completes when the TUI ships in v1.9.0.)*
- **113.** As a user, I want Storestation to ship inside the normal installers and update with the app, so that enabling it never means installing something extra.
- **136.** As a visitor, I want an expanded umux landing page — a documentation section and more content beyond the current one-pager — so that I can learn and adopt umux without digging through the repo. *(Scope decided at v1.7.0 planning.)*

## Implementation Decisions

- **SessionCore** *(deep module)* — one session-management interface with two interchangeable drivers: *in-process* (today's behavior, Storestation OFF) and *daemon-client* (all operations proxied to umux Storestation over the local socket, Storestation ON). Desktop and CLI become views; the UI never knows which driver is active.
- **umux Storestation** *(deep module)* — headless Rust binary owning PTYs and session state: serves the **local socket API**, survives app close, optional per-OS autostart (mechanism per platform decided at /carve), clean shutdown with no orphan shells, crash-leftover cleanup on next start.
- **Socket protocol ownership (2026-09-12):** because Storestation now **precedes** the live CLI, umux Storestation defines the local-socket protocol; the desktop CliGateway (v1.8.0) joins it. The protocol is designed extensible from day one.
- **Settings → Storestation section** — daemon on/off (default off), autostart on/off, live status; the paired-devices list with revoke fills in later with the Bridge work (`development` branch).
- **Branch model (PO decision 2026-09-12):** development happens on a long-lived **`core` branch**; releases are tagged from `main` as normal versions. Test instances run against an **isolated data directory** (separate store, socket, sessions — e.g. `~/.umux-test`) so the test daemon never touches a daily-use umux.
- **Additive by design:** with Storestation never enabled, the app behaves exactly like v1.6.x.
- **Landing page** — lives in the existing Cloudflare Pages deployment (umux.pages.dev); scope (docs structure, sections, content) decided at v1.7.0 planning with Adam.

## Assumptions

- A local, per-user socket is an acceptable security boundary (no remote exposure).
- Cloudflare Workers/Pages free tiers keep covering the landing page (unchanged hosting).
- Per-OS autostart mechanisms (systemd user unit / LaunchAgent / Windows autostart) are enough for story #109; exact choice at /carve.
- The zero-cost policy holds — Storestation is just another binary in the existing installers.

## Tradeoffs Considered

- **Background daemon** — was rejected on 2026-08-28 as the single most complex component and deferred; **superseded 2026-09-12** — the PO pulled it forward as v1.7.0, accepting the complexity to get attach-first.
- **Storestation mandatory for the desktop app** — rejected (2026-09-04): the app stays fully standalone; Storestation optional and off by default.
- **Autostart always-on** — rejected (2026-09-04): user-controlled switches in the dedicated Storestation Settings section.
- **Building Storestation on the `development` branch** — superseded 2026-09-12: Storestation gets its own `core` branch; the `development` branch remains reserved for the Bridge/PWA ecosystem finale.

## Validation Strategy

- **Storestation ON:** Adam starts an agent, closes every umux window — the agent keeps running (`umux status` answers against Storestation, output still growing); `umux attach` restores the view.
- **Storestation OFF:** everything matches v1.6.x behavior exactly.
- **Lifecycle:** stopping Storestation leaves zero orphan shells (process audit); a hard-killed Storestation leaves no garbage after the next start; autostart actually launches Storestation after a reboot.
- **Settings:** the Storestation section's toggles persist; status reflects reality.
- **HITL order:** macOS and Windows first, then Ubuntu (PO decision 2026-09-12).
- **Landing page:** the new docs render on umux.pages.dev and the deploy stays zero-cost.

## Out of Scope

- The rest of the ecosystem (Bridge, PWA, NativeApps) — `development` branch, ships as v3.0.0 (documented in the Ecosystem PRD there).
- The TUI itself and full live-CLI parity — v1.9.0 and v1.8.0 respectively.
- Plugins, marketplace, browser pane — Beyond the Ecosystem. Cross-machine synchronization — out of scope (remote access is control, not sync).
