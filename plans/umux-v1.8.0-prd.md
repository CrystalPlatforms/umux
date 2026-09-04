# umux v1.8.0 — PRD (agent UX & convenience + native menus)

**Status:** planned — builds after v1.7.0
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**History:** this scope was the old v1.2 "agent UX" package (formerly v1.4.0; renumbered to v1.8.0 on 2026-09-02); it moved here in the 2026-08-28 re-scope. Native menus (issue #56) were added 2026-08-29.

## Problem Statement

When several agents run at once, finished panels are easy to miss — the user hunts tab-by-tab to find who is done. Power users must memorize every shortcut because there is no way to browse or rebind actions. And umux's features are invisible to anyone who looks for them where desktop apps normally keep them: the menu bar.

## Solution

v1.8.0 makes agent attention management automatic: finished/waiting panels carry **unread markers** (ring around the panel, badge on its tab and workspace row) that clear only when viewed, plus a **jump-to-unread** shortcut. A **command palette** lists every app action, a **GUI shortcut editor** rebinds any of them, Claude Code **teammates/subagents open as native panes**, and **native menus** (macOS menu bar; ☰ button on Windows/Linux) expose every action — enforced by a standing rule so they never go stale.

## User Stories

*(story numbers match the master PRD)*

- **56.** As a developer, I want a finished or waiting panel to carry a visible marker (ring around the panel, badge on its tab and workspace row) that clears only when I view that panel, so that I never hunt tab-by-tab for who finished.
- **57.** As a developer, I want a shortcut that jumps to the most recent unviewed finished/waiting panel, so that one keypress takes me where attention is needed.
- **58.** As a user, I want a command palette (Cmd/Ctrl+Shift+P) listing all app actions, so that I can reach any function without memorizing shortcuts.
- **59.** As a user, I want to view and rebind keyboard shortcuts in Settings, so that I can replace defaults I find awkward — without editing config files.
- **60.** As a developer running Claude Code teams, I want each teammate/subagent to open as its own native pane (via the CLI/socket API), so that agent teams are visible and steerable instead of hidden background processes.
- **82.** As a macOS user, I want a native menu bar (File / Edit / View / Help) exposing the app's actions, so that umux feels at home on the Mac and features are discoverable by browsing menus.
- **83.** As a Windows/Linux user, I want a menu button (☰) next to the app title exposing the same actions, so that the same menu map exists on every platform.
- **84.** As a user, I want every new feature to ship together with its menu entry, so that the menus stay a complete, accurate map of the app instead of falling out of date. *(Standing rule for all future releases, starting with the v1.8.0 menus themselves.)*

## Implementation Decisions

- **AppMenus** — a single action registry (every app action listed once) rendered as the native menu bar on macOS and the ☰ dropdown on Windows/Linux. Building menus from the registry — not by hand — is what enforces the story #84 rule.
- **Unread markers** — derived from the same OSC-derived agent-state stream (working → finished/waiting); cleared on panel view; never from screen-content reading.
- **Command palette + shortcut editor** — both consume the same action registry (actions, labels, current bindings), so palette, menus, and the editor can never disagree.
- **Teammates as panes** — built on the v1.7.0 live CLI/socket API; a dependency on v1.7.0 shipping first.
- Menu entries for the new controls (palette toggle, jump-to-unread, marker behavior) are part of this release's own menu map — story #84 applies to itself.

## Assumptions

- The v1.7.0 socket/live API is sufficient to detect and spawn panes for Claude Code teammate/subagent processes.
- Every app action can be expressed in the registry form (id, label, handler, binding) — including future features, per the standing rule.
- Users accept that markers clear only on actually viewing the panel (not on window focus).

## Tradeoffs Considered

- **Hand-written menus** — rejected: they drift the first time a feature ships without a menu update; the registry makes omission structurally impossible.
- **Palette without a shortcut editor** — rejected: the PO wants both reachability (palette) and personalization (rebinding); they share the registry anyway.
- **Deriving "unread" from terminal output content** — rejected: unread state comes from OSC-derived agent state only, consistent with the standing detection rules.
- **Clearing markers on window focus** — rejected: passive focus would silently mark work as seen; clearing happens on viewing the panel itself.
- **Native menus earlier (in the import-&-CLI release)** — deferred: issue #56 was added 2026-08-29 and scheduled here, after the v1.7.0 API that teammates-as-panes depends on.

## Validation Strategy

- **Unread markers (story #56):** an agent finishes in a background panel → ring + badges appear; they clear only after Adam views that panel.
- **Jump-to-unread (story #57):** with several unread panels, one shortcut lands on the most recent one.
- **Palette (story #58):** the palette opens, lists every action, and executes them.
- **Shortcut editor (story #59):** Adam rebinds a default shortcut in Settings and the new binding works after restart.
- **Teammates (story #60):** a Claude Code teams session opens each teammate as its own pane; each is steerable.
- **Menus (stories #82–#84):** the macOS menu bar and Windows/Linux ☰ menu list every app action — including everything shipped in this release; a test/check verifies the registry covers every action.
- **Acceptance threshold:** all of the above pass on Adam's machine, on all three platforms for the menus.

## Out of Scope

- Background daemon — v2.0.
- Custom theming of the palette/menus beyond the app's default look — out of scope per the master PRD.
- Macro/command scripting beyond the CLI/socket surface — v2.0 plugin territory.

## Further Notes

- Build order: v1.7.0 → **v1.8.0** → v1.9.0 (kept per the master Roadmap, 2026-08-31).
- The story #84 standing rule binds every later package — v1.9.0's new controls ship with menu entries too.
