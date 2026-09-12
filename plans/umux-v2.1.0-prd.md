# umux v2.1.0 — PRD (herdr importer + lucide icons + spring press feedback)

**Status:** planned — the herdr importer was v1.7.0 scope and the icon/press polish was old-v1.9.0; **merged into one release 2026-09-12** in the roadmap split; **extended 2026-09-12 (afternoon)** with the global accent color and custom colors (stories #139–#140, /ask discovery the same day).
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**History:** icons/press discovered 2026-09-01 ("umux v1.5.0, v1.6.0 plan" session); renumbered v1.6.0 → v1.9.0 on 2026-09-07; herdr confirmed to ship with the importer line on 2026-08-31.

## Problem Statement

herdr users — the closest audience umux has — have no import path, while cmux users already do. Meanwhile every icon is a hand-rolled SVG component (17 in `WorkspaceShell`, plus three duplicated inline close SVGs and a CSS-tricked mute bell) — the set drifts from any standard and invites divergence. Press feedback is inconsistent: most buttons scale on `:active` with a plain ease-out, while menu items, tab close buttons, sidebar rows, and Settings switches give no feedback at all.

## Solution

Three packages in one release:

1. **herdr importer** — the same import wizard reading herdr's saved session state (workspaces, tabs, panes, working directories — optionally worktree checkouts and agent sessions on explicit opt-in), mirroring the cmux wizard.
2. **lucide-react icons** — 1:1 swap for the hand-rolled set.
3. **spring press feedback** — a springy, Apple-style press effect on **every** interactive element, as a standing rule for all future ones.
4. **accent color & custom colors** — a **global accent** in Settings (eight palette colors / **custom** picker / default blue) that every accent surface follows, with color-less items automatically inheriting it; and a **Custom** entry in the Color context menu of workspaces, tabs, and groups — any color via a picker, beyond the fixed eight. *(Supersedes v1.5.0's fixed-palette-only rule — PO decision 2026-09-12.)*

## User Stories

*(story numbers match the master PRD)*

- **65.** As a developer moving from herdr, I want the same wizard reading herdr's saved session state (workspaces, tabs, panes, working directories — optionally worktree checkouts and agent sessions on explicit opt-in), so that switching to umux is equally painless.
- **66.** *(already shipped for cmux, applies here too)* As a user, I want importers to treat the source tool's files strictly read-only, so that umux can never damage an existing herdr setup.
- **100.** As a user, I want all app icons to come from lucide-react (1:1 replacements for the hand-rolled set), so that the icon language is consistent and maintainable.
- **101.** As a user, I want a springy Apple-style press effect on every interactive element — buttons with icons and labels, menu items, workspace/group/tab rows, switches — so that the UI feels alive and consistent; every future interactive element ships with it. *(Standing rule, like story #84.)*
- **139.** *(added 2026-09-12)* As a user, I want a **global accent color** in Settings — one of the eight palette colors, a **custom** color from a picker, or the default blue — applied to every accent surface in the app (active edges, highlights, controls), so that umux stops being blue-only. Items **without** an explicit color automatically follow the global accent (instead of a hardcoded blue).
- **140.** *(added 2026-09-12)* As a user, I want a **Custom** entry in the Color context menu of workspaces, tabs, and groups — any color from a picker, beyond the fixed eight — so that my projects can wear exactly the color I want. *(Amends v1.5.0: the fixed-palette-only rule is superseded by the PO, 2026-09-12.)*

## Implementation Decisions

- **HerdrImporter** *(deep, pure)* — same interface and parse → plan → apply pipeline as CmuxImporter; reads herdr's saved session state (unofficial format); worktree checkouts and agent sessions import only on explicit opt-in; `from herdr` collision suffix.
- **lucide-react** (tree-shaken, pinned version) replaces all hand-rolled SVGs: the 17 icon components in `WorkspaceShell.tsx`, the three duplicated inline close SVGs (`SettingsDialog.tsx`, `CmuxImportWizard.tsx`, the update banner), and the mute bell — which becomes a lucide `Bell`/`BellOff` pair. Sizes and stroke width from lucide props; visual output stays 1:1.
- **Press effect:** the existing `--press-scale` token extends to the elements that lack it (`.menu-item`, `.tab-close`, workspace/group/tab rows, Settings switches) and the easing upgrades from plain ease-out to a spring-like return per /apple-design. `prefers-reduced-motion` keeps disabling all of it.
- **Standing rule:** every future interactive element in umux ships with the press effect from day one (recorded 2026-09-01).
- **Accent model (2026-09-12):** one global setting — `accent: "default" | palette hex | custom hex` — persisted with the rest of Settings. Every surface that today hardcodes the blue accent reads the token instead. The item-color precedence stays: **item color > global accent > default blue**; a workspace/tab/group with no color renders the accent (that is the "auto" behavior — no new menu entry needed). The **Custom** picker lives as an extra entry in the existing Color context-menu submenu (all three: workspace, tab, group) and in Settings.
- Note: a native iOS prototype built by the PO on 2026-09-12 already carries the umux-green accent (#2F7D52) — the accent token lets the desktop match it.

## Assumptions

- herdr's `session.json` is parseable and stable enough for an unofficial importer (no supported import path is documented; the format may change without notice — fixes are best-effort, after the fact).
- The pin/icon changes are cosmetic-only; no model changes.

## Tradeoffs Considered

- **herdr importer earlier** — rejected: unofficial format; it ships with the importer line (re-confirmed 2026-08-31; placed in this release since 2026-09-12).
- **Icon redesign freedom while switching libraries** — rejected: strict 1:1 swap, same shapes (PO choice).
- **Keeping plain ease-out on the already-covered buttons** — rejected: one spring feel everywhere instead of two (PO choice, /apple-design).

## Validation Strategy

- **herdr import (#65):** herdr import brings in Adam's Ubuntu workspaces; herdr files untouched (checksum before/after); collisions suffixed; unit tests against committed fixture files — happy path, missing/extra fields, collisions, malformed input.
- **Icons (#100):** type-check/build with lucide imports; no remaining hand-rolled `<svg>` in `src/`; all icons render in Settings, the import wizard, the update banner, and the mute button (Bell/BellOff states).
- **Press (#101):** every menu item, row, switch, and button presses with a springy return; reduced-motion on — no animation.
- **Accent (#139):** in Settings Adam picks each of the eight palette colors, then a custom picker color — every accent surface follows immediately; with a custom workspace/tab/group color set, the item keeps its own color (precedence); unsetting the item color returns it to the accent.
- **Custom colors (#140):** the Custom entry in the Color menu applies an arbitrary color to a workspace, a tab, and a group; choices persist across restarts; older configs (no custom colors) load unchanged.
- **HITL (Adam):** all of the above on his machine.

## Out of Scope

- Live synchronization with herdr — import is strictly one-time.
- Command palette / shortcut editor — v2.2.0. Native menus + pinned tabs — v2.3.0.
