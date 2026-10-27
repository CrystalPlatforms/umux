# umux v1.8.0 — PRD (live CLI + local socket)

**Status:** planned — was v1.7.0 scope ("umux Terminal + live control"); **renumbered 2026-09-12** into its own release, after umux Core (v1.7.0).
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Confirmed:** 2026-08-31 ("versions cleanup" discovery — full live parity, not a curated subset); **discovery refresh 2026-10-27** (/ask + /blueprint with the PO): unified command family, socket ownership, offline fallback, retrofit package, security model. Protocol reference: [`umux-storestation-cli-protocol.md`](./umux-storestation-cli-protocol.md).

## Problem Statement

The shipped CLI (tags v1.0.3–v1.0.4, now v1.6.x) is offline-only: scripts and AI agents can inspect and edit *saved definitions*, but cannot see or steer a *running* app. Anything alive — workspaces with open tabs, running panels, agent states, Settings — is invisible to automation. On top of that the v1.6.x surface is human-shaped only: no `--json`, prose errors, no introspection — automation gets half a CLI.

## Solution

The `umux` CLI becomes **one command family** (canonical `resource.verb` names; the old short names stay as aliases) that adapts to what is actually running, and the app/daemon expose a **local socket API** speaking the **Storestation protocol v1** — the same vocabulary the daemon already serves. **Full parity with the running app** (confirmed 2026-08-31, re-confirmed 2026-10-27): anything the UI can do — workspaces, groups, tabs, panes, splits, rename, send, **and Settings actions** — the CLI/socket can do live. Every mouse-only interaction (drag to reorder, pane resize, multi-select bulk actions) gets a numbered command equivalent, so parity is complete and testable.

### One family, three worlds (PO 2026-10-27)

Example: `umux workspaces create "projekt"`.

