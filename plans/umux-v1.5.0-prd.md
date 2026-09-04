# umux v1.5.0 — PRD (planned)

**Status:** planned _(discovery 2026-09-01, "umux v1.5.0, v1.6.0 plan" session; not started)_
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Scope:** first of two small UI releases between v1.0.4 and v1.7.0 — workspace/tab/group colors, workspace-rename cleanup, port-click open.

## Problem Statement

The sidebar gives every workspace, tab, and group the same look, so with a dozen projects the user scans names only — there is no at-a-glance color coding like the one cmux users are used to. Every workspace row permanently shows a rename button that duplicates the context-menu action and adds visual noise. And when a tab's tooltip shows a listening port, clicking it only copies the URL — checking a dev server still means pasting it into a browser by hand.

## Solution

A fixed **eight-color palette** assignable from the context menu to any workspace, tab, or group. The chosen color shows as a **dot next to the name** (always visible, everywhere the name renders) and as the **left edge highlight while the item is active** (replacing today's blue accent edge; invisible when inactive). Items without a color look exactly as today. The **inline rename pencil disappears from workspace rows** — renaming stays in the context menu. **Clicking a port** in the tooltip now opens `http://localhost:{port}` in the system browser **and** still copies the URL.

## User Stories

_(story numbers match the master PRD)_

- **94.** As a user, I want to give a workspace, tab, or group one of eight fixed colors from its context menu, so that I can tell projects apart at a glance (cmux-style).
- **95.** As a user, I want the chosen color shown as a dot next to the name everywhere it renders (sidebar rows, tab bar) and as the left edge highlight while the item is active — the edge invisible when inactive, so the color is always visible without adding noise.
- **96.** As a user, I want items without a chosen color to look exactly as today (no dot, default blue active accent), so that the palette stays opt-in.
- **97.** As a user, I want colors to persist across restarts like the rest of the workspace model, so that I set them once.
- **98.** As a user, I want the inline rename pencil removed from workspace rows — rename stays available from the workspace context menu — so that the row stays clean and there is one obvious rename entry point.
- **99.** As a developer, I want clicking a port in the tooltip to open `http://localhost:{port}` in my default browser and still copy the URL, so that checking a dev server is one click instead of paste-and-go.

## Implementation Decisions

- **Model:** optional `color` field added to `Workspace`, `Tab`, and `Group` — in the TypeScript model (`src/workspaces.ts`) **and** the mirrored Rust model (`store_core`), as `Option` with skip-serializing so stores saved by older versions load unchanged. Persisted through the existing `save_workspaces` flow; no new files, no migration.
- **Palette (fixed, 8, dark-theme friendly):** light green `#4ade80`, dark green `#16a34a`, light blue `#60a5fa`, dark blue `#2563eb`, yellow `#eab308`, red `#ef4444`, pink `#ec4899`, purple `#a855f7`.
- **Color picker:** a "Color" submenu with the eight swatches in the context menu of workspace rows, tabs, and groups. Picking a color again (or a "None"/clear entry) unsets it. The swatch rows get the press feedback from the start (forward application of the v1.6.0 standing rule, story #101).
- **Presentation:** colored dot beside the name wherever the name renders (sidebar workspace/group rows, sidebar tab rows, tab bar); the active-item left edge takes the item's color instead of the default accent and is shown only while active. Unset color → today's rendering unchanged.
- **Rename cleanup:** delete the pencil button from workspace-row actions; the context menu's "Rename workspace" is the single entry point (it already calls the same `startRename` handler, so behavior is unchanged). Group rows keep their inline pencil. Tab rename (double-click / menu) is untouched.
- **Port click:** the Tauri opener plugin (new dependency: `tauri-plugin-opener` + `@tauri-apps/plugin-opener`, capability added) opens `http://localhost:{port}` in the system browser; the existing clipboard copy stays; the tooltip title becomes "Open http://localhost:{port}".

## Assumptions

- Colors are cosmetic only — no filtering, sorting, or grouping by color.
- The port URL is always `http://localhost:{port}` (no protocol detection, no non-loopback hosts); **every** port opens when clicked — a Postgres port will open a browser tab that fails to render, and that is accepted (PO decision 2026-09-01).
- Only workspace rows lose the inline rename button; group rows keep theirs.
- Stores written by v1.0.4 and earlier (no `color` fields) load unchanged — the field is optional and omitted when empty.

## Tradeoffs Considered

- **Auto-assigning colors cyclically on creation** (cmux-style) — rejected by the PO: colors are chosen manually from the palette only.
- **An exception list so non-HTTP ports only copy** — rejected: every port opens; simplicity over protocol guessing.
- **Removing rename from the context menu as well** — rejected: the menu keeps it; only the always-visible button goes.

## Validation Strategy

- **Automated:** color round-trip through the store model (TS + Rust); unset/none clears the field; the tab-bar/sidebar render list reflects dot/edge classes; `copyPort` string unchanged.
- **HITL (Adam):** assign each of the eight colors to a workspace, a tab, and a group — dot always visible, edge colored only while active; restart umux — colors persist; old configs (pre-v1.5.0) still load; workspace rows show no pencil and context-menu Rename still works; start a dev server, click its port — the browser opens `http://localhost:{port}` and the URL is on the clipboard.

## Out of Scope

- Custom/arbitrary colors, color auto-assignment, color legends or theming.
- Everything in the sibling release: lucide-react icons, the press-feedback overhaul, and the pinned-tab rebuild ([`umux-v1.6.0-prd.md`](./umux-v1.6.0-prd.md)).

## Further Notes

- Roadmap renumbered 2026-09-02: this release was planned as v1.0.5 and became **v1.5.0** so roadmap numbers continue after the shipped tags (v1.0.0–v1.0.4).
- Standing rule (story #84): the color menu gets its native-menu entry in v1.8.0 once the menu registry exists.
