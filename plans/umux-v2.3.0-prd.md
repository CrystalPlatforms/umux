# umux v2.3.0 — PRD (native menus + pinned tabs)

**Status:** planned — native menus were v1.8.0 scope (issue #56, added 2026-08-29) and the pinned-tab rebuild was old-v1.9.0; **merged into one release 2026-09-12** in the roadmap split.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.

## Problem Statement

umux's features are invisible to anyone who looks for them where desktop apps normally keep them: the menu bar. And pinned tabs are only a hidden flag: a pinned tab still shows the close button and can be closed — or closed from the context menu — by accident, and it doesn't hold a stable place in the tab bar.

## Solution

**Native menus** — a macOS menu bar (File / Edit / View / Help) and a ☰ menu button on Windows/Linux — rendered **from the ActionRegistry (v2.2.0)**, exposing every app action; a standing rule keeps them complete forever. This release also backfills menu entries for everything shipped since v1.6.1 in one pass. Plus the **pinned-tab rebuild**: the pin indicator replaces the close button entirely, pinned tabs cannot be closed until unpinned, and pinned tabs always sort to the front of the tab bar.

## User Stories

*(story numbers match the master PRD)*

- **82.** As a macOS user, I want a native menu bar (File / Edit / View / Help) exposing the app's actions, so that umux feels at home on the Mac and features are discoverable by browsing menus.
- **83.** As a Windows/Linux user, I want a menu button (☰) next to the app title exposing the same actions, so that the same menu map exists on every platform.
- **84.** As a user, I want every new feature to ship together with its menu entry, so that the menus stay a complete, accurate map of the app instead of falling out of date. *(Standing rule for all future releases, starting with these menus themselves.)*
- **102.** As a developer, I want a pinned tab to show a pin indicator in place of the close button (non-interactive; unpin from the context menu), so that pinning is visible at a glance.
- **103.** As a developer, I want pinned tabs to be unclosable — no close button, "Close tab" disabled in the context menu — until I unpin them, so that I never lose a pinned terminal by accident.
- **104.** As a developer, I want pinned tabs to always sit at the front of the tab bar (user order kept within the pinned and unpinned zones), so that they are always in the same place.

## Implementation Decisions

- **AppMenus** — rendered from the **ActionRegistry (v2.2.0)** as the native menu bar on macOS and the ☰ dropdown on Windows/Linux. Building menus from the registry — not by hand — is what enforces the story #84 rule.
- **Backfill:** the v1.6.1–v2.2.0 features (shell picker, sidebar switches, Agents View placement, switcher labels, palette, shortcut editor, markers/jump) get their menu entries in one pass here.
- **Pinned tabs:** `Tab.pinned` already exists and persists — no model change. When pinned, the close button is not rendered; a **non-interactive** pin indicator sits in its place (a span, not a disabled button). Unpinning stays in the context menu. "Close tab" is disabled (with a hint) while pinned. The tab-bar render list sorts pinned tabs before unpinned ones — stable within each zone; dragging a pinned tab into the unpinned zone is blocked, and dragging an unpinned tab before the pinned zone lands it after the pinned block.

## Assumptions

- The pin change affects **tabs only** — pinned workspaces and groups keep today's rendering.
- Clicking the pin indicator does nothing (PO decision 2026-09-01: indicator only, unpin via context menu).
- The unclosable rule is per-tab: closing a **whole workspace** that contains pinned tabs still works and closes them with it.
- The pinned-first sort is a render-time ordering; the saved `order` data is not rewritten by it.

## Tradeoffs Considered

- **Hand-written menus** — rejected: they drift the first time a feature ships without a menu update; the registry makes omission structurally impossible.
- **Native menus earlier** — deferred (2026-08-29 → the menu registry had to exist first; the registry lands v2.2.0 since 2026-09-12).
- **Click-to-unpin on the pin indicator** — rejected by the PO: the indicator is not a button; unpinning stays in the context menu.
- **Keeping a disabled X on pinned tabs** — rejected by the PO: the pin takes its place ("the X shouldn't exist").

## Validation Strategy

- **Menus (#82–#84):** the macOS menu bar and Windows/Linux ☰ menu list every app action — including everything shipped in v1.6.1–v2.3.0; a test/check verifies the registry covers every action.
- **Pinned tabs (#102–#104):** pin a tab — the pin icon replaces X, clicking it does nothing, context-menu "Close tab" is disabled; unpin — X returns and closing works; pinned tabs stay in front after dragging and after a restart; closing a workspace with a pinned tab inside still closes.
- **HITL (Adam):** all three platforms for the menus.

## Out of Scope

- Pinning behavior changes for workspaces and groups; a tmux-style pinned zone with separators; auto-pinning new tabs.
- Custom theming of the menus beyond the app's default look — out of scope per the master PRD.
