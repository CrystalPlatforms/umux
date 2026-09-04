# umux v1.6.0 — PRD (planned)

**Status:** planned _(discovery 2026-09-01, "umux v1.5.0, v1.6.0 plan" session; follows v1.5.0; not started)_
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Scope:** second of two small UI releases between v1.0.4 and v1.7.0 — lucide-react icons, spring press feedback everywhere, pinned-tab rebuild.

## Problem Statement

Every icon is a hand-rolled SVG component (17 in `WorkspaceShell`, plus three duplicated inline close SVGs and a CSS-tricked mute bell) — the set drifts from any standard and invites divergence. Press feedback is inconsistent: most buttons scale on `:active` with a plain ease-out, while menu items, tab close buttons, sidebar rows, and Settings switches give no feedback at all. And pinned tabs are only a hidden flag: a pinned tab still shows the close button and can be closed — or closed from the context menu — by accident, and it doesn't hold a stable place in the tab bar.

## Solution

Switch all icons to **lucide-react** (1:1 — the hand-rolled set already mimics Lucide geometry). Give **every interactive element** a springy, Apple-style press effect — labeled buttons, icon buttons, menu items, tab close, workspace/group/tab rows, switches — and make it a standing rule for all future ones. **Rebuild pinned tabs:** the pin indicator replaces the close button entirely, pinned tabs cannot be closed until unpinned, and pinned tabs always sort to the front of the tab bar.

## User Stories

_(story numbers match the master PRD)_

- **100.** As a user, I want all app icons to come from lucide-react (1:1 replacements for the hand-rolled set), so that the icon language is consistent and maintainable.
- **101.** As a user, I want a springy Apple-style press effect on every interactive element — buttons with icons and labels, menu items, workspace/group/tab rows, switches — so that the UI feels alive and consistent; every future interactive element ships with it. _(Standing rule, like story #84.)_
- **102.** As a developer, I want a pinned tab to show a pin indicator in place of the close button (non-interactive; unpin from the context menu), so that pinning is visible at a glance.
- **103.** As a developer, I want pinned tabs to be unclosable — no close button, "Close tab" disabled in the context menu — until I unpin them, so that I never lose a pinned terminal by accident.
- **104.** As a developer, I want pinned tabs to always sit at the front of the tab bar (user order kept within the pinned and unpinned zones), so that they are always in the same place.

## Implementation Decisions

- **lucide-react** (tree-shaken, pinned version) replaces all hand-rolled SVGs: the 17 icon components in `WorkspaceShell.tsx`, the three duplicated inline close SVGs (`SettingsDialog.tsx`, `CmuxImportWizard.tsx`, the update banner), and the mute bell — which becomes a lucide `Bell`/`BellOff` pair instead of the CSS strike-through. Sizes and stroke width come from lucide props; visual output stays 1:1 where lucide has an identical shape.
- **Press effect:** the existing `--press-scale` token is extended to the elements that lack it (`.menu-item`, `.tab-close`, workspace/group/tab rows, Settings switches) and the easing is upgraded from plain ease-out to a spring-like return per /apple-design, so every element presses and releases with the same feel. `prefers-reduced-motion` continues to disable all of it (already wired).
- **Standing rule:** recorded in Claude's MEMORY and as story #101 — every future interactive element in umux ships with the press effect from day one.
- **Pinned tabs:** `Tab.pinned` already exists and persists — no model change. When pinned, the close button is not rendered; a **non-interactive** pin indicator sits in its place (a span, not a disabled button). Unpinning stays in the context menu. "Close tab" is disabled (with a hint) while pinned. The tab-bar render list sorts pinned tabs before unpinned ones — stable within each zone; dragging a pinned tab into the unpinned zone is blocked, and dragging an unpinned tab before the pinned zone lands it after the pinned block.

## Assumptions

- The pin change affects **tabs only** — pinned workspaces and groups keep today's rendering.
- Clicking the pin indicator does nothing (PO decision 2026-09-01: indicator only, unpin via context menu).
- The unclosable rule is per-tab: closing a **whole workspace** that contains pinned tabs still works and closes them with it.
- The pinned-first sort is a render-time ordering; the saved `order` data is not rewritten by it.

## Tradeoffs Considered

- **Click-to-unpin on the pin indicator** — rejected by the PO: the indicator is not a button; unpinning stays in the context menu.
- **Keeping a disabled X on pinned tabs** — rejected by the PO: the pin takes its place ("the X shouldn't exist").
- **Icon redesign freedom while switching libraries** — rejected: strict 1:1 swap, same shapes (PO choice).
- **Keeping plain ease-out on the already-covered buttons** — rejected: one spring feel everywhere instead of two (PO choice, /apple-design).

## Validation Strategy

- **Automated:** pinned-first ordering of the tab-bar render list (pure function test); close-blocked logic (pinned tab produces no close path); type-check/build with lucide imports; no remaining hand-rolled `<svg>` in `src/`.
- **HITL (Adam):** pin a tab — the pin icon replaces X, clicking it does nothing, context-menu "Close tab" is disabled; unpin — X returns and closing works; pinned tabs stay in front after dragging and after a restart; close a workspace with a pinned tab inside — still closes; every menu item, row, switch, and button presses with a springy return; reduced-motion on — no animation; all icons render in Settings, the import wizard, the update banner, and the mute button (Bell/BellOff states).

## Out of Scope

- Pinning behavior changes for workspaces and groups; a tmux-style pinned zone with separators; auto-pinning new tabs.
- Everything in the sibling patch: colors, port-click open, rename cleanup ([`umux-v1.5.0-prd.md`](./umux-v1.5.0-prd.md)).

## Further Notes

- The press-effect standing rule was saved to Claude's persistent MEMORY on 2026-09-01 and applies to all future umux work, not only v1.6.0.
- Standing rule (story #84): the pinned-tab actions keep their context-menu entries; their native-menu entries arrive with the v1.8.0 menu registry.
