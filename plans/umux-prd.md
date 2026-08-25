# umux — Product Requirements Document (PRD)

**Project:** umux
**Type:** Open-source desktop application
**Platform:** Linux (Ubuntu/Wayland) + Windows + macOS
**Stack:** Tauri v2 (Rust backend) + React + TypeScript (frontend)
**Status:** v0.1 shipped (Linux only); v0.2 and v1.0 planned — see Roadmap

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

### Key data flows
- **Keystrokes:** xterm.js → CommandBridge → `PtyService.write` → PTY.
- **Output:** PTY → PtyService output stream → OscParser (inspect) → CommandBridge event → xterm.js; in parallel, OscParser notification events → NotificationService → desktop.
- **Persistence:** WorkspaceStore writes on workspace/panel/layout changes.

### Technology-specific constraints
- Supported platforms: **Linux (developed and tested on Ubuntu/Wayland), Windows 10+, and macOS 11+** (universal binary: Apple Silicon + Intel). X11 sessions on Linux are not tested and not officially supported.
- Completion detection relies on AI CLI tools emitting OSC 9;9 / OSC 99 / OSC 777 sequences (Claude Code needs `preferredNotifChannel: "iterm2"` — it emits them automatically only in Ghostty/Kitty/iTerm2; see README). No output pattern matching, ever. Clarified 2026-08-25: the per-panel status model MAY additionally detect an AI CLI's PRESENCE by reading the panel's foreground process name from the OS process table (a closed known-CLI list, never terminal content) — completion itself stays OSC-only.
- Builds are **unsigned** (no paid certificates, zero-cost policy). First-run warnings (macOS Gatekeeper, Windows SmartScreen) are documented in the README rather than paid away.
- CI (GitHub Actions, on release publish) builds all three platforms: `.deb` + `.AppImage` (Linux), NSIS `-setup.exe` (Windows), universal `.dmg` (macOS).

---

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

### Quality criteria
- The application stays responsive under continuous output.
- Closing a panel never leaks shell processes.
- Normal terminal output is byte-identical whether or not the OSC parser is active.

### Acceptance thresholds (per release)
- **v0.1 (shipped):** Adam can, on his own Ubuntu (Wayland) machine: create workspaces, split into two resizable panels, run shells and an SSH session, receive a desktop notification when Claude Code finishes generating, and reopen the app with his workspaces intact.
- **v0.2.0:** Adam can, on Ubuntu: split into 3+ panels, see accurate agent status indicators, toggle features in Settings, and reopen the app with his full layout, working directories, and shells restored; closing a busy panel always asks, closing an idle one never does.
- **v1.0.0:** Adam can install and use umux on his Windows machine and his Mac (per-OS shell and config directory working, notifications firing); CI attaches `.exe`, `.dmg`, and Linux artifacts to the release; the landing page on Cloudflare Pages is live.

---

## Roadmap

- **v0.2.0 — features (Linux first):** unlimited panels (PaneLayout rewritten for N panels), agent status per panel, Settings screen with feature toggles, session restore (layout/panels/working dirs/shells), consistent close confirmations, Aptabase analytics (opt-out).
- **v1.0.0 — full cross-platform launch:** Windows (NSIS `.exe`) and macOS (universal `.dmg`) builds via GitHub Actions, per-OS shells and config directories, unsigned-first-run docs, landing page on Cloudflare Pages (`*.pages.dev`, zero cost), start of promotion (X/Twitter + dev.to, English; Adam produces the media himself, ≤1 h/week; YouTube deferred).
- **v2.0 — later:** plugin system with a browsable marketplace, and an optional background server so sessions survive app close (full herdr-style persistence).

## Out of Scope

- Browser integration of any kind.
- Git integration (status, diffs, commits, etc.) inside the application.
- Support for the X11 display server (untested; Wayland is the reference Linux session).
- Custom theming / advanced appearance customization beyond a sensible default.
- Synchronization of workspaces across machines.
- Mobile platforms.
- Paid code-signing certificates (builds stay unsigned; first-run warnings are documented).
- Plugin system and background session server before v2.0 (see Roadmap).

---

## Further Notes

- umux is open-source and hosted publicly on GitHub. There is no commercial revenue model and no hard deadline; post-launch success is measured by GitHub stars, release downloads, and anonymized Aptabase usage, on top of Adam testing locally.
- Adam is the product owner and does not write code; implementation is performed by Claude Code, with Adam reviewing output and testing locally. Explanations should therefore be step-by-step and beginner-friendly.
- Planning artifacts live in `./plans/`. This PRD is followed by an implementation plan (`umux-plan.md`, produced via `/carve`) and then decomposed into GitHub issues (via `/dispatch`).
