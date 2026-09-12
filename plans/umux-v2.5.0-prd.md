# umux v2.5.0 — PRD (SSH View — remote umux control over SSH)

**Status:** planned — **new scope, discovered 2026-09-12** (/ask session: 13 questions in 2 series, all decisions confirmed by the PO the same day).
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Depends on:** the live CLI (v1.8.0) on the **remote** machine; the sidebar switcher + renameable labels (#130, v2.0.0); SSH support (shipped).

## Problem Statement

umux exists only on the machine it is installed on. A developer who works across several machines (desktop + laptop + servers) can reach the *other* machines only as raw SSH terminals — there is no way to see or steer the umux instance running there: its workspaces, its agents, its state. Meanwhile the phone/PWA path (Bridge, `development` branch) solves "away from the desk", not "at my other desk".

## Solution

The **[SSH] tab** of the sidebar switcher (v2.0.0) becomes the remote-operations view:

- A **machine list** (saved SSH connections + "add new"), each row showing the host, its online/offline status, the remote umux version (or "no umux"), and the count of active agents.
- **A machine with umux ≥ v1.8.0 is fully controllable**: its workspace tree rendered like the Local view, its agents listed, and **every action the remote `umux` CLI supports executed over plain SSH** — remote control through the remote's own CLI, no new protocol.
- **A machine without umux** opens as a plain SSH session (exactly today's behavior), with the remote-umux mode switching on automatically the moment a remote umux is detected — and umux **offers a remote install**: the user's choice of the CLI or the full desktop app.
- **An outdated remote** gets a version-mismatch prompt and a **remote update** through the existing update mechanism (issue #55).
- **Deliberately out:** no embedded remote terminal for umux machines and no live panel-output preview — that would be "too much" (PO decision 2026-09-12). The Bridge/PWA path stays untouched — both access channels coexist.

## User Stories

*(story numbers match the master PRD; added 2026-09-12)*

- **131.** As a user, I want the SSH view to list my saved SSH connections plus an "add new" entry — each row showing the host, its online/offline status, the remote umux version (or "no umux"), and the count of active agents on that machine, so that I pick a computer before connecting.
- **132.** As a user, I want a remote machine running umux **≥ v1.8.0** to be fully controllable from the SSH view — its workspace tree rendered like the Local view, its agents listed, and every action the remote `umux` CLI supports executed over SSH — so that I steer a remote umux from my own desk. *(No embedded terminal and no live panel-output preview.)*
- **133.** As a user, I want a machine without umux to open as a plain SSH session (as today), with the remote-umux mode switching on automatically the moment a remote umux is detected, and an offer to **install remotely** — my choice of the CLI or the full desktop app — so that upgrading the remote is one decision, not a chore.
- **134.** As a user, I want a version-mismatch prompt with **remote update** driven through the existing update mechanism (issue #55), so that an older remote umux can be brought to the required version straight from the SSH view.

## Implementation Decisions

- **Transport (2026-09-12):** the SSH view drives the remote machine by **running `umux …` commands over the existing SSH transport** (SshManager) and reading their JSON output. No socket forwarding, no new daemon on the remote beyond umux itself.
- **Detection:** `umux --version` / `umux status` over SSH on connect; version compared against the minimum (v1.8.0) and against the local version for the mismatch prompt.
- **Install offer:** one prompt, two choices — **CLI** (the documented install script/curl path) or the **full desktop app** (per-OS package); per-OS install mechanics at /carve.
- **Remote update:** reuse the shipped updater path (issue #55) executed remotely; the SSH view shows the outcome.
- **Naming:** the switcher tabs [Local][Agents][SSH] carry defaults but are **renameable** (story #130); the SSH view's own machine-picker sits at the bottom of the view (PO sketch 2026-09-12).
- **Relationship to Bridge (2026-09-12):** none — nothing changes. SSH View = desktop→desktop over the user's own SSH; Bridge = phone/PWA over the Cloudflare relay (`development` branch). Both coexist.

## Assumptions

- The remote's live CLI (≥ v1.8.0) covers everything the SSH view needs to show and steer (full app parity — story #87).
- Saved SSH connections (host, user, port, keys) already exist in umux; the machine list builds on them.
- Running commands over SSH is an acceptable security model — it grants nothing SSH itself doesn't already grant.

## Tradeoffs Considered

- **Socket forwarding instead of CLI-over-SSH** — rejected (PO, Q1 = 1): the CLI path is simpler and protocol-free; forwarding can be revisited later if a concrete need appears.
- **Embedded remote terminal / live panel-output preview for umux machines** — rejected (PO, 2026-09-12): "to już będzie za dużo"; plain SSH sessions remain available for non-umux machines.
- **Restricting the machine list to umux machines only** — rejected: machines without umux stay first-class (plain SSH), with the install offer as the bridge to more.
- **Auto-install without asking** — rejected: the install offer is always an explicit user choice (CLI vs full app).

## Validation Strategy

- **Machine list (#131):** saved machines show host, online status, umux version (or "no umux"), agent counts; "add new" opens the existing SSH connect flow.
- **Remote control (#132):** on a machine with umux ≥ v1.8.0, Adam steers the remote workspace tree live — create/rename/switch/send all round-trip through SSH; the remote tree matches what the remote machine itself shows.
- **No-umux path (#133):** a clean machine opens a plain SSH session; after installing umux remotely (either choice), the mode flips to remote control on the next connect — automatically.
- **Remote update (#134):** a remote with an older umux gets the mismatch prompt; the remote update brings it to the required version; the offer respects the zero-cost policy (GitHub Releases only).
- **Persistence:** renamed switcher labels and the machine list survive restarts.
- **HITL (Adam):** macOS + Windows first, Ubuntu after (the 2026-09-12 HITL order).

## Out of Scope

- Embedded remote terminals or live panel-output preview for umux machines (2026-09-12) — plain SSH sessions cover the terminal need on non-umux machines.
- Live sync/mirroring of workspaces between machines — out of scope per the master PRD (Bridge is control, not sync; so is the SSH View).
- Phone/PWA remote access — Bridge + Application on the `development` branch (final release v3.0.0); unaffected by this release.
- Teammates-as-panes for remote teams — v2.4.0 stays local.
