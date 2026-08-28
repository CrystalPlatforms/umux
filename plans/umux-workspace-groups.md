# Plan: Workspace groups (collapsible cmux-style sidebar tree)

> Source PRD: [issue #46](https://github.com/CrystalPlatforms/umux/issues/46) (discovery 2026-08-28, Q1–Q13)

## Architectural decisions

Durable decisions that apply across all phases:

- **Architecture style**: the in-memory workspace model extends from a flat list into a **tree of nodes** (group nodes and workspace nodes). Every sidebar mutation is a **pure state operation** (same discipline as the existing model); drag & drop, context menus and multi-select are thin bindings over it. Logic stays 100% testable without UI.
- **Data model**: every node has a stable ID, a parent (or none = top level) and a position among its siblings. Groups and top-level workspaces share **one interleaved order**. Each group additionally carries a `collapsed` flag and a `pinned` flag (pin composes at every level).
- **Key entities**: `Group` {id, name, parentId, order, collapsed, pinned} and `Workspace` (existing shape, gains a parent reference). The UI renders a **flattened, ordered list with depth** for indentation.
- **Integrations**: persistence stays in the **Rust-side store** — the serialized config gains group nodes and per-group flags. A pre-groups (flat) config loads with every workspace at top level; a corrupted file falls back to defaults (existing behavior). The **group-aware cmux importer** is pulled forward into this plan (Phase 8): source grouping maps onto this tree, a flat source imports flat. The full wizard UX (choose-what-to-import, preview) remains with the v1.2.0 import feature; herdr stays v1.3.0.
- **Hard constraints**: agent completion detection (OSC) is untouched — the group badge is a pure aggregation over existing per-panel statuses. Multi-select modifier: **Cmd on macOS, Ctrl on Linux/Windows**. New workspaces always attach at top level. Terminal output remains byte-identical.

---

## Phase 1: Two-line rows + menu cleanup

**User stories**: 5, 27

### What to build

Every workspace row in the sidebar becomes two-line: the name on the first line, the agent status chips on the second — chips must never obscure or squeeze the name, at any sidebar width. In the same pass, the workspace context menu loses "Split horizontal"/"Split vertical" (Pin / Rename / Delete remain); the tab context menu keeps its split actions unchanged. No persistence changes.

### Assumptions carried in

- Existing row rendering, chips (active-panel emphasis, minis) and ports tooltip keep working; only their layout container changes.

### Out of scope for this phase

- No groups yet — rows simply get taller.
- No changes to tab rows or panel-level chips.

### Acceptance criteria

- [ ] Row renders name and chips as two separate lines; chips wrap/scale instead of covering the name at narrow widths — [test: component suite — "row shows name line and status line"]
- [ ] Workspace context menu contains no split items; tab context menu still contains both — [test: component suite — "workspace menu items" / "tab menu items"]
- [ ] No behavioral regressions in the existing suites — [command: `npm test`]
- [ ] HITL: narrow the sidebar on macOS/Ubuntu — the workspace name stays fully readable; right-click menus match the new contents — [observable: HITL checklist]

---

## Phase 2: Groups exist and survive restart

**User stories**: 1 (partially — structure lands, filling groups comes in Phase 3), 10, 12, 14 (empty group), 26, 31

### What to build

The state model becomes a tree and the config schema grows up with it. "New group" in the sidebar creates an empty group via an inline name field (same interaction pattern as creating a workspace); groups rename via their context menu and an empty group can be deleted from the same menu. Groups render at top level alongside workspaces. The Rust store serializes and loads the new shape; **a pre-groups config loads exactly as today (all workspaces top level, no groups)** and a corrupted file still falls back to defaults. End-to-end: create a group → restart umux → it is still there.

### Assumptions carried in

- Phase 1's row layout is in place; group rows reuse the same row chrome (name line, later also a status/badge line).

### Out of scope for this phase

- No workspace can be moved into a group yet (groups are empty shells).
- No collapse/expand, no pin, no nesting.

### Acceptance criteria

- [ ] "New group" appends an empty group node at top level; rename and delete (empty) mutate the tree — [test: tree unit suite — createGroup/renameGroup/deleteGroup]
- [ ] Group rows render with their name and a context menu (Rename, Delete) — [test: component suite — "group row menu"]
- [ ] Store round-trips groups, tree order and node parents — [test: store tests — "tree config round-trips"]
- [ ] Pre-groups flat config parses to all workspaces at top level with zero groups — [test: store tests — "old flat config loads flat"]
- [ ] Corrupted config still yields defaults, no crash — [test: store tests — "corrupted config returns defaults"]
- [ ] `npm test` and `cargo test` (Tauri crate) green — [command: `npm test && cd src-tauri && cargo test`]
- [ ] HITL: create group, rename it, quit and relaunch umux — group and name survive; launch once with a pre-upgrade config — workspaces unchanged at top level — [observable: HITL checklist]

---

## Phase 3: Filing workspaces into groups

**User stories**: 6, 7, 8, 9, 11, 24, 29, 30

### What to build

Workspaces become movable within the tree. Dragging a workspace **onto** a group moves it inside (appended at the end); dragging it to **top-level space** moves it out; dragging between siblings reorders. The workspace context menu gains "Move to group…" (pick an existing group, or type a new name to create it on the fly). The "+" action keeps creating workspaces at top level only. Groups and workspaces share one draggable, interleaved order.

### Assumptions carried in

- Phase 2's tree model and persistence are live; drag infrastructure from workspace reordering is reused and extended with "drop on group" targets.

### Out of scope for this phase

- No nesting (targets are top-level groups only), no collapse, no multi-select, no pin interactions with groups.

### Acceptance criteria

- [ ] Move into group appends at the end of the target; move to top-level space restores top level; reorder inside a group preserves relative order — [test: tree unit suite — moveNode variants]
- [ ] "Move to group…" targets an existing group; typing a fresh name creates that group and files the workspace into it — [test: tree unit suite — move-to-new-group]
- [ ] "+" always attaches the new workspace at top level — [test: tree unit suite — createWorkspace stays top-level]
- [ ] Group and workspace order interleaves and survives save/load — [test: tree unit suite + store tests — "mixed order round-trips"]
- [ ] HITL: file a workspace by drag, pull it back out by drag, move one via the menu — all three paths land in the same state; restart keeps the layout — [observable: HITL checklist]

---

## Phase 4: Collapse + agent badge

**User stories**: 2, 3, 4 (flat level — subtree summing completes in Phase 5), 25

### What to build

Clicking a group toggles it collapsed/expanded in place. A collapsed group hides its children and shows a live badge `● N` — the number of its workspaces that currently have at least one active agent. Collapsed/expanded flags persist in the config, so the sidebar looks the same after a restart.

### Assumptions carried in

- Phase 3's tree with workspaces inside groups; per-panel agent statuses flow exactly as today (OSC untouched) and are exposed to the sidebar already (chips) — the badge re-reads the same data.

### Out of scope for this phase

- No nesting, so the badge counts direct children only (subtree summing lands with nesting).
- No visual difference between "agents active" and "agents idle" beyond the count.

### Acceptance criteria

- [ ] Click toggles collapse per group; state lives in the tree, not in transient UI state — [test: tree unit suite — toggleCollapse; component suite — "click toggles"]
- [ ] Badge equals the count of workspaces with ≥1 active-agent panel in the group; updates when panel statuses change — [test: aggregation unit suite — count / update]
- [ ] Collapsed flags round-trip through the store — [test: store tests — "collapsed flags round-trip"]
- [ ] HITL: start an agent in a workspace, collapse its group — badge shows 1; let the agent finish — badge clears; restart keeps the group collapsed — [observable: HITL checklist]

---

## Phase 5: Nesting + Unpack / Delete group

**User stories**: 4 (full subtree summing), 13, 14 (full), 15, 16, 28, 32

### What to build

Groups nest inside groups without a depth limit; the flattened render drives indentation. Drag safety: dropping a group into its own descendant is **rejected** — the cursor shows "not allowed" and nothing changes; hovering a collapsed group during a drag expands it after a short delay. The group context menu becomes complete: **Unpack group** (dissolve the group, its workspaces and subgroups return to top level, nothing closes) and **Delete group** (removes the group with everything inside) behind a **shared confirmation dialog** showing how many workspaces will be affected and a warning when any has a live process. The badge now sums the whole subtree. If the deleted/unpacked group held the active workspace, activation falls back per the existing workspace-delete rules.

### Assumptions carried in

- Phases 2–4's tree, persistence, drag and badge are in place; the confirmation dialog mechanics from workspace close are reusable.

### Out of scope for this phase

- No multi-select yet — the shared confirmation is built here for Delete group and gets reused by batch actions in Phase 7.
- No pin (Phase 6).

### Acceptance criteria

- [ ] Nesting to arbitrary depth renders with increasing indentation and round-trips — [test: tree unit suite — nest N levels; store tests]
- [ ] Moving a group into its own descendant is rejected; state unchanged — [test: tree unit suite — cycle rejected] and the cursor signals "not allowed" — [observable: HITL checklist]
- [ ] Hover over a collapsed group during a drag expands it after a short delay — [test: component suite — "hover expands"] — [observable: HITL checklist]
- [ ] Unpack dissolves the group; every workspace and subgroup returns to top level; no workspace closes — [test: tree unit suite — unpack preserves all]
- [ ] Delete removes the subtree; the shared confirmation lists the affected workspace count and warns on live processes — [test: tree unit suite — delete subtree; component suite — "confirmation contents"]
- [ ] Badge sums workspaces with active agents across the whole subtree — [test: aggregation unit suite — subtree sum]
- [ ] HITL: build group → subgroup → workspace, drag the parent onto its own child (nothing happens), Unpack a copy, Delete the original with a running agent and check the warning appears — [observable: HITL checklist]

---

## Phase 6: Pin per container

**User stories**: 22, 23

### What to build

Pin becomes **per container**: a pinned workspace sorts first within its own group; a pinned group sorts first at its own level. The group context menu gains Pin/Unpin (workspace pin already exists and keeps its glyph). Composes with nesting: inside a pinned group, its pinned children still lead.

### Assumptions carried in

- Phase 5's nesting and complete group menu; existing workspace pin glyph and "pinned first" ordering logic — rescope from global to per-parent.

### Out of scope for this phase

- No global pin above everything; no pinning from the multi-selection (Phase 7 wires batch pin).

### Acceptance criteria

- [ ] Pinned workspace leads only within its parent group; moving it elsewhere keeps pin local to the new container — [test: tree unit suite — per-container pin order]
- [ ] Pinned group leads at its level; nested pinned nodes still lead inside it — [test: tree unit suite — pinned group + nested pin]
- [ ] Group menu shows Pin/Unpin with correct labels; pin state round-trips — [test: component suite — "group menu pin"; store tests]
- [ ] HITL: pin a workspace inside a group and a group itself — both float to the top of their own level only — [observable: HITL checklist]

---

## Phase 7: Multi-select + batch actions

**User stories**: 17, 18, 19, 20, 21 (23 — batch pin from the selection menu)

### What to build

Cmd+click (macOS) / Ctrl+click (Linux/Windows) on workspace or group rows builds a selection — mixed sets allowed. Dragging any selected row moves the **entire selection** (into a group, to top level, or reordering); the selection context menu applies actions to all: Move to group…, Pin/Unpin all, Close all (keep workspaces) and Delete all — the destructive ones resolve to **one shared confirmation** (reuse from Phase 5). Rename is hidden for multi-selections. Escape or clicking the sidebar background clears the selection.

### Assumptions carried in

- Every single-node operation from Phases 2–6 exists as a pure operation; batch = resolving the selection to targets and applying the single op per target, with one shared confirmation up front.

### Out of scope for this phase

- No batch rename; no selection persistence across restarts; no rubber-band/marquee selection; no keyboard navigation.

### Acceptance criteria

- [ ] Selection toggles per row (add/remove), supports mixed workspaces+groups, and clears on Escape/background click — [test: selection unit suite — toggle/clear/mixed; component suite — "selection visuals"]
- [ ] Dragging one member moves every selected node in one operation; invalid/cycle members are skipped without aborting the rest — [test: tree unit suite — batch move]
- [ ] Batch destructive actions resolve to one shared confirmation listing the total affected workspaces and live-process warning; confirm applies to all — [test: component suite — "batch confirmation"; tree unit suite — batch resolve]
- [ ] Rename appears only for single selections — [test: component suite — "rename visibility"]
- [ ] `npm test` and `cargo test` green across the whole feature — [command: `npm test && cd src-tauri && cargo test`]
- [ ] HITL: select 3 workspaces + 1 group, drop them into another group; batch-delete a selection with a running agent and confirm the warning; Escape clears — [observable: HITL checklist]

---

## Phase 8: Group-aware cmux import

**User stories**: 33 (PRD #46); partially main-PRD import stories 61–64 (apply path only — full wizard UX stays v1.2.0)

### What to build

A pure importer that parses the **PO's real cmux data** into an import plan mapped onto the tree delivered by Phases 2–5. Confirmed by inspection (2026-08-28): cmux's `cmux.json` holds only workspace **definitions** (name, working directory, color, panel surfaces) and **no groups** — grouping lives in cmux's **session store** (`workspaceGroups[]` with name, collapsed, pinned + per-workspace `groupId`), with **flat groups** (no group-in-group). The importer therefore reads **both files (strictly read-only)**: definitions from the config, grouping from the session store; groups import flat, and a missing session store means a flat import. Name collisions resolve automatically with a ` from cmux` suffix; nothing is ever overwritten. A **minimal one-shot apply path** applies the plan to the live tree — no choose-what step, no preview screen (those stay with the v1.2.0 wizard). **Fixture policy:** the real cmux files stay on the PO's machine and are **never published**; tests run against a **sanitized fixture** (names/paths/UUIDs anonymized) derived from the capture, committed only after the PO approves it.

### Assumptions carried in

- The tree model, migration and persistence from Phases 2–5 are final and stable.
- A real cmux config fixture exists as a committed test input before coding starts (open item from the 2026-08-28 upgrade session).

### Out of scope for this phase

- Full import wizard UX: what-to-import choice and preview (v1.2.0 import feature).
- herdr import (v1.3.0); `umux import` CLI wiring (v1.2.0).

### Acceptance criteria

- [ ] Fixture parses into an import plan; a grouped source yields groups (nested per source), a flat source yields a flat plan — [test: importer unit suite — fixture happy path]
- [ ] Applying the plan creates the imported workspaces and groups in the live tree; colliding names get the ` from cmux` suffix; nothing overwrites — [test: tree unit suite — import apply + collision]
- [ ] The source file is never modified — [test: importer unit suite — read-only checksum proof]
- [ ] Malformed input produces a clear error and leaves state untouched — [test: importer unit suite — malformed input]
- [ ] HITL: run the minimal import against the real cmux config on the PO's Mac — workspaces and groups appear correctly and cmux's files are byte-identical before/after (checksums) — [observable: HITL checklist]

---

## Definition of done (feature)

- All eight phases merged, `npm test` and `cargo test` green, HITL checklist fully ticked on macOS + Ubuntu.
- PRD #46 stories 1–33 each covered by at least one AC above (33 via Phase 8).
- OSC-based completion detection and terminal byte streams untouched.
- OSC-based completion detection and terminal byte streams untouched.
