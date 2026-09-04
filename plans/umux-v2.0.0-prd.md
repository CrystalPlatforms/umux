# umux v2.0.0 — PRD — umux Ecosystem

**Status:** discovered 2026-09-04 (dedicated /ask session, 30 questions in 4 series + /blueprint component sketch) — replaces the former outline of the same file. Implementation starts **after v1.5.0–v1.9.0**, on a long-lived `development` branch, no deadline.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins. Story numbering below continues the master's (#105+).

## Problem Statement

Closing the umux window kills every session: agents stop mid-task, long jobs die, and there is nothing to reattach to — the work existed only as long as the window did. And umux exists only on the machine it is installed on: once its owner leaves the desk, they are blind — no statuses, no way to see what an agent did, no way to nudge it, no signal when it needs a decision. The desktop app, the TUI, and the CLI also live as separate islands with separate lifetimes instead of views on the same running work.

## Solution

**umux Ecosystem** — five named pieces that turn umux from "an app" into a workspace platform:

1. **umux Desktop** — the existing app; gains the ability to run attached to Core.
2. **umux Terminal** — the existing TUI; same ability.
3. **umux Core** — an optional background daemon (off by default) that owns the terminal sessions. With Core on, closing every window leaves agents and long jobs alive; `umux attach` brings any view back; the CLI and TUI drive the same live sessions while no window is open.
4. **umux Application** — an installable PWA: sign in (Google or GitHub), see your paired machines and their workspaces, tabs, and agent statuses, watch an agent's output read-only ("like on the computer"), send a message to a running agent, and get a push notification when an agent finishes or waits for approval.
5. **umux Bridge** — the remote-access link between the PWA and a paired machine: one-time-code pairing, device list with revoke, one-year pairing expiry, relayed through Cloudflare with TLS so the machine opens no inbound ports.

The cloud is a dumb cable joiner: it relays encrypted traffic and holds accounts/pairings — never message content. No plugins, no marketplace, no browser pane (all moved to **Beyond 2.0**).

## User Stories

### umux Core (daemon) — v2.0
105. As a developer, I want Core to keep my terminal sessions alive after I close umux Desktop or umux Terminal, so that a closed window never kills a running agent or long job.
106. As a developer, I want `umux attach` to reattach Desktop, Terminal, or the CLI to Core's living sessions, so that coming back costs seconds and loses nothing.
107. As a user, I want Core OFF to leave the desktop app behaving exactly as today (sessions end when the app closes), so that the daemon is pure opt-in.
108. As a user, I want a dedicated Core section in Settings — daemon on/off, autostart on/off, and running status — styled like the import wizard, so that all ecosystem controls live in one obvious place.
109. As a user, I want Core to start automatically at login when autostart is enabled, so that Bridge/PWA reach my machine even after a reboot.
110. As a developer, I want stopping Core to terminate its shells cleanly (no orphan processes) and a crashed Core's leftovers to be detected and cleaned on the next start, so that the daemon never litters my system.
111. As a developer, I want the CLI's live commands to work against Core while no window is open, so that scripts and agents can drive umux headlessly.
112. As a developer, I want umux Terminal (TUI) to be able to attach to the same Core sessions as the desktop app, so that both are interchangeable views on the same work. *(Store separation from v1.7.0 is untouched; this is about live sessions only.)*
113. As a user, I want Core to ship inside the normal installers and update with the app, so that enabling it never means installing something extra.

### umux Application (PWA) — v2.0
114. As a user, I want to install the umux Application on my phone straight from the browser (PWA), so that the ecosystem travels with me without an app store.
115. As a user, I want to sign in with Google or GitHub, so that I don't create yet another password.
116. As a user, I want to see my paired machines with their workspaces, tabs, and agent statuses (or "offline"), so that I know what is running from anywhere.
117. As a user, I want to watch an agent's output read-only — the same thing I would see on my computer — so that I can catch up on what the agent did.
118. As a user, I want to see recent output history of an agent tab in the PWA, so that I get context, not just the last line.
119. As a user, I want to send a message to a running agent from the PWA, so that I can redirect or follow up without being at the desk.
120. As a user, I want a push notification when an agent finishes or waits for my approval, so that I come back exactly when needed.
121. As a user, I want everything in the PWA read-only except the single "send message to agent" action, so that a stolen session can never type shell commands.

### umux Bridge (remote access) — v2.0
122. As a user, I want to pair a machine with my account using a one-time code shown on the machine and entered in the PWA, so that only I can reach my computers.
123. As a user, I want to see and revoke paired devices in Settings, so that a lost device can be cut off instantly.
124. As a user, I want pairings to expire after one year, so that stale authorizations don't live forever. *(Re-pair to continue.)*
125. As a user, I want all remote traffic relayed through Cloudflare over TLS with the machine holding no open inbound ports, so that remote access is safe from any network.
126. As a maintainer, I want the relay to store no message content (pass-through only; accounts and pairings are the only persisted data), so that the privacy-friendly story stays true.
127. As a maintainer, I want accounts limited to 3 paired machines, so that free-tier limits are protected from abuse.

