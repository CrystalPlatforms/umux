# umux v2.1.0 — PRD (herdr importer + Tabler icons + spring press feedback)

**Status:** planned. The herdr importer was v1.7.0 scope and the icon/press polish was old-v1.9.0; merged into one release on 2026-09-12 in the roadmap split. Extended the same day (afternoon) with the global accent color and custom colors (stories #139–#140, /ask discovery). Icon library changed from Lucide to Tabler at PO request.

**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md). On any conflict the master wins.

**History:** icons/press discovered 2026-09-01 ("umux v1.5.0, v1.6.0 plan" session); renumbered v1.6.0 → v1.9.0 on 2026-09-07; herdr confirmed to ship with the importer line on 2026-08-31.

## Problem Statement

herdr users, the closest audience umux has, have no import path, while cmux users already do. Meanwhile every icon is a hand-rolled SVG component (17 in `WorkspaceShell`, plus three duplicated inline close SVGs and a CSS-tricked mute bell). The set drifts from any standard and invites divergence. Press feedback is inconsistent: most buttons scale on `:active` with a plain ease-out, while menu items, tab close buttons, sidebar rows, and Settings switches give no feedback at all.

## Solution

Four packages in one release:

1. **herdr importer:** the same import wizard, reading herdr's saved session state (workspaces, tabs, panes, working directories; optionally worktree checkouts and agent sessions on explicit opt-in), mirroring the cmux wizard.
2. **Tabler icons (`@tabler/icons-react`):** closest-equivalent swap for the hand-rolled set, sourced from https://tabler.io/icons.
3. **Spring press feedback:** a springy, Apple-style press effect on **every** interactive element, as a standing rule for all future ones.
4. **Accent color and custom colors:** a **global accent** in Settings (eight palette colors, a **custom** picker, or default blue) that every accent surface follows, with color-less items automatically inheriting it; and a **Custom** entry in the Color context menu of workspaces, tabs, and groups, allowing any color from a picker beyond the fixed eight. *(Supersedes v1.5.0's fixed-palette-only rule, PO decision 2026-09-12.)*

## User Stories

*(story numbers match the master PRD)*

- **65.** As a developer moving from herdr, I want the same wizard reading herdr's saved session state (workspaces, tabs, panes, working directories; optionally worktree checkouts and agent sessions on explicit opt-in), so that switching to umux is equally painless.
- **66.** *(already shipped for cmux, applies here too)* As a user, I want importers to treat the source tool's files strictly read-only, so that umux can never damage an existing herdr setup.
- **100.** As a user, I want all app icons to come from Tabler Icons (the closest equivalents to the hand-rolled set), so that the icon language is consistent, maintained by a single library, and easy to extend.
- **101.** As a user, I want a springy Apple-style press effect on every interactive element (buttons with icons and labels, menu items, workspace/group/tab rows, switches), so that the UI feels alive and consistent. Every future interactive element ships with it. *(Standing rule, like story #84.)*
- **139.** *(added 2026-09-12)* As a user, I want a **global accent color** in Settings (one of the eight palette colors, a **custom** color from a picker, or the default blue) applied to every accent surface in the app (active edges, highlights, controls), so that umux stops being blue-only. Items **without** an explicit color automatically follow the global accent instead of a hardcoded blue.
- **140.** *(added 2026-09-12)* As a user, I want a **Custom** entry in the Color context menu of workspaces, tabs, and groups, offering any color from a picker beyond the fixed eight, so that my projects can wear exactly the color I want. *(Amends v1.5.0: the fixed-palette-only rule is superseded by the PO, 2026-09-12.)*

## Implementation Decisions

- **HerdrImporter** *(deep, pure)*: same interface and parse → plan → apply pipeline as CmuxImporter; reads herdr's saved session state (unofficial format); worktree checkouts and agent sessions import only on explicit opt-in; `from herdr` collision suffix.
- **Tabler icons** (`@tabler/icons-react`, pinned exact version, tree-shaken, MIT-licensed) replace all hand-rolled SVGs: the 17 icon components in `WorkspaceShell.tsx`, the three duplicated inline close SVGs (`SettingsDialog.tsx`, `CmuxImportWizard.tsx`, the update banner), and the mute bell, which becomes Tabler's `IconBell` / `IconBellOff` pair. Each hand-rolled icon maps to its closest Tabler equivalent, and the mapping table is committed with the PR. Size uses the `size` prop; stroke weight uses the `stroke` prop, set once as a shared default so all icons match. Where no close Tabler equivalent exists, the substitution is listed in the PR for PO review.
- **Press effect:** the existing `--press-scale` token extends to the elements that lack it (`.menu-item`, `.tab-close`, workspace/group/tab rows, Settings switches), and the easing upgrades from plain ease-out to a spring-like return per /apple-design. `prefers-reduced-motion` keeps disabling all of it.
- **Standing rule:** every future interactive element in umux ships with the press effect from day one (recorded 2026-09-01).
- **Accent model (2026-09-12):** one global setting, `accent: "default" | palette hex | custom hex`, persisted with the rest of Settings. Every surface that today hardcodes the blue accent reads the token instead. Item-color precedence stays: **item color > global accent > default blue**. A workspace/tab/group with no color renders the accent, which is the "auto" behavior and needs no new menu entry. The **Custom** picker lives as an extra entry in the existing Color context-menu submenu (workspace, tab, and group) and in Settings.
- Note: a native iOS prototype built by the PO on 2026-09-12 already carries the umux-green accent (#2F7D52). The accent token lets the desktop match it.

## Assumptions

- herdr's `session.json` is parseable and stable enough for an unofficial importer. No supported import path is documented; the format may change without notice, and fixes are best-effort, after the fact.
- Tabler's shapes differ slightly from the current set, so the icon swap is a visual change within the same metaphors, not a pixel-identical one. The PO accepts this on review.
- The icon and press changes are cosmetic only; no model changes.

## Tradeoffs Considered

- **herdr importer earlier:** rejected. Unofficial format; it ships with the importer line (re-confirmed 2026-08-31, placed in this release since 2026-09-12).
- **Strict 1:1 icon shapes:** dropped. The PO chose Tabler, whose shapes are not identical to the current set, so the target is the closest equivalent with a reviewed mapping table.
- **Keeping plain ease-out on the already-covered buttons:** rejected. One spring feel everywhere instead of two (PO choice, /apple-design).

## Validation Strategy

- **herdr import (#65):** herdr import brings in Adam's Ubuntu workspaces; herdr files untouched (checksum before/after); collisions suffixed; unit tests against committed fixture files covering the happy path, missing/extra fields, collisions, and malformed input.
- **Icons (#100):** type-check and build with `@tabler/icons-react` imports; no remaining hand-rolled `<svg>` in `src/`; all icons render in Settings, the import wizard, the update banner, and the mute button (Bell/BellOff states); the committed mapping table is reviewed by the PO, with each substitution noted.
- **Press (#101):** every menu item, row, switch, and button presses with a springy return; reduced-motion on means no animation.
- **Accent (#139):** in Settings Adam picks each of the eight palette colors, then a custom picker color; every accent surface follows immediately. With a custom workspace/tab/group color set, the item keeps its own color (precedence). Unsetting the item color returns it to the accent.
- **Custom colors (#140):** the Custom entry in the Color menu applies an arbitrary color to a workspace, a tab, and a group; choices persist across restarts; older configs (no custom colors) load unchanged.
- **HITL (Adam):** all of the above on his machine, including a side-by-side visual check of the icon swap.

## Out of Scope

- Live synchronization with herdr. Import is strictly one-time.
- Command palette and shortcut editor: v2.2.0. Native menus and pinned tabs: v2.3.0.
