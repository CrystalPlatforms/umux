# umux — Product Requirements Document (PRD)

**Project:** umux
**Type:** Open-source desktop application
**Platform:** Linux (Ubuntu/Wayland) + Windows + macOS
**Stack:** Tauri v2 (Rust backend) + React + TypeScript (frontend)
**Status:** v1.2.0 scope fully shipped (as tags v1.0.2–v1.0.4, latest 2026-08-31); roadmap reshuffled 2026-08-28 after a new discovery session (import from cmux/herdr, `umux` CLI, umux Terminal TUI); extended 2026-08-29 with in-app updates (issue #55, v1.2.0) and native menus (issue #56, v1.4.0); extended 2026-08-31 after the "versions cleanup" discovery — v1.3.0 confirmed (Ctrl+B prefix, release gated on the TUI working on all three platforms, full live-CLI parity, herdr importer stays) and a new v1.5.0 package created (issue #67 + sidebar/metadata polish) — see Roadmap

---

## Problem Statement

Power users and developers accumulate many terminal windows across several projects over the course of a workday. Each project may need multiple shells — a build process, log tails, a local server, an AI coding assistant — and these windows quickly become scattered across virtual desktops or buried in a long window list. There is no lightweight, project-oriented way to group terminals into persistent, switchable collections, so users lose context, waste time hunting for the right window, and end up with a cluttered, hard-to-navigate terminal setup.

Existing tools each solve only part of the problem. Terminal multiplexers (tmux, screen) provide splitting but live entirely inside a single terminal window, do not offer a native desktop experience, and are intimidating for beginners. Newer agent-era multiplexers (e.g. herdr) add real power on top of the tmux model — agent status, persistent session servers, plugins — but keep the terminal-native, keybinding-first learning curve. Tiling terminals (Tilix, Terminator) offer pane splitting but lack the concept of named, persistent project workspaces that survive restarts. None of them watch the terminal stream to notify the user when a long-running AI CLI task (such as Claude Code or Aider generating code) has finished, so the user must either keep watching the window or periodically check back.

## Solution

umux is an open-source terminal workspace manager for **Linux, Windows, and macOS**. It is a single native desktop application that contains its own built-in terminal (Approach A). Users organize their terminals into named **workspaces**, where each workspace typically corresponds to a project. Within a workspace, the terminal area can be split into **any number of resizable panels** (drag the dividers). The application also inspects the terminal byte stream for completion signals emitted by AI CLI tools — which emit standard OSC 9;9 / OSC 99 / OSC 777 escape sequences — and fires a native desktop notification when such a task finishes, so the user can step away while an AI generates code.

The Rust backend owns the pseudoterminals (PTY), SSH connections, the OSC parser, and the desktop notification bridge. The React + TypeScript frontend renders the terminal surface and the workspace/pane user interface.

---

## User Stories

### Workspaces
1. As a developer, I want to create a new named workspace, so that I can group terminals for a single project.
2. As a developer, I want to switch between workspaces quickly, so that I can context-switch between projects without losing my terminals.
3. As a developer, I want to rename a workspace, so that the label reflects the current project.
4. As a developer, I want to delete a workspace, so that I can clean up projects I no longer work on.
5. As a developer, I want to close a workspace without deleting its definition, so that I can temporarily clear my screen while keeping the project configured.
6. As a developer, I want to see a list of all my workspaces with their names, so that I can find the right one at a glance.
7. As a developer, I want to reorder workspaces, so that my most-used projects are easiest to reach.
8. As a developer, I want my workspaces to persist across application restarts, so that I do not have to rebuild my setup every time I reopen umux.

### Terminals (PTY)
9. As a developer, I want each new panel to open an interactive shell, so that I can run commands immediately.
10. As a developer, I want the terminal to support common shell features (colors, cursor movement, alternate screen), so that tools like vim, htop, and fzf render correctly.
11. As a developer, I want to type into a panel and see my keystrokes sent to the correct shell, so that input always goes to the panel I am looking at.
12. As a developer, I want a panel to resize its terminal dimensions when its on-screen size changes, so that line wrapping and full-screen apps adapt correctly.
13. As a developer, I want to close a panel and have its underlying shell process terminated cleanly, so that no orphan processes are left behind.
14. As a developer, I want to choose which shell is launched (defaulting to the user's `$SHELL`), so that my preferred shell configuration is respected.

### Pane Layout
15. As a developer, I want to split a single panel into two, so that I can watch a log next to the command I am running.
16. As a developer, I want to split into as many panels as I need (no hard limit), so that complex projects get the layout they need while simple setups stay simple.
17. As a developer, I want to split either horizontally or vertically, so that I can arrange panels to fit my task.
18. As a developer, I want to drag the divider between two panels to resize them, so that I can give more room to the panel I am focused on.
19. As a developer, I want the divider to snap to a sensible minimum size on each side, so that a panel never collapses to zero.
20. As a developer, I want to close a panel and have the neighboring panels fill the space, so that the layout stays clean after closing.

### SSH
21. As a developer, I want to open a panel connected to a remote machine over SSH, so that I can work on a server from inside the same workspace.
22. As a developer, I want to enter SSH connection details (host, user, port), so that I can connect to arbitrary remote hosts.
23. As a developer, I want umux to authenticate using my local SSH agent and keys, so that I do not have to re-enter passwords or copy keys into the app.
24. As a developer, I want to see a clear error when an SSH connection fails, so that I can diagnose the problem.
25. As a developer, I want an SSH-backed panel to behave like a local panel (input, output, resize), so that the experience is consistent regardless of where the shell runs.

### Notifications (AI CLI completion)
26. As a developer, I want umux to show a desktop notification when an AI CLI tool (such as Claude Code) finishes generating, so that I can step away and be alerted the moment it is done.
27. As a developer, I want the notification to work automatically without configuring the AI tool, so that completion detection "just works."
28. As a developer, I want the notification to include a short message (and the originating workspace/panel where available), so that I know which task finished.
29. As a developer, I want umux to ignore escape sequences that are not completion signals, so that normal terminal output is never altered or blocked.
30. As a developer, I want to be able to mute notifications temporarily, so that a busy session does not spam my desktop.

### Application & UX
31. As a new user, I want a clear, empty state that tells me how to create my first workspace, so that I am never stuck on a blank screen.
32. As a user, I want the application window to be resizable, so that it fits my monitor and workflow.
33. As a user, I want keyboard shortcuts for common actions (new workspace, switch workspace, split, close panel), so that I can move fast without the mouse.
34. As a developer, I want focus to move clearly to the panel I click, so that it is always obvious where my keystrokes will go.
35. As a user, I want the UI to remain responsive while a long-running command produces output, so that the app never freezes.

### Reliability & Persistence
36. As a developer, I want the application to recover gracefully if a single shell crashes, so that the rest of my workspaces keep running.
37. As a developer, I want panel layouts and working directories to be saved, so that reopening a workspace restores a sensible starting state.
38. As a developer, I want a corrupted config file to fall back to defaults instead of crashing the app, so that a bad write never blocks startup.

### Agent Status (AI CLI awareness) — v0.2.0
39. As a developer, I want each panel to show a live status indicator (working / waiting for me / idle), derived from the OSC activity stream, so that I can see at a glance which agent needs attention.
40. As a developer, I want the status indicator to be accurate (no false "done" states), so that I never walk away from an agent that is still working.
41. As a developer, I want to turn the status indicators off in Settings, so that the UI stays minimal if I do not want them.

### Settings & Feature Toggles — v0.2.0
42. As a user, I want a Settings screen with on/off switches for optional features (agent status, notifications), so that umux adapts to how I work.
43. As a user, I want my settings to persist across restarts, so that I do not reconfigure umux every launch.

### Session Restore — v0.2.0
44. As a developer, I want umux to restore my workspaces, panels, layout, working directories, and shells when I reopen the app, so that a restart costs me seconds, not minutes. (Running processes are not resumed — see Roadmap v2.0 for a background server.)
45. As a developer, I want restored shells to open in the correct working directory, so that context is preserved.

### Safe Closing — v0.2.0
46. As a developer, I want closing a panel that has a running process to always ask for confirmation, and closing an idle panel to never ask, so that the behavior is predictable and I never lose agent work by accident.

### Privacy-friendly Analytics — v0.2.0
47. As the maintainer, I want a single anonymized event via Aptabase (app open only — install/usage counting; free tier, official Tauri SDK; no Settings switch — always on, with a kill-switch flag in settings.json for a full opt-out), so that real usage is measured instead of guessed from downloads.

### Pane Zoom — v1.0.0
48. As a developer, I want to zoom the focused panel to fill its tab and, with the same shortcut, return to the exact previous layout (other panels keep running untouched), so that I can read one agent's output closely without disturbing the rest.

### Sidebar Metadata — v1.0.0
49. As a developer, I want each tab row in the sidebar to show the git branch of that tab's working directory (read-only display), so that I always know which branch each terminal is on at a glance.
50. As a developer, I want each tab to surface the ports its shells are listening on (shown in a hover tooltip, not permanently), so that I can find a dev server without hunting through panels.

### Scriptability (CLI + Socket API) — v1.2.0 & v1.3.0 (re-scoped 2026-08-28)
51. As a developer, I want a `umux` CLI that ships with the app, so that I can drive umux from scripts, shells, and tool hooks. *(v1.2.0)*
52. As a developer, I want `umux notify "text"` to raise a desktop notification without the app running, so that tools without OSC support (or explicit hooks) can alert me. *(v1.2.0)*
53. As a developer, I want `umux list` / `umux status` to print workspaces, tabs, panels, and agent states as JSON, so that scripts and agents can inspect the current setup. *(v1.3.0 — live)*
54. As a developer, I want control commands (`umux new-workspace`, `umux new-tab`, `umux split`, `umux send`), so that external tooling can build and drive layouts. *(v1.3.0 — live)*
55. As an AI agent, I want the same surface exposed over a local socket API, so that I can orchestrate umux programmatically without parsing CLI text. *(v1.3.0)*
87. As a developer, I want the live CLI/socket surface to cover **everything the running app can do** — every workspace, group, tab, pane, split, rename, send, and settings action, not a curated subset — so that anything achievable in the UI is scriptable live. *(v1.3.0 — full parity confirmed 2026-08-31)*

*(Note: v1.2.0 ships the offline subset — notify plus the definition CRUD below; v1.3.0 adds the live surface through the running app's local socket at full parity with the app (2026-08-31). Headless/daemon operation remains v2.0.)*

### Agent UX & Convenience — v1.4.0 (moved from the old v1.2, 2026-08-28)
56. As a developer, I want a finished or waiting panel to carry a visible marker (ring around the panel, badge on its tab and workspace row) that clears only when I view that panel, so that I never hunt tab-by-tab for who finished.
57. As a developer, I want a shortcut that jumps to the most recent unviewed finished/waiting panel, so that one keypress takes me where attention is needed.
58. As a user, I want a command palette (Cmd/Ctrl+Shift+P) listing all app actions, so that I can reach any function without memorizing shortcuts.
59. As a user, I want to view and rebind keyboard shortcuts in Settings, so that I can replace defaults I find awkward — without editing config files.
60. As a developer running Claude Code teams, I want each teammate/subagent to open as its own native pane (via the CLI/socket API), so that agent teams are visible and steerable instead of hidden background processes.

### Native Menus — v1.4.0 (issue #56, added 2026-08-29)
82. As a macOS user, I want a native menu bar (File / Edit / View / Help) exposing the app's actions, so that umux feels at home on the Mac and features are discoverable by browsing menus.
83. As a Windows/Linux user, I want a menu button (☰) next to the app title exposing the same actions, so that the same menu map exists on every platform.
84. As a user, I want every new feature to ship together with its menu entry, so that the menus stay a complete, accurate map of the app instead of falling out of date. *(Standing rule for all future releases, starting with the v1.4.0 menus themselves.)*

### Import from cmux / herdr — v1.2.0 (cmux) & v1.3.0 (herdr) — added 2026-08-28

> Update 2026-08-28: the **group-aware cmux importer** (parsing + a minimal one-shot import path) is pulled forward into the workspace-groups feature (issue #46) — it cannot be built without the tree model that feature delivers. The full wizard UX (what-to-import choice, preview) remains v1.2.0; herdr stays v1.3.0. Implementation is blocked on capturing Adam's real `~/.config/cmux/cmux.json` as a fixture.
61. As a developer moving from cmux, I want a one-time import wizard in Settings, so that my existing workspaces appear in umux without rebuilding them by hand.
62. As a developer, I want to choose what gets imported (workspace names and order, grouping, working directories, tabs and panel layout), so that I bring only what I need. Grouping present in the source tool is recreated as umux workspace groups; a flat source imports flat (decided 2026-08-28, workspace-groups feature).
63. As a developer, I want a preview of the import plan before anything is written, so that I see exactly which workspaces will be created.
64. As a developer, I want name collisions resolved automatically with a `from cmux` suffix (respectively `from herdr`), so that an import can never silently overwrite my existing workspaces.
65. As a developer moving from herdr (v1.3.0), I want the same wizard reading herdr's saved session state (workspaces, tabs, panes, working directories — optionally worktree checkouts and agent sessions on explicit opt-in), so that switching to umux is equally painless.
66. As a user, I want importers to treat the source tool's files strictly read-only, so that umux can never damage an existing cmux/herdr setup.

### umux CLI (offline) — v1.2.0 — added 2026-08-28
67. As a developer, I want the desktop installers (NSIS, DMG, .deb) to put a `umux` binary on PATH (AppImage limitation documented), so that the CLI works in every terminal without extra setup.
68. As a developer, I want `umux` with no arguments to print help listing every command, so that I can discover the CLI without opening docs.
69. As a developer, I want `umux list --desk` to print the saved workspaces, tabs, and panels of the desktop store, so that I can inspect definitions without launching the app.
70. As a developer, I want `umux new/rm/rename/split` to manage workspaces and edit panel layouts in the saved definitions, so that umux is fully manageable from a plain terminal.
71. As a developer, I want `umux import cmux` and `umux export` mirroring the wizard, so that imports and exports are scriptable.
72. As a developer, I want `umux config get/set` (including the default-launch-mode setting), so that I can change settings without the GUI.
73. As a developer, I want store-targeting commands to require `--desk/--desktop` or `--term/--terminal` and to print a hint when the flag is missing, so that I never touch the wrong store by accident.
74. As a developer, I want all CLI writes to go through the same store library the app uses, so that CLI and desktop can never corrupt each other's files.

### Auto-Update — v1.2.0 (issue #55, added 2026-08-29)
85. As a user, I want umux to check for updates and tell me when a new version is available (in Settings, and on startup), so that I never run an old version without knowing.
86. As a user, I want to download and apply the update from inside the app — GitHub Releases as the source — so that updating is one click instead of a manual reinstall.

### umux Terminal (TUI) — v1.3.0 — added 2026-08-28
75. As a developer, I want `umux --term` (or `--terminal`) to launch umux Terminal — a full TUI with sidebar, tabs, and unlimited panes — so that I can use umux inside any terminal, over SSH, or on a headless machine.
76. As a developer, I want tmux-style prefix shortcuts plus mouse support in the TUI, so that panel management matches multiplexer conventions. *(Prefix key: Ctrl+B — confirmed 2026-08-31.)*
77. As a developer, I want agent status (working / waiting / idle) shown in the TUI sidebar and panel titles, derived from the same OSC detection as the desktop app, so that agent awareness is identical in both modes.
78. As a Terminal-first user, I want a setting (in desktop Settings and via `umux config set`) that makes plain `umux` launch Terminal instead of printing help, so that I skip a keystroke every time.
79. As a developer, I want Desktop and Terminal to keep separate saved states, so that neither mode surprises the other.
80. As a developer, I want export/import between the Desktop and Terminal states (CLI commands and UI buttons), so that I can move my setup between modes in both directions.
81. As a developer, I want the v1.3.0 release to ship only once umux Terminal works on **all three platforms** — Linux, macOS, and Windows (ConPTY) — so that no platform receives a half-finished TUI. *(Changed 2026-08-31: the old "Linux + macOS first, Windows later" split was rejected by the PO.)*

### Shell Picker — Windows — v1.5.0 (issue #67, added 2026-08-31)

> Standing rule from story #84: each v1.5.0 feature below ships with its menu entry once the v1.4.0 menu registry exists.

88. As a Windows user, I want to pick my default shell in Settings from an auto-detected list (PowerShell, cmd, Git Bash, WSL) or enter a custom command, so that new tabs open in the shell I actually use.
89. As a Windows user, I want an arrow next to the "+ New tab" button that opens a dropdown for choosing the shell of that specific new tab, so that I can spawn, say, a WSL tab without changing my default.
90. As a user, I want the shell picker to affect local tabs only — SSH tabs and macOS/Linux keep today's behavior — so that remote sessions stay predictable. *(Windows-only by decision, 2026-08-31.)*

### Sidebar Resize — all platforms — v1.5.0 (added 2026-08-31)

91. As a Windows/Linux user, I want the sidebar's right-edge drag to resize it — the gesture that already works on macOS — and I want the chosen width to persist across restarts, so that my layout survives a reboot on every platform.

### Metadata Switches — v1.5.0 (added 2026-08-31)

92. As a user, I want a Settings switch that hides the git branch on tab rows (default: off), so that the sidebar stays minimal when I don't care about branches.
93. As a developer, I want each workspace row to show, per tab, one line combining that tab's agent-status chip with the folder that tab's shell is in — every tab gets a line (with or without an agent), duplicate folders are not merged — toggled by a Settings switch (default: off), so that I can see at a glance where every terminal sits. *(Rides the working directories umux already tracks; follows story #42's Settings pattern.)*

---

## Implementation Decisions

### Architecture: Approach A — built-in terminal
umux is a single desktop application with its own embedded terminal surface, rather than a tool that manipulates external terminal windows. This is simpler and far more stable on Wayland (where manipulating foreign windows is restricted), and it gives umux full control over the byte stream needed for OSC-based completion detection.

### Major functional components

**Backend (Rust):**

1. **PtyService** *(deep module)* — Owns the lifecycle of pseudoterminals.
   - Interface: `open(request) → handle`; `write(handle, bytes)`; `resize(handle, cols, rows)`; `close(handle)`; plus a per-handle output stream of bytes.
   - Encapsulates fork/exec (via a portable-pty-style crate or `nix`), file-descriptor management, signal handling, and clean teardown.
   - Cross-platform via `portable-pty` (ConPTY on Windows). Default shell per OS: `$SHELL` or `/bin/bash` on Linux/macOS, PowerShell on Windows.
   - Deep because the interface is small but it hides a large, OS-specific body of work.

2. **OscParser** *(deep module)* — A pure, stateful byte-stream parser.
   - Interface: `push(bytes) → (passthrough_bytes, emitted_events)`.
   - Recognizes OSC 9;9, OSC 99, and OSC 777 completion sequences, extracts their payloads as notification events, and forwards all non-matching bytes untouched so the terminal output is never altered.
   - Deep because the surface is tiny but it implements a state machine, handles partial sequences that span chunk boundaries, and supports three protocols. It is trivially unit-testable with fixed byte fixtures and has no I/O dependencies.

3. **NotificationService** — Consumes parsed OSC notification events and delivers them to the desktop via `notify-rust` (D-Bus/libnotify on Linux, native notification centers on Windows and macOS). Debounced and idempotent. Behavior on Linux is considered stable — do not change it; only platform portability is verified.

4. **SshManager** *(deep module)* — Opens PTY-backed shells over SSH.
   - Interface: `connect(spec) → session`, reusing PtyService's output-stream abstraction so the frontend treats local and remote panels identically.
   - Encapsulates the SSH transport, agent/key authentication, and channel-to-PTY bridging.
   - Supported on Linux and macOS for v1.0.0; Windows SSH is deferred (agent/keystore differences).

5. **WorkspaceStore** — Persists workspace definitions (names, order, panel layout, working directories, SSH targets) and application settings to a per-OS config directory: `~/.config/umux` (or `$XDG_CONFIG_HOME/umux`) on Linux, `%APPDATA%\umux` on Windows, `~/Library/Application Support/umux` on macOS. Read on startup, written on change.

6. **CommandBridge** — The Tauri command surface (`invoke` handlers) that exposes the above services to the frontend and ferries PTY output to the terminal renderer over a Tauri event channel.

**Frontend (React + TypeScript):**

7. **TerminalSurface** — Wraps `xterm.js`; attaches to a PTY handle's output stream and sends keystrokes back through the CommandBridge. One instance per panel.

8. **PaneLayout** *(deep module)* — Owns the split state and geometry of **any number of panels** within a workspace (v0.2.0 lifts the original two-panel cap). Computes geometry for a tree of splits, handles drag-resize of dividers, and enforces per-panel minimum sizes. Pure layout logic, testable without a live terminal.

9. **WorkspaceShell** — The workspace switcher UI: list of workspaces, create/rename/delete/close actions.

10. **SshConnectDialog** — UI for entering or selecting an SSH target and opening a remote panel.

11. **SettingsScreen** — Settings UI with feature toggles (agent status, notifications), persisted through WorkspaceStore.

12. **AgentStatusIndicator** — Per-panel status indicator (working / waiting / idle) driven by OSC-derived events.

**Planned v1.2.0:**

13. **StoreCore** *(deep module)* — The workspace-definition persistence library, extracted from the desktop backend so the desktop app and the `umux` CLI share one implementation.
    - Interface: load a store → workspace model; apply mutations (create/remove/rename/reorder/set-layout/import); save atomically; export/import in a neutral exchange format.
    - Encapsulates per-OS path resolution, schema versioning, validation, corruption fallback, and atomic writes. Deep because the webview app and the CLI both depend on a tiny interface that hides all file-format concerns.
14. **CmuxImporter** *(deep, pure)* — Parses a cmux configuration into an import plan.
    - Interface: `parse(text) → ImportPlan` (workspaces, tabs, panels, layout, working directories; **grouping mapped to umux workspace groups — flat when the source stores none**; collision-resolved names with a `from cmux` suffix).
    - Read-only toward cmux files; no I/O beyond the text it is handed, so it is unit-testable against captured fixture files.
15. **umux CLI binary** — Same Rust codebase, separate binary target. v1.2.0 ships offline commands only: list/new/rm/rename/split/import/export/config/notify/version; store-targeting commands require `--desk/--desktop` or `--term/--terminal`, otherwise they exit with a hint. `umux` with no arguments prints help; `umux --term` launches nothing yet (v1.3.0) and says so.
16. **UpdateChecker** *(planned v1.2.0)* — In-app update check + one-click apply, via Tauri's updater plugin against GitHub Releases. Update bundles are signed with Tauri's built-in key generation (`tauri signer` — free), so the zero-cost policy holds; installer first-run warnings remain documented as today. Detailed design (endpoint, cadence, startup vs manual check) at /carve time.

**Planned v1.3.0:**

17. **HerdrImporter** *(deep, pure)* — Same interface as CmuxImporter, reading herdr's saved session state (unofficial format). Optionally imports worktree checkouts and agent sessions on explicit opt-in.
18. **umux Terminal (TUI)** — A terminal-native frontend (Rust, no webview): sidebar with workspaces, tabs, unlimited panels, tmux-style prefix shortcuts (prefix **Ctrl+B**, confirmed 2026-08-31) plus mouse support. Reuses PtyService and OscParser; shows agent status in the sidebar and panel titles. Keeps its **own** store (separate state from the desktop, decided 2026-08-28). No background daemon in v1.3.0 — closing the terminal ends its sessions (daemon stays v2.0). Ships on all three platforms; the v1.3.0 release waits until the Windows (ConPTY) TUI works too (2026-08-31 — replaces the old "Linux + macOS first").
19. **CliGateway** *(planned v1.3.0, was v1.1 scope)* — Local socket server inside the running app exposing the live surface at **full parity with the app** (2026-08-31: every action the UI supports, not a curated subset) to the CLI and directly to agents; the CLI and the socket expose the same surface. Detailed design at /carve time.

**Planned v1.4.0:**

20. **AppMenus** — A single action registry (every app action listed once) rendered as the native menu bar (File/Edit/View/Help) on macOS and as a ☰ dropdown beside the app title on Windows/Linux. Building menus from the registry — not by hand — is what enforces the "every feature ships with its menu entry" rule (story #84).

**Planned v1.5.0:**

21. **ShellDetector** *(deep, pure)* — Turns injected probe results (PATH scan + registry checks on Windows) into the installed-shell list (display name + launch command) for both the Settings default-shell picker and the "+ New tab" dropdown. The pure core does no I/O — detection, ranking, and dedup are unit-testable; setups the probes miss land in the custom entry. The remaining v1.5.0 items (sidebar width persistence, the two metadata switches, per-tab folder lines on workspace rows) extend the existing SettingsScreen, SettingsStore, and WorkspaceShell on top of the working directories umux already tracks — no further new modules.

### Key data flows
- **Keystrokes:** xterm.js → CommandBridge → `PtyService.write` → PTY.
- **Output:** PTY → PtyService output stream → OscParser (inspect) → CommandBridge event → xterm.js; in parallel, OscParser notification events → NotificationService → desktop.
- **Persistence:** WorkspaceStore writes on workspace/panel/layout changes.

### Technology-specific constraints
- Supported platforms: **Linux (developed and tested on Ubuntu/Wayland), Windows 10+, and macOS 11+** (universal binary: Apple Silicon + Intel). X11 sessions on Linux are not tested and not officially supported.
- Completion detection relies on AI CLI tools emitting OSC 9;9 / OSC 99 / OSC 777 sequences (Claude Code needs `preferredNotifChannel: "iterm2"` — it emits them automatically only in Ghostty/Kitty/iTerm2; see README). No output pattern matching, ever. Clarified 2026-08-25: the per-panel status model MAY additionally detect an AI CLI's PRESENCE by reading the panel's foreground process name from the OS process table (a closed known-CLI list, never terminal content) — completion itself stays OSC-only.
- Screen-content agent-state classification (herdr-style working/blocked/idle/done read from pane output) is explicitly rejected — agent state derives from OSC events and the known-CLI process-presence check only (decided 2026-08-26).
- Builds are **unsigned** (no paid certificates, zero-cost policy). First-run warnings (macOS Gatekeeper, Windows SmartScreen) are documented in the README rather than paid away.
- In-app updates (v1.2.0) run through Tauri's updater plugin against GitHub Releases; update bundles are signed with Tauri's built-in (free) signing key, so the zero-cost policy holds. GitHub Releases is the single source of truth — no extra update server.
- CI (GitHub Actions, on release publish) builds all three platforms: `.deb` + `.AppImage` (Linux), NSIS `-setup.exe` (Windows), universal `.dmg` (macOS).
- Importers (cmux, herdr) read **internal, undocumented formats** of third-party tools — unofficial by nature; they may break when those tools update, and they must never write to the source tool's files. A real `cmux.json` sample is captured as a test fixture **before** implementation starts (open item, 2026-08-28).
- Version numbering (decided 2026-08-28): the old v1.1 "scriptability" scope is split — offline CLI + `umux notify` land in v1.2.0, the live surface + socket API in v1.3.0; the old v1.2 "agent UX" scope moves to v1.4.0. Extended 2026-08-31 (versions-cleanup discovery): v1.5.0 created (Windows shell picker #67, sidebar resize fix + width persistence, metadata switches) and the v1.3.0 release gate moved to all three platforms (TUI included).
- Shell picking (v1.5.0, issue #67) is Windows-local-tabs only: macOS/Linux keep the current default-shell behavior and SSH tabs are untouched (decided 2026-08-31). Detection runs at Settings/first-use time; the list is never a hard-coded assumption that a specific shell exists — the custom entry covers the rest.
- Installers and the CLI: NSIS adds the install directory to PATH; the DMG installs the CLI beside the app and documents a one-line PATH setup; the .deb ships a `/usr/bin/umux`; the AppImage cannot persistently modify PATH, so the alias/extraction workaround is documented. Zero-cost policy applies (no paid signing, no paid hosting).

---

## Assumptions

- cmux's `cmux.json` actually contains workspace layouts the importer can map (its public schema suggests so, but it is unverified against a real file — Adam's `~/.config/cmux/cmux.json` must be captured as a fixture before implementation starts; if layouts live only in cmux's session store, the importer reads that instead). The same holds for **grouping**: if cmux's files carry folder/group structure, the import recreates it as umux workspace groups; if not, the import stays flat (decided 2026-08-28 with the workspace-groups feature, issue #46).
- herdr's `session.json` is parseable and stable enough for an unofficial importer (no supported import path is documented; the format may change without notice).
- cmux/herdr users accept a **one-time** import; live synchronization is not expected by anyone.
- Concurrent CLI edits while the desktop app is open are rare; v1.2.0 relies on atomic writes without cross-process locking (last write wins). If real-world conflicts appear, a lock/notify mechanism is a follow-up.
- Zero-cost distribution holds: GitHub Releases and unsigned builds remain the only channels (no paid certificates, no paid hosting).
- The TUI can reuse the PtyService/OscParser abstractions without a backend rewrite.
- The Windows TUI rides the same portable-pty/ConPTY path the desktop app already uses, so the porting risk is the TUI frontend, not ConPTY itself (basis for the 2026-08-31 all-platform release gate).
- Windows shell auto-detection (PATH + registry) covers standard installs; non-standard setups are handled by the custom entry — accepted 2026-08-31.
- Sidebar width persistence rides the existing settings storage; no store schema migration is expected.

## Tradeoffs Considered

- **Background daemon in v1.3.0** — rejected: the single most complex component (autostart, crash recovery, secure socket); deferred to v2.0 after being explained to the PO (2026-08-28).
- **Live CLI commands in v1.2.0** — rejected: they require the socket gateway; v1.2.0 stays small (definitions + notify only).
- **CLI with its own duplicated store code** — rejected: two implementations of file writes invite drift and corruption; a shared StoreCore is used instead.
- **Continuous two-way sync with cmux/herdr** — rejected: one-time import is far simpler and cannot clobber either tool.
- **Interactive collision resolution in the wizard** — rejected: the PO chose the automatic `from cmux`/`from herdr` suffix (no per-name questions).
- **Prefix-less (direct) TUI shortcuts** — rejected: they collide with programs running inside panels; tmux-style prefix + mouse chosen instead.
- **herdr importer in v1.2.0** — rejected: unofficial format; it ships with v1.3.0 alongside the TUI (re-confirmed by the PO, 2026-08-31).
- **Shipping v1.3.0 without the Windows TUI** — rejected by the PO (2026-08-31): the release waits until the TUI works on all three platforms, even though this delays it.
- **A curated live-command subset in v1.3.0** — rejected by the PO (2026-08-31): full parity chosen — anything the UI can do, the live CLI/socket must do.
- **Merging duplicate folder lines on workspace rows** — rejected (2026-08-31): one line per tab keeps the tab↔folder mapping unambiguous.
- **macOS/Linux shell pickers in v1.5.0** — deferred: issue #67 is scoped to Windows local tabs only (2026-08-31).
- **Publishing the PRD as a GitHub issue** — rejected by the PO: issues are created later, in /dispatch.

## Validation Strategy

### Per-user-story verification
Each user story above maps to one or more acceptance checks. Most are verified by Adam running umux locally on Ubuntu (Wayland); the deep modules are additionally covered by automated tests.

### Component "done" criteria
- **OscParser:** Unit tests cover each supported OSC protocol, including sequences split across byte-chunk boundaries and unrelated escape sequences passing through unmodified.
- **PtyService:** A panel opens an interactive shell, accepts input, produces output, resizes cleanly, and leaves no orphan process after close.
- **PaneLayout:** Unit tests cover split creation, arbitrary N-panel layouts, drag-resize clamping at minimum sizes, and panel-close filling behavior.
- **SshManager:** A panel connects to a remote host using the local SSH agent, supports input/output/resize, and surfaces a clear error on failure.
- **NotificationService:** A desktop notification appears when a simulated completion sequence is injected, and mute works.
- **WorkspaceStore:** Workspaces survive an application restart, and a corrupted config falls back to defaults without crashing.
- **CmuxImporter / HerdrImporter:** Unit tests against committed fixture files: happy path, missing or extra fields, name collisions (suffix applied), malformed input (clear error), and proof the source file is never written.
- **StoreCore:** Round-trip tests (load → mutate → save → load), corrupted-file fallback, atomic-write verification, and export/import round-trip equality.
- **umux CLI:** Integration tests per command against a temporary store; a missing `--desk/--term` flag exits non-zero with a hint; `umux` with no arguments prints every command; `umux --version` reports the app version.
- **ShellDetector:** Unit tests against synthetic probe results — standard shells found and ranked, duplicates deduped, nothing found → only the custom entry remains.

### Quality criteria
- The application stays responsive under continuous output.
- Closing a panel never leaks shell processes.
- Normal terminal output is byte-identical whether or not the OSC parser is active.

### Acceptance thresholds (per release)
- **v0.1 (shipped):** Adam can, on his own Ubuntu (Wayland) machine: create workspaces, split into two resizable panels, run shells and an SSH session, receive a desktop notification when Claude Code finishes generating, and reopen the app with his workspaces intact.
- **v0.2.0:** Adam can, on Ubuntu: split into 3+ panels, see accurate agent status indicators, toggle features in Settings, and reopen the app with his full layout, working directories, and shells restored; closing a busy panel always asks, closing an idle one never does.
- **v1.0.0:** Adam can install and use umux on his Windows machine and his Mac (per-OS shell and config directory working, notifications firing); CI attaches `.exe`, `.dmg`, and Linux artifacts to the release; the landing page on Cloudflare Pages is live; the focused panel zooms to fill the tab and back with one shortcut; tab rows show the branch and hovering shows listening ports.
- **v1.2.0:** On his Mac, Adam imports his real cmux workspaces through the wizard (preview shown, collisions suffixed, cmux files untouched — verified by checksums before/after); every offline CLI command edits the desktop store correctly; `umux notify` fires a notification with the app closed; after each installer, a fresh terminal's `umux --version` works (Windows PATH, macOS DMG, Linux .deb; AppImage workaround documented); the landing page shows the `curl` install command; the app detects a newly published release and applies it from inside the app (Settings → check for updates).
- **v1.3.0:** Adam drives the TUI over SSH on Ubuntu (sidebar, tabs, panes, Ctrl+B-prefixed shortcuts, mouse); agent statuses in the TUI match the desktop app; with the app running, `umux status`/`send`/split steer it live and the socket API returns the same data; herdr import brings in his Ubuntu workspaces; export/import moves setups between Desktop and Terminal in both directions; the TUI runs on Windows (ConPTY) with the same feature set, and every app action — not a subset — is reachable live through the CLI/socket. *(Release gate, 2026-08-31: all three platforms before shipping.)*
- **v1.4.0:** A finished agent's panel/tab shows a marker that clears only after Adam views it; one shortcut jumps to the most recent unread; the command palette opens; a shortcut can be rebound in Settings; a Claude Code teams session opens teammates as separate panes; the macOS menu bar and the Windows/Linux ☰ menu list every app action — including everything shipped in this release.
- **v1.5.0:** On Windows, Adam picks a detected default shell in Settings, spawns a Git Bash tab via the "+ New tab" arrow dropdown, and a WSL tab; on Linux he drags the sidebar to a new width and it survives a restart; with both switches off the sidebar shows neither branches nor folders; with them on, tab rows show branches and workspace rows list one folder line per tab next to its agent chip; every new control has its menu entry (story #84).

---

## Roadmap

- **v0.2.0 — features (Linux first):** unlimited panels (PaneLayout rewritten for N panels), agent status per panel, Settings screen with feature toggles, session restore (layout/panels/working dirs/shells), consistent close confirmations, Aptabase analytics (opt-out). *(shipped)*
- **v1.0.0 — full cross-platform launch:** Windows (NSIS `.exe`) and macOS (universal `.dmg`) builds via GitHub Actions, per-OS shells and config directories, unsigned-first-run docs, landing page on Cloudflare Pages (`*.pages.dev`, zero cost); pane zoom; sidebar metadata (git branch on tab rows, listening ports as a hover tooltip). Promotion (Show HN, Reddit, X/Twitter, dev.to; English) starts only after the landing page is live, with marketing content produced using content skills (Adam, ≤1 h/week; YouTube deferred).
- **v1.2.0 — import & CLI foundations:** one-time import wizard for cmux (what-to-import choice, preview, automatic `from cmux` collision suffix; Windows ships without import), `umux` CLI offline (list/new/rm/rename/split/import/export/config/notify; `--desk/--term` targeting flags with a hint on error), StoreCore extracted so the CLI and the app share one store implementation, CLI installed to PATH by NSIS/DMG/.deb (AppImage workaround documented), an in-app update check + one-click update from GitHub Releases (issue #55), and an "install the CLI" section (curl command) on the landing page.
- **v1.3.0 — umux Terminal + live control:** full TUI (sidebar/tabs/panes, tmux-style prefix — Ctrl+B — + mouse, OSC agent statuses in sidebar and panel titles), separate Desktop/Terminal states with export/import in both directions, live CLI + local socket API at **full parity with the app** (the old v1.1 scope), and the herdr importer (unofficial session format). **Release gate (2026-08-31): ships only once the TUI works on all three platforms — Linux, macOS, Windows (ConPTY).**
- **v1.4.0 — agent UX & convenience (moved from the old v1.2):** unread markers (panel rings + tab/workspace badges, cleared on view) with a jump-to-unread shortcut, command palette, GUI shortcut editor in Settings, teammates/subagents as native panes (built on the v1.3.0 API), plus native menus: a macOS menu bar (File/Edit/View/Help) and a ☰ menu button on Windows/Linux exposing every app action (issue #56; standing rule from story #84: every future feature ships with its menu entry).
- **v1.5.0 — Windows shell picker & sidebar polish (added 2026-08-31, issue #67):** a default-shell picker in Windows Settings (auto-detected PowerShell/cmd/Git Bash/WSL + custom entry) plus a shell dropdown on the arrow beside "+ New tab" (local tabs only), the sidebar drag-resize fix for Windows/Linux with persisted width, and two Settings switches defaulting to **off** — git branch on tab rows, and per-tab working directories on workspace rows (agent chip + folder, one line per tab, duplicates unmerged). Menu entries per story #84.
- **v2.0 — later:** a background server (daemon) so sessions survive app close — `umux attach` and a CLI that works while no window is open (full herdr-style persistence) — plus the plugin system with a browsable marketplace and a built-in scriptable browser pane.

## Out of Scope

- Browser integration before v2.0 (a scriptable browser pane is planned for v2.0 — see Roadmap).
- Git integration (status, diffs, commits, branch management) inside the application. Read-only display of the current branch name and of listening ports in the sidebar is allowed and planned for v1.0.0 (clarified 2026-08-26).
- Support for the X11 display server (untested; Wayland is the reference Linux session).
- Custom theming / advanced appearance customization beyond a sensible default.
- Synchronization of workspaces across machines.
- Mobile platforms.
- Paid code-signing certificates (builds stay unsigned; first-run warnings are documented).
- Plugin system and background session server before v2.0 (see Roadmap).
- Continuous synchronization with cmux/herdr — import is strictly one-time (decided 2026-08-28), and umux never writes to another tool's files.
- A background daemon / headless umux before v2.0 — v1.3.0's umux Terminal lives only as long as its terminal window is open.
- ~~umux Terminal on Windows in v1.3.0~~ — reversed 2026-08-31: v1.3.0 now ships only once the TUI works on all three platforms (story #81).
- Shell picking on macOS/Linux and for SSH tabs in v1.5.0 (issue #67 is Windows local tabs only).

---

## Further Notes

- umux is open-source and hosted publicly on GitHub. There is no commercial revenue model and no hard deadline; post-launch success is measured by GitHub activity (stars, issues and PRs from outside contributors), release downloads, and anonymized Aptabase usage, on top of Adam testing locally.
- Adam is the product owner and does not write code; implementation is performed by Claude Code, with Adam reviewing output and testing locally. Explanations should therefore be step-by-step and beginner-friendly.
- Planning artifacts live in `./plans/`. This PRD is followed by an implementation plan (`umux-plan.md`, produced via `/carve`) and then decomposed into GitHub issues (via `/dispatch`).