## Implementation Decisions

### Naming
- **umux Desktop**, **umux Terminal**, **umux Application** (the PWA), **umux Core** (the daemon), **umux Bridge** (the remote-access link). Working names — the PO called them "not binding" (2026-09-04). Future: integration with the Crystal Studio ecosystem (**Beyond 2.0**).

### Major functional components (for /carve)

**Rust (main repo):**

1. **SessionCore** *(deep module)* — One session-management interface with two interchangeable drivers:
   - *in-process driver* — today's behavior, PTYs owned by the app (Core OFF);
   - *daemon-client driver* — all operations proxied to umux Core over the local socket (Core ON).
   Desktop/TUI/CLI become views; the UI never knows which driver is active. The v1.7.0 CliGateway socket protocol evolves into Core's local API.

2. **umux Core** *(deep module)* — Headless Rust binary owning PTYs and session state: serves the local socket API, survives app close, optional per-OS autostart (mechanism per platform decided at /carve), clean shutdown with no orphan shells, crash-leftover detection on next start. Shipped inside the existing NSIS/DMG/.deb/AppImage packages — no separate install.

3. **AgentMonitor** *(deep, pure)* — Consumes the OscParser stream plus a bounded output tail per panel and derives: agent state (working / waiting / finished / idle), the read-only PWA view, and push triggers. Pure logic over fed bytes — unit-testable. No new output semantics beyond the already-decided OSC-only completion + known-CLI process presence.

4. **BridgeAgent** *(deep module)* — Outbound-only WebSocket(s) from Core to the Cloudflare relay, authenticated by the pairing token; speaks a versioned, whitelisted JSON protocol (status events, read-only output tail, message-to-agent inbox). There is deliberately **no** message type that types into the terminal or runs commands.

5. **Relay + Auth (Cloudflare Workers, Durable Objects, KV/D1)** — The relay joins viewer and machine connections pass-through (persists no content); the Auth worker implements Google + GitHub OAuth directly, accounts, the machine registry (limit 3), pairing (one-time code), device revoke, and one-year expiry. PWA hosted on Cloudflare Pages; Web Push (VAPID, free) fired on agent finished/waiting.

**Frontend (main repo):**

6. **Settings → Core section** — Import-wizard-style section: daemon on/off (default off), autostart on/off, live status, paired-devices list with revoke.

### Key data flows (new in v2.0)
- **Remote view:** PTY output → OscParser/AgentMonitor → status + bounded tail → Core → BridgeAgent → relay (WSS) → PWA.
- **Remote action:** PWA → relay → BridgeAgent → Core → PTY stdin of the agent panel (messages only).
- **Push:** AgentMonitor state transition (finished / waiting) → relay → Push Worker → device (Web Push).
- **Attach:** Desktop/TUI/CLI SessionCore daemon-client → Core local socket.

### Technology-specific constraints
- **Cloud is Cloudflare-only and free-tier only** (Workers, Durable Objects, KV/D1, Pages). No other cloud, no self-host option in v2.0.
- **Auth providers:** Google + GitHub via direct OAuth on Workers — no third-party auth vendor.
- **Pairing security model:** one-time code, device list + revoke in Settings, one-year expiry, 3 machines per account, TLS everywhere. End-to-end encryption deferred (see Tradeoffs).
- **PWA writes:** exactly one — "send message to agent". No terminal input, no commands, no file access.
- **Development model (PO decision, 2026-09-04):** all v2.0 work happens in the main repo on a long-lived **`development` branch**; `main` receives only 1.x releases; merge at 2.0.0. Test builds run against an **isolated data directory** (separate store, socket, sessions — e.g. `~/.umux-test`) so the test ecosystem never touches a daily-use umux (PO decision, 2026-09-04).
- **Compatibility:** every v2.0 feature is additive; with Core never enabled, the app behaves exactly like v1.9.x.
- **Scope guard:** no plugins, no marketplace, no browser pane — **Beyond 2.0**. Cross-machine workspace **synchronization** remains out of scope; Bridge is remote *control*, not sync (2026-09-04).

## Assumptions

- Cloudflare free tiers suffice for the PO plus early users (Workers ~100k requests/day, Durable Objects free allotment, KV/D1, Pages); the 3-machine limit guards against abuse. Verified monthly during testing.
- Google and GitHub OAuth remain free with open registration.
- A suitable free domain (candidates: DigitalPlat `umux.dpdns.org`, DNSHE `umux.de5.net` / `us.ci` — both checked live 2026-09-04) is available when Bridge ships; fallback is the existing `*.pages.dev` URL. Decision deferred, non-blocking.
- A PWA viewer sees the agent panel's output as a live tail + bounded recent history; full scrollback archives are not expected by anyone.
- "Send message" = writing to the agent panel's stdin (the same channel as `umux send`); the PO accepts doing this without seeing the live prompt state.
- One-year pairing expiry is acceptable UX (re-pair once a year).
- Users accept that PWA history is recent output only.
- The v1.7.0 CliGateway socket protocol proves extensible into Core's local API without a rewrite (it is the declared foundation).

