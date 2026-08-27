# Plan: v1.0.0 — Pane Zoom + Sidebar Tab Metadata

> Source PRD: `plans/umux-prd.md` — user stories #48 (Pane Zoom), #49 (branch on tab rows), #50 (ports in hover tooltip), added 2026-08-26. Scope decided in the /ask session of 2026-08-26 (decisions D10, D2/D15).

## Architectural decisions

Durable decisions that apply across all phases:

- **Architecture style**: no new processes or services. The features extend the existing two-process Tauri model: read-only "tab environment" queries join the existing CommandBridge `invoke` surface; the frontend stays a pure consumer. Zoom is entirely view-layer.
- **Data model**: **no persisted schema changes.** WorkspaceStore's config shape is untouched — zoom is runtime-only view state (like the existing active-tab/active-panel runtime records), and branch/ports are computed on demand, never stored.
- **Key entities**:
  - `toggle-zoom` app command — follows the existing Ctrl+Shift+letter shortcut convention (`Ctrl+Shift+Z`), **plus a UI button** in the panel chrome that toggles the same state (Adam's requirement, 2026-08-26).
  - Tab environment info (computed, per tab): git branch of the tab's focused panel's **starting working directory** (static config — live `cd` tracking explicitly deferred), and listening TCP ports of the tab's panels' process trees.
- **Integrations / OS boundary**: all OS inspection lives in the Rust backend, read-only. Git metadata is read directly from `.git` files (HEAD symbolic ref, following the worktree redirect file) — **no `git` binary is spawned**. Ports come from per-OS socket/process tables matched against panel process trees. SSH-backed panels never report local metadata (their working directory is a remote path).
- **Pull model, no background polling**: branch refreshes on UI events (tab set change, focused-panel change, panel config change); ports are fetched **only while hovering** the tab row.
- **Constraints honored (PRD)**: OSC-only completion detection untouched — these features read process/filesystem metadata, never terminal content (same class as the approved known-CLI presence check, 2026-08-25). Read-only display only; no git management (out-of-scope clarification, 2026-08-26). Zero-cost policy (no new dependencies that require paid services).

---

## Phase 1: Pane zoom (Ctrl+Shift+Z + UI button)

**User stories**: #48

### What to build

The focused panel expands to fill its entire tab area with one action — the `Ctrl+Shift+Z` shortcut or a zoom button in the panel UI — and the same action returns to the exact previous layout (identical split ratios, same panel order). While zoomed, the covered panels keep their shells running untouched (their PTYs were never touched — zoom is a view state, not a layout mutation). Closing the zoomed panel exits zoom gracefully and shows the remaining layout intact. Zoom state is per tab, runtime-only: it survives switching tabs within a session but never reaches the persisted config, so a restarted app always comes up unzoomed.

### Assumptions carried in

- The per-workspace focused-panel runtime state works (story #34); zoom always applies to the focused panel.
- The existing pure shortcut-matching module and its Ctrl+Shift+letter convention (letters n/t/h/v/w, arrows, and `Cmd+,` exception) are the baseline; `Z` is free.
- PaneLayout's split-tree geometry is the single source of layout truth; zoom renders on top of it and never mutates it.

### Out of scope for this phase

- Zoom persistence across app restarts.
- Animated transitions beyond the app's existing design language.
- Zooming into a new window / moving a panel between tabs.
- Any change to Panel/Tab/Workspace persisted shapes.

### Acceptance criteria

- [ ] With ≥2 panels, the zoom action expands the focused panel to fill the tab; the same action restores a layout identical to the pre-zoom one (same ratios) — [test: Vitest on the pure zoom-state logic; observable: HITL split 3 panels → zoom → unzoom]
- [ ] `Ctrl+Shift+Z` matches only when both modifiers are held; plain `Ctrl+Z` (shell suspend) passes through to the terminal untouched — [test: unit tests in the shortcut-matching module]
- [ ] A zoom button in the panel UI toggles the same state as the shortcut — [observable: HITL]
- [ ] Closing the zoomed panel exits zoom and the remaining panels fill the space per normal close behavior — [test: unit; observable: HITL]
- [ ] Covered panels keep running while zoomed (unzoom into a live `top` / log tail still streaming) — [observable: HITL]
- [ ] Zoom never persists: after app restart every tab renders its normal layout — [observable: HITL restart; test: config round-trip has no zoom keys]

---

## Phase 2: Git branch on tab rows

**User stories**: #49

### What to build

Each tab row in the sidebar shows a small branch label (e.g. `main`) computed from the **starting working directory of the tab's focused panel** (first panel as fallback when no explicit focus). The Rust backend answers a read-only query — directory → branch — by parsing `.git` directly: symbolic-ref from `HEAD` for a normal repo, following the `.git` file redirect for worktrees, and a short SHA for detached HEAD. No repository present → no label at all (the row stays clean; no placeholder, no error). SSH-backed panels never show a branch (their working directory is remote). The label refreshes when the tab set changes, the focused panel changes, or a panel's configured directory changes — there is no timer and no filesystem watching.

### Assumptions carried in

- Panels carry a persisted optional working-directory config; tabs own split trees whose leaves are panel ids (#37 model).
- The CommandBridge invoke surface and sidebar tab rows exist and are extended, not replaced.
- Phase 1's precedent that runtime-only additions leave the persisted config untouched.

### Out of scope for this phase

- Live tracking of `cd` inside the shell (deferred — a possible post-1.0 enhancement; decided against for v1.0.0, 2026-08-26).
- Branch display for SSH panels.
- Any git write operations, status, diffs, ahead/behind counts.
- Ports (Phase 3).

### Acceptance criteria

- [ ] Given fixture `.git` layouts, the backend returns: branch name for a normal repo; the redirect target's branch for a worktree; a short SHA for detached HEAD; "none" for a directory without `.git` — [test: cargo unit tests on the pure parser]
- [ ] A tab whose focused panel's directory is a repo shows the branch; switching focused panel within the tab updates the label — [test: Vitest on the tab→label mapping; observable: HITL in the umux repo → `main`]
- [ ] Non-repo directories and SSH panels show no branch label and produce no error — [test: unit; observable: HITL]
- [ ] Works with Windows and macOS path forms — [observable: HITL on Adam's Mac and Windows machine]

---

## Phase 3: Listening ports in the tab tooltip

**User stories**: #50

### What to build

Hovering a tab row in the sidebar fetches (on hover, never on a timer) that tab's listening TCP ports and shows them in a tooltip, e.g. `3000 · 5173`. The backend enumerates listening sockets together with owning PIDs (per-OS mechanism), matches those PIDs against the process trees rooted at each panel's shell, and aggregates per tab — a dev server started as a grandchild of the shell counts; unrelated system processes never do. A tab with no listeners shows an explicit empty state ("No listening ports") so the tooltip's silence is never ambiguous. Killing the server and hovering again drops the port.

### Assumptions carried in

- The backend owns the panel shell processes and can enumerate their descendants.
- The read-only-query + pull-on-hover pattern established in Phase 2.
- Tooltip presentation follows the app's existing design language (Apple-design pass).

### Out of scope for this phase

- Continuous background polling or live-updating tooltips (hover pulls only — zero background cost while unused).
- UDP sockets; IPv6-vs-IPv4 distinction (deduplicate by port number).
- Process names next to ports; clicking a port to open it (browser pane is v2.0).

### Acceptance criteria

- [ ] Given fixture socket/PID tables, the pure matcher assigns ports to the correct panel process tree: direct children and grandchildren count, unrelated PIDs are excluded, and multiple panels' ports stay separated per tab — [test: cargo unit tests]
- [ ] Running a server in a panel (`python3 -m http.server 8000`) then hovering the tab shows `8000`; after killing it, the next hover no longer lists it — [observable: HITL]
- [ ] Hovering a tab with no listeners shows the explicit "No listening ports" state — [observable: HITL]
- [ ] No backend port queries occur while nothing is hovered — [test: Vitest with a fake invoke bridge counting calls]
- [ ] Port enumeration works on Linux, macOS, and Windows — [observable: HITL per platform; unit tests on per-OS parsing where pure]
