# umux v2.0.0 — PRD (Agents View, unread markers & the sidebar view switcher)

**Status:** planned — was the core of v1.8.0 ("agent UX & convenience"); **renumbered 2026-09-12** into its own release after the TUI (v1.9.0). The Agents View stories were added 2026-09-08; the placement setting (#135) and the sidebar switcher (#130) come from the 2026-09-12 SSH View discovery.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Note:** the *Ecosystem* PRD lives on the `development` branch (as `umux-v3.0.0-prd.md` there); this file is the main-branch extract for roadmap v2.0.0.

## Problem Statement

When several agents run at once, finished panels are easy to miss — the user hunts tab-by-tab to find who is done. The agents themselves are invisible as a group: there is no single place that says "these three CLIs are running right now, here is where each one lives". And as umux grows three sidebar surfaces (workspaces, agents, SSH), the sidebar needs one obvious structure instead of an ever-longer single list.

## Solution

Finished/waiting panels carry **unread markers** (ring around the panel, badge on its tab and workspace row) that clear only when viewed, plus a **jump-to-unread** shortcut. The **Agents View** lists every currently running agent — as a docked split at the bottom of the sidebar *or* as a separate full view, per a Settings choice. And the sidebar gets a **view switcher — [Local][Agents][SSH]** — with **renameable tab labels**; the SSH tab itself arrives in v2.5.0.

## User Stories

*(story numbers match the master PRD)*

- **56.** As a developer, I want a finished or waiting panel to carry a visible marker (ring around the panel, badge on its tab and workspace row) that clears only when I view that panel, so that I never hunt tab-by-tab for who finished.
- **57.** As a developer, I want a shortcut that jumps to the most recent unviewed finished/waiting panel, so that one keypress takes me where attention is needed.
- **128.** *(added 2026-09-08)* As a developer running several AI CLIs at once, I want an **Agents View** listing every currently running agent as one row showing its agent-status chip, its folder, and its tab name, so that I see all running agents at a glance. Entries appear the moment a CLI agent (Claude Code, Codex, Grok, …) is detected and disappear the moment it exits; ordering is by most recent signal, newest first.
- **129.** *(added 2026-09-08)* As a user, I want clicking an Agents View row to take me straight to that agent — expand its group if collapsed, switch to its workspace, activate its tab, and focus its panel, also clearing that panel's unread badge (story #56) — while the section is a collapsed dark **AGENTS** header whose chevron is visible only when agents are running; **local panels only** for now (SSH later).
- **135.** *(added 2026-09-12)* As a user, I want the Agents View placement in Settings — **off by default** (story #129 stands) / docked split at the bottom of the Local view / a separate full **[Agents]** tab — so that I choose how agents surface; the [Agents] tab appears in the switcher only in the separate-tab mode.
- **130.** *(added 2026-09-12)* As a user, I want a view switcher at the top of the sidebar — **[Local][Agents][SSH]** — where Local is today's workspace view, Agents lists every running agent, and SSH manages remote machines (from v2.5.0), so that the three surfaces share one sidebar. *(Names are defaults only — every tab label can be renamed in place and the custom names persist.)*

## Implementation Decisions

- **Unread markers** — derived from the same OSC-derived agent-state stream (working → finished/waiting); cleared on panel view; never from screen-content reading.
- **Agents View detection** *(decided 2026-09-08)* — membership comes from the same OSC-derived agent-state stream plus PTY lifetime (panel gets an exit signal / tab closed), staying within the "no process polling" hard constraint. **Alternative noted:** OS-level process detection would catch agents more literally but breaks the standing constraint — revisit only if OSC-based membership proves insufficient.
- **Docked layout** *(decided 2026-09-08)* — in the split mode the section sits at the bottom of the sidebar behind a vertical (up-down) drag-resizable split; rows sorted by most recent agent signal, newest first; each row = agent-status chip + folder + tab name; the collapsed dark **AGENTS** header shows its chevron only while agents are running (no EmptyState when the list is empty). Local panels only; SSH later.
- **Placement setting (#135, 2026-09-12)** — one Settings choice: *off (default)* / *docked split* / *separate tab*. In the separate-tab mode the section renders as a full view behind the **[Agents]** switcher tab; in the docked mode the [Agents] tab is not shown; with *off* neither appears and the sidebar is exactly today's.
- **Switcher labels (#130)** — the three tab labels ("Local", "Agents", "SSH") are defaults; each can be renamed in place and the custom names persist in settings.
- **Menu entries** — per the story #84 standing rule, all new controls get registry-driven menu entries when the menu registry lands (v2.3.0).

## Assumptions

- Users accept that markers clear only on actually viewing the panel (not on window focus).
- The OSC/PTY-based detection is sufficient to list running agents without process polling.

## Tradeoffs Considered

- **Deriving "unread" from terminal output content** — rejected: unread state comes from OSC-derived agent state only, consistent with the standing detection rules.
- **Clearing markers on window focus** — rejected: passive focus would silently mark work as seen; clearing happens on viewing the panel itself.
- **Agents View via OS process detection** — rejected for now (breaks the "no process polling" constraint); noted as the fallback.
- **A single fixed placement** — superseded 2026-09-12: the PO chose the three-way Settings choice (off / split / separate tab) over the original single switch.

## Validation Strategy

- **Unread markers (#56):** an agent finishes in a background panel → ring + badges appear; they clear only after Adam views that panel.
- **Jump-to-unread (#57):** with several unread panels, one shortcut lands on the most recent one.
- **Agents View (#128–#129):** in the docked mode, launching `claude` in a background tab makes a row appear under AGENTS within a moment; clicking it lands in that exact workspace/tab/panel and clears its unread badge; exiting the CLI removes the row; with several agents, newest signal is on top; dragging the split changes the section's height and persists.
- **Placement (#135):** switching between off / split / separate tab changes only what the setting promises; the default leaves the sidebar exactly as before.
- **Switcher (#130):** labels rename and persist across restarts; the [SSH] tab is present but shows the coming-soon state until v2.5.0 (or ships hidden until then — decided at /carve).

## Out of Scope

- Agents View for **SSH/remote panels** — deferred (2026-09-08: local panels only for now).
- The SSH view itself — v2.5.0.
- Background daemon — v1.7.0. Command palette and shortcut editor — v2.2.0. Native menus — v2.3.0.