## Tradeoffs Considered

- **Plugin system + marketplace in v2.0** — rejected by the PO (2026-09-04): v2.0 is ecosystem-only; both move to Beyond 2.0.
- **Scriptable browser pane in v2.0** — rejected by the PO (2026-09-04): least ecosystem-related pillar; Beyond 2.0.
- **Full remote terminal control in the PWA** — rejected: typing shell commands from a remote session is the top security risk and not needed; message-to-agent only.
- **Chat-style extracted agent messages in the PWA** — rejected for v2.0 (the PO wants it "like on the computer"): raw read-only view of the agent panel output; extraction may come later.
- **LAN-only remote access** — rejected by the PO (2026-09-04): control from anywhere was the point; Cloudflare relay chosen.
- **Third-party auth vendor (Auth0/Clerk/Stack…)** — rejected: free tiers exist today, but direct OAuth on Workers keeps zero-cost permanent and dependencies minimal.
- **End-to-end encryption in v2.0** — deferred by the PO (2026-09-04): TLS now; E2E as a future version (more work, more failure modes).
- **Separate repo for ecosystem code** — rejected by the PO (2026-09-04): everything in the main repo on a `development` branch.
- **Core mandatory for the desktop app in v2.0** — rejected: the app stays fully standalone; Core optional and off by default (2026-09-04).
- **Autostart always-on** — rejected: user-controlled switches in the dedicated Core Settings section (2026-09-04).
- **No push notifications in v2.0** — rejected by the PO (2026-09-04): Web Push is free via Cloudflare and pairs naturally with "agent waits for approval".

## Validation Strategy

Adam tests on **Windows 11, macOS, and Ubuntu** using a **test instance** built from the `development` branch with the isolated data directory; the daily-use umux must be untouched throughout.

### Per-user-story verification
- **Core (#104–#112):** with Core ON, close the desktop mid-agent-run → the agent keeps running (`umux status` live, output still growing); `umux attach` restores the view; with Core OFF everything matches v1.9.x behavior; clean stop leaves zero orphan shells (process audit per platform); a hard-killed Core leaves no garbage after the next start; autostart actually launches Core after a reboot on each OS; the Core Settings section's toggles persist.
- **PWA (#113–#120):** installable to the phone home screen; sign-in works with both Google and GitHub; machines/workspaces/statuses match what the desktop shows, including "offline" for a sleeping machine; the read-only view shows the same content as the panel; sent messages appear in the agent session; a push arrives on finish and on waiting; the protocol contains no terminal-input or command message type (enforced by tests over BridgeAgent's codec).
- **Bridge (#121–#126):** an unpaired device cannot connect; a revoked device is cut off within seconds; expiry enforced (TTL shortened in a test environment); relay code review confirms no content persistence; a fourth machine is refused with a clear message.

### Component "done" criteria
- **SessionCore:** unit tests over the interface with a fake driver + integration tests with both drivers proving identical observable behavior for every operation.
- **umux Core:** integration tests — spawn, write, resize, close, clean shutdown, crash-recovery scan; no orphan processes after every path.
- **AgentMonitor:** unit tests with fixed byte fixtures — each state transition, tail bounds, push-trigger conditions.
- **BridgeAgent:** unit tests over the codec (whitelist exhaustive; unknown message types rejected).
- **Relay/Auth:** integration tests against a staging Worker — OAuth both providers, pairing happy path + wrong code + expired + revoked, machine limit enforced, relay forwards bytes without storing them.

### Acceptance threshold (v2.0.0)
Adam, with test instances on all three machines: starts an agent on the Mac, closes every umux window, leaves home, opens the PWA on his phone, sees the agent still running, reads its output, sends it a message, receives a push when it finishes, and — back at the Mac — `umux attach` restores everything. On Windows, with Core never enabled, umux behaves exactly like v1.9.x. Monthly Cloudflare usage stays within the free tier.

## Out of Scope

- Plugin system and marketplace — **Beyond 2.0** (moved here from the old v2.0 plan, 2026-09-04).
- Scriptable browser pane — **Beyond 2.0** (moved here from the old v2.0 plan, 2026-09-04).
- Crystal Studio ecosystem integration — **Beyond 2.0**.
- Cross-machine synchronization of workspaces (Bridge is remote control, not sync).
- Full remote terminal (typing shell commands from the PWA) and multiple simultaneous remote viewers driving the same machine.
- End-to-end encryption (future version); chat-extraction agent view (future version).
- Native mobile apps (PWA only); sharing machines between accounts; self-hosted or non-Cloudflare relay.
- Any paid component (certificates, hosting, auth vendors).

## Further Notes

- Roadmap position: after v1.9.0; **no deadline**; the `development` branch is created when v2.0 work actually starts — nothing changes on `main` today.
- Names are working names; final naming may adjust before release.
- Deep modules (SessionCore, AgentMonitor, BridgeAgent codec) and the per-OS autostart mechanisms get their detailed design at /carve time.
- Discovery trail: /ask session 2026-09-04 (Q1–Q30), /blueprint component sketch same day; the former outline in this file was replaced by this PRD.