| Situation | What happens |
|---|---|
| **App alive** (with or without Storestation) | Executes **live** through the app's socket gateway |
| **Daemon only** (Storestation ON, windows closed) | Layout commands **refuse** (exit 3) with the hint „run `umux attach`"; session commands work (the daemon's domain); Storestation/Core settings work |
| **Nothing alive** | Layout mutations **write saved definitions** (exactly today's offline behavior, story continuity with v1.6.x); reads print saved definitions |

Commands that inherently need a live target (`tabs send`, attach-like actions) always require one — exit 3 with the v1.7.0 error object.

## User Stories

*(#53, #54, #55, #87 match the master PRD; the rest are local to this release.)*

1. **(#53)** As a developer, I want `umux list` / `umux status` to print live workspaces, tabs, panels, and agent states as JSON, so that scripts and agents can inspect the current setup. When the app is alive the output has **two sections: live state + saved definitions** (PO 2026-10-27); with nothing alive it shows saved definitions as today.
2. **(#54)** As a developer, I want control commands (`workspaces create`, `tabs create`, `panes split`, `tabs send`), so that external tooling can build and drive layouts.
3. **(#55)** As an AI agent, I want the same surface exposed over a local socket API, so that I can orchestrate umux programmatically without parsing CLI text.
4. **(#87)** As a developer, I want the live CLI/socket surface to cover **everything the running app can do** — every workspace, group, tab, pane, split, rename, send, **and settings action** (PO 2026-10-27: Settings included) — not a curated subset.
5. As a user, I want every mouse-only interaction to have a numbered command equivalent (tab reorder/move, pane resize, bulk tab close, …), so layout automation is complete and parity is verifiable program-wide.
6. As a user, I want **one command family**: canonical `resource.verb` names whose behavior adapts to the three worlds above — so I never learn two vocabularies. *(PO 2026-10-27: "chcę żeby były tylko jedne".)*
7. As a developer, I want the CLI to **never write the store while the app is running** — every live mutation goes through the app's gateway — so the store always has exactly one writer.
8. As a script author, I want the old v1.6.x commands **retrofitted** — `--json`, the error catalog (`code`/`message`/`next`/`retryable`), `--limit` on `list`, and `create`/`delete`/`update` aliases beside `new`/`rm`/`rename` — with **nothing removed**, so existing scripts keep working and new automation gets structured output everywhere. *(From the /agent-cli audit: skipping this is a Blocker at v1.8.0. PO confirmed the full package 2026-10-27.)*
9. As a tooling author, I want `--help` and `agent-context` **generated from a single command table**, so the documented surface cannot drift from the real one.
10. As a user, I want `umux status --json` to report **who serves the socket** (app / daemon / none) and the protocol version, so I can diagnose connectivity. *(Additive fields.)*
11. As a user, I want the live CLI documented in the README and in the landing Docs section, so humans and agents can onboard. A full standalone umux docs site is future work (PO 2026-10-27).

## Implementation Decisions

- **Socket ownership — "whoever is alive" (PO 2026-10-27):** the app serves the protocol when Storestation is OFF; the daemon serves when it is ON and the app connects to it **as a client** (the v1.7.0 pin: "the gateway becomes one more client of the socket rather than the sole server"). The CLI connects to whichever socket is alive; `umux status` reports which one answered. Handover rules (daemon starts/stops while the app runs) — detail at /carve.
- **One command family (PO 2026-10-27):** canonical names `resource.verb` (e.g. `umux workspaces create`, `umux tabs send`); the v1.6.x short names remain as **aliases of the same commands**. No second, parallel surface.
- **Single-writer guarantee:** while the app runs, the CLI never touches the store — the gateway is the only write path; direct store writes happen only in the "nothing alive" fallback.
- **Full parity inventory:** drawn from the actual UI at /carve (workspaces, groups, tabs, panes, splits, send, agent status, Settings) and becomes the parity-test fixture; mouse-only actions map to numbered ops (e.g. `tabs move`, `panes resize`, bulk close).
- **Security (PO 2026-10-27):** local per-user transport only — Unix socket perms `0600` / Windows named pipe with per-user ACL; **no auth, no token, no remote exposure, ever** (the tmux/iTerm2 model: anything running as the user may steer the terminal).
- **One command table (single source of truth):** `--help`, `agent-context`, and CLI dispatch are generated from it; a parity test asserts they agree (agent-cli backstop: the surface passes ~20 commands).
- **Protocol v1 grows additively:** the daemon already answers `unknownOp` for unimplemented ops, which keeps this release purely additive; new error codes join the catalog the same way.
- **Exit codes:** live commands that need a target reuse exit 3 from the v1.7.0 catalog ("required but not reachable"); offline is a state, not an error, where a valid offline answer exists.
- **Docs:** README gains a "Live CLI" section (with agent examples built on `agent-context`); the landing Docs section grows accordingly — **landing changes always gated by the PO's acceptance before push** (standing rule).
- **QuickType** (TS types generated from the protocol JSON Schema): decision deferred to /carve (PO 2026-10-27).

## Assumptions

- The Storestation socket protocol v1 proves extensible to the desktop gateway without a rewrite — growth is additive ops only (first /carve prototype de-risks this).
- Direct store writes are safe **only** when nothing is alive: the single-instance app is closed and the daemon (which never writes the store) is off.
- The app-as-daemon-client mode preserves the single-instance / no-double-writer guarantees won in v1.7.0.
- A local, per-user socket without auth is an acceptable security boundary (PO signed off 2026-10-27).
- Every mouse interaction is expressible as data (ids + numbers/strings) — no gesture-only semantics.

## Tradeoffs Considered

- **Two command families (separate live verbs vs offline verbs)** — rejected by the PO (2026-10-27): two vocabularies to learn, two code paths, drift between them.
- **Live-only unified commands (no offline fallback)** — rejected by the PO (2026-10-27): would kill v1.6.x offline planning, and the old aliases would behave differently from their canon — two kinds back through the back door.
- **Daemon always serves the socket (Storestation required for live)** — rejected (2026-10-27): would make the optional, default-off daemon a hard dependency of the flagship feature.
- **Two sockets (app's + daemon's), CLI merges answers** — rejected (2026-10-27): duplicate state and conflicting answers; more moving parts than one server behind one path.
- **Daemon writes the store while windows are closed (headless layout edits)** — rejected (Q2, 2026-10-27): reintroduces the double-writer risk the architecture spent v1.7.0 eliminating; an honest refusal + `umux attach` hint is one line and cannot corrupt state.
- **Queue UI ops and execute them at next app start** — rejected: hidden, delayed execution surprises users and agents about what actually happened.
- **Optional socket auth token** — rejected (2026-10-27): adds no safety within a single user account; tmux/iTerm2 precedent.
- **A curated live-command subset** — rejected by the PO (2026-08-31): full parity chosen.
- **A second, desktop-specific protocol** — rejected (2026-09-12): Storestation owns the protocol; the gateway reuses it.
- **Hand-maintained `agent-context` / `--help` layers** — superseded (2026-10-27): with 20+ commands the hand-written layers drift; generated from one table.

## Validation Strategy

- With the app running: `umux status` / `send` / `split` steer it live; the socket API returns the same data as the CLI (contract test).
- **Full-parity check:** inventory-driven — every UI action (Settings included, story 4) is reachable through both the CLI and a raw socket client; automated where possible, HITL for the rest.
- **Three-worlds HITL:** `workspaces create` with the app closed → the definition is saved and appears at next launch; with the app open → created live; daemon-only → exit 3 refusal naming `umux attach`.
- **Backward compatibility:** the alias suite (`new`, `rm`, `rename`, …) behaves exactly as before when nothing is alive; existing scripts keep working.
- **Single-writer test:** while the app runs, mutations never touch the store file directly (observable via file mtime/watcher in tests).
- `agent-context` parity test against `--help` (generated from the same table — story 9).
- `umux status --json` schema test: the new gateway fields are additive; older clients ignore them (story 10).
- Security: socket perms `0600` (Unix) / per-user ACL (Windows) asserted; no network listener exists (port-scan check).
- **HITL order: macOS → Windows → Ubuntu** (PO 2026-10-27), walked on the PO's real hardware; npm/cargo suites stay green at known baselines.

## Out of Scope

- umux Terminal (TUI) — v1.9.0.
- herdr importer — v2.1.0.
- Remote/SSH exposure of the socket — never local-network; remote control arrives via the SSH View (v2.5.0) and Bridge (`development` branch).
- Removing or renaming old commands away — aliases only; nothing breaks.
- A full standalone documentation site — future work; v1.8.0 grows only the README section and the landing Docs section.
- QuickType adoption — decided at /carve, not here.
