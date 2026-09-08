# Plan: umux v1.6.0 — cross-platform shell picker & sidebar polish

> Source PRD: [`plans/umux-v1.6.0-prd.md`](./umux-v1.6.0-prd.md) (stories #88–#93) · GitHub issue #67 · master PRD [`umux-prd.md`](./umux-prd.md) wins on conflict

## Architectural decisions

Durable decisions that apply across all phases:

- **Two-process Tauri model** (unchanged): Rust backend owns all OS I/O (shell probing, PTY spawn, cwd snapshots); React+TS renders picker/UI; communicate via `invoke` + events.
- **ShellDetector split**: Rust performs the *probes* — PATH scan, `/etc/shells`, login-shell from the environment, Windows registry — and returns raw probe results via a new command. A **pure TS module** receives the injected probe results and does dedup, ranking, display names, and the arrow-visibility rule (hide when ≤1 shell). The pure core is fully unit-testable in Vitest with synthetic probe fixtures; no shell is ever assumed to exist.
- **Shell selection rides the existing PTY-open parameter**: `pty_open` already accepts an optional `shell` override (currently unused by the frontend). The Settings default shell and the per-tab dropdown both pass this parameter; backend fallback chain (`override → $SHELL → passwd → /bin/sh`, hardcoded `powershell.exe` on Windows when nothing else) stays as the "auto" path. **Picker scope: local tabs on all three platforms; SSH tabs completely untouched.**
- **Settings schema** (both Rust `Settings` in store_core and the TS mirror in `src/settings.ts`, camelCase, with defaults — no migration): `defaultShell: string | null` (null = auto-detect, current behavior), `sidebarWidth: number | null` (null = CSS default), `hideTabBranch: boolean` (default **false**), `showTabFolders: boolean` (default **false**).
- **Menu entries (story #84) deferred to v1.8.0** — decided at /carve 2026-09-08: the v1.8.0 menu registry does not exist yet; all four new controls get their menu entries in one pass when it lands. Nothing menu-related in this package.
- **Windows cwd**: process-cwd tracking is not implemented on Windows (`process_cwd` returns `None`), so folder lines there show the tab's **starting directory** (already persisted in `Panel.workingDirectory`) — accepted by the PO 2026-09-08; live cwd on Windows is a separate future task.

---

## Phase 1: Default shell in Settings, end-to-end

**User stories**: #88, #90 (part)

### What to build

A thin vertical slice from Settings UI down to a spawned PTY. Backend: implement the shell probes per platform (PATH scan; on Unix also `/etc/shells` and the login shell; on Windows also registry checks) and expose them through a new command returning raw probe results. Frontend: the pure ShellDetector turns those results into a ranked, deduped list of `{displayName, launchCommand}` entries; a **custom entry** field lets the user type any command. The list lands in a new Settings section — the user picks a default shell (or "Auto"), it persists via the existing `save_settings` flow, and every subsequently opened local tab (the "+" button, the `new-tab` shortcut, restored sessions) spawns through that shell by passing it to `pty_open`. Choosing "Auto" keeps today's behavior exactly.

### Assumptions carried in

- Existing backend fallback chain and `-l`/`-NoLogo` spawn flags stay as-is for the auto path.
- Settings storage round-trip (Rust store + TS `coerceSettings`) works; only new fields added.
- Probe misses (non-standard installs) are covered by the custom entry — accepted by the PO.

### Out of scope for this phase

- No per-tab dropdown / "+" arrow (Phase 2).
- No SSH tabs (out of scope for the whole package).
- No menu entries (v1.8.0).

### Acceptance criteria

- [ ] ShellDetector pure core: ranks and dedups synthetic probe fixtures per platform, produces display name + launch command, and a "nothing found" fixture yields an empty list (custom entry is the only fallback) — [test: `src/shellDetector.test.ts`]
- [ ] Custom entry survives save/reload through `coerceSettings` and is passed through to `pty_open` verbatim — [test: `src/settings.test.ts` + `src/shellDetector.test.ts`]
- [ ] `invoke('save_settings', { settings: { defaultShell: '<picked>' } })`, app restart, new tab → PTY process is the picked shell — [observable: `echo $0` (Unix) / `$PSVersionTable` vs `cmd` banner (Windows) inside the new tab]
- [ ] Setting default back to "Auto" restores today's shell exactly — [observable: new tab matches pre-change shell]
- [ ] SSH tab still opens with its remote default shell, unaffected — [observable: open SSH tab, `echo $SHELL` remote]

---

## Phase 2: "+ New tab" arrow — per-tab shell dropdown

**User stories**: #89, #90

### What to build

A small arrow rendered next to the existing "+ New tab" button. Clicking "+" itself keeps spawning the Settings default (Phase 1); the arrow opens a dropdown listing the detected shells (plus the custom command if one is saved) and spawns exactly one new local tab in the chosen shell. The **arrow is rendered only when the detector finds more than one shell** — with zero or one detected shells the UI is byte-identical to today. Dropdown contents come from the same ShellDetector list as Settings (single source of truth).

### Assumptions carried in

- Phase 1 probes, detector, and the `pty_open` shell parameter all work end-to-end.
- The tab-creation flow (`addTab` + panel-open) accepts an optional shell without changing tab naming, layout, or restore semantics.

### Out of scope for this phase

- No per-tab *default* persistence — the dropdown affects only the tab it spawns; the Settings default is unchanged.
- No shell switching inside a running tab (spawn-time choice only).
- No menu entries (v1.8.0).

### Acceptance criteria

- [ ] Arrow-visibility rule: one detected shell → arrow not rendered; two+ → rendered — [test: pure visibility function with synthetic detector outputs, `src/shellDetector.test.ts`]
- [ ] Choosing a non-default shell from the dropdown spawns a tab running that shell — [observable: spawn a fish tab next to a bash default on Ubuntu; Git Bash + WSL on Windows; zsh vs bash on macOS]
- [ ] Clicking "+" (and the `new-tab` shortcut) still spawns the Settings default — [observable: default shell unchanged]
- [ ] SSH tabs never show the picker behavior and open exactly as before on all platforms — [observable: SSH tab as pre-change]

---

## Phase 3: Sidebar drag-resize on Windows/Linux + width persistence

**User stories**: #91

### What to build

Make the existing right-edge drag gesture actually resize the sidebar on Windows (WebView2) and Linux (WebKitGTK), not only macOS — replacing or shoring up the pointer-capture-dependent handlers with a mechanism that works across all three webviews (still bounded by the existing min width and the 75%-of-window max). The chosen width persists to the new `sidebarWidth` setting and is restored on startup; deleting/resetting falls back to the CSS default.

### Assumptions carried in

- Existing min/max bounds and the collapse gesture are kept; this phase is resize + persistence only.
- Settings storage needs no migration (new field with default).

### Out of scope for this phase

- No sidebar collapse/expand changes (works everywhere already).
- No layout redesign of the sidebar itself.

### Acceptance criteria

- [ ] On Linux (WebKitGTK) and Windows (WebView2), dragging the resizer changes the sidebar width live, respecting min/max bounds — [observable: manual drag on each platform]
- [ ] Width survives a full app restart — [observable: drag, quit, relaunch → same width]
- [ ] A saved width that is invalid/out-of-range (hand-edited settings.json) falls back to the default instead of breaking layout — [test: `coerceSettings` clamps/nulls `sidebarWidth`, `src/settings.test.ts`]
- [ ] macOS behavior unchanged — [observable: drag still works on the Mac]

---

## Phase 4: Settings switch — hide git branch on tab rows

**User stories**: #92

### What to build

A new Settings switch, **default off**, that hides the git-branch label on tab rows when enabled. Only the branch text is hidden — the ports tooltip and everything else on tab rows stay. The switch rides the existing settings save/load; the branch-refresh machinery (20s tick, Enter-triggered refresh) can keep running or be skipped when hidden, whichever is simpler, with no visible difference to the user.

### Assumptions carried in

- Phase 3 added the settings-UI pattern for a persisted toggle (same section).
- Branch data plumbing (`tabBranch.ts`, `git_branches` command) is untouched.

### Out of scope for this phase

- No changes to branch *data* fetching or the agent-status chips.
- No menu entries (v1.8.0).

### Acceptance criteria

- [ ] Default state (fresh install): branches visible, switch off — [test: `defaultSettings`/`coerceSettings` fixtures, `src/settings.test.ts`]
- [ ] Switch on → no branch label on any tab row; ports tooltip still present — [test: render WorkspaceShell with the setting on, assert `.tab-branch` absent, `src/WorkspaceShell.test.tsx` or equivalent]
- [ ] Toggle off → branches reappear without restart — [observable: flip in Settings, rows update immediately]
- [ ] The preference survives restart — [observable: relaunch with switch on → still hidden]

---

## Phase 5: Settings switch — per-tab folders on workspace rows

**User stories**: #93

### What to build

A new Settings switch, **default off**, that renders on each workspace row **one line per tab**: that tab's agent-status chip followed by the folder the tab's shell is in. Every tab gets a line (agent or not); duplicate folders are **not** merged (explicit 2026-08-31 decision — keeps tab↔folder mapping unambiguous). Data comes from the working directories umux already tracks (snapshot flow); on Windows this is the tab's starting directory (live cwd there is a separate future task). Long paths truncate so the row layout doesn't break — exact truncation (middle-ellipsis of the tail segment) decided during implementation without redesigning the row.

### Assumptions carried in

- `Panel.workingDirectory` is populated on Linux/macOS snapshots and set at spawn everywhere; Phase 3/4 established the settings-toggle pattern.
- Workspace-row layout can absorb an extra line per tab without redesign (PRD assumption).

### Out of scope for this phase

- No live cwd tracking on Windows (future task).
- No merging/grouping of duplicate folders (rejected in PRD).
- No redesign of the agent-status chip; no menu entries (v1.8.0).

### Acceptance criteria

- [ ] Default state: no folder lines anywhere, chips render exactly as today — [test: default-settings render, `src/WorkspaceShell.test.tsx` or equivalent]
- [ ] Switch on: a workspace with N tabs renders exactly N folder lines, one per tab, each pairing its chip with its folder; identical folders appear as separate lines — [test: render with a multi-tab fixture incl. duplicates, `src/WorkspaceShell.test.tsx` or equivalent]
- [ ] A tab without an agent still gets its folder line (chip shows idle/no-agent state) — [test: fixture with mixed agent states]
- [ ] Long paths don't break the row layout — [test/observable: render with a very long path fixture; row stays single-line-height with truncation]
- [ ] Preference survives restart; toggling off removes the lines immediately — [observable: flip in Settings, relaunch]
