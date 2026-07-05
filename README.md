**Project:** umux
**Type:** Open-source desktop application
**Platform:** Ubuntu (Wayland)
**Stack:** Tauri v2 (Rust backend) + React + TypeScript (frontend)
**Status:** Planning

---

## Problem Statement

Power users and developers on Ubuntu accumulate many terminal windows across several projects over the course of a workday. Each project may need multiple shells — a build process, log tails, a local server, an AI coding assistant — and these windows quickly become scattered across virtual desktops or buried in a long window list. There is no lightweight, project-oriented way to group terminals into persistent, switchable collections, so users lose context, waste time hunting for the right window, and end up with a cluttered, hard-to-navigate terminal setup.

Existing tools each solve only part of the problem. Terminal multiplexers (tmux, screen) provide splitting but live entirely inside a single terminal window, do not offer a native desktop experience, and are intimidating for beginners. Tiling terminals (Tilix, Terminator) offer pane splitting but lack the concept of named, persistent project workspaces that survive restarts. None of them watch the terminal stream to notify the user when a long-running AI CLI task (such as Claude Code or Aider generating code) has finished, so the user must either keep watching the window or periodically check back.

## Solution

umux is an open-source terminal workspace manager for Ubuntu (Wayland). It is a single native desktop application that contains its own built-in terminal (Approach A). Users organize their terminals into named **workspaces**, where each workspace typically corresponds to a project. Within a workspace, the terminal area can be split into **up to two panels** that can be resized by dragging the divider. The application also inspects the terminal byte stream for completion signals emitted by AI CLI tools — which emit standard OSC 9;9 / OSC 99 / OSC 777 escape sequences — and fires a native desktop notification when such a task finishes, so the user can step away while an AI generates code.

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
16. As a developer, I want splitting to be limited to two panels per workspace, so that the layout stays simple and predictable.
17. As a developer, I want to split either horizontally or vertically, so that I can arrange panels to fit my task.
18. As a developer, I want to drag the divider between two panels to resize them, so that I can give more room to the panel I am focused on.
19. As a developer, I want the divider to snap to a sensible minimum size on each side, so that a panel never collapses to zero.
20. As a developer, I want to close one of two panels and have the remaining panel fill the space, so that the layout stays clean after closing.

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

---

## Implementation Decisions

### Architecture: Approach A — built-in terminal
umux is a single desktop application with its own embedded terminal surface, rather than a tool that manipulates external terminal windows. This is simpler and far more stable on Wayland (where manipulating foreign windows is restricted), and it gives umux full control over the byte stream needed for OSC-based completion detection.

### Major functional components

**Backend (Rust):**

1. **PtyService** *(deep module)* — Owns the lifecycle of pseudoterminals.
   - Interface: `open(request) → handle`; `write(handle, bytes)`; `resize(handle, cols, rows)`; `close(handle)`; plus a per-handle output stream of bytes.
   - Encapsulates fork/exec (via a portable-pty-style crate or `nix`), file-descriptor management, signal handling, and clean teardown.
   - Deep because the interface is small but it hides a large, OS-specific body of work.

2. **OscParser** *(deep module)* — A pure, stateful byte-stream parser.
   - Interface: `push(bytes) → (passthrough_bytes, emitted_events)`.
   - Recognizes OSC 9;9, OSC 99, and OSC 777 completion sequences, extracts their payloads as notification events, and forwards all non-matching bytes untouched so the terminal output is never altered.
   - Deep because the surface is tiny but it implements a state machine, handles partial sequences that span chunk boundaries, and supports three protocols. It is trivially unit-testable with fixed byte fixtures and has no I/O dependencies.

3. **NotificationService** — Consumes parsed OSC notification events and delivers them to the desktop via libnotify (`notify-rust`). Debounced and idempotent.

4. **SshManager** *(deep module)* — Opens PTY-backed shells over SSH.
   - Interface: `connect(spec) → session`, reusing PtyService's output-stream abstraction so the frontend treats local and remote panels identically.
   - Encapsulates the SSH transport, agent/key authentication, and channel-to-PTY bridging.

5. **WorkspaceStore** — Persists workspace definitions (names, order, panel layout, working directories, SSH targets) to a config file under `~/.config/umux`. Read on startup, written on change.

6. **CommandBridge** — The Tauri command surface (`invoke` handlers) that exposes the above services to the frontend and ferries PTY output to the terminal renderer over a Tauri event channel.

**Frontend (React + TypeScript):**

7. **TerminalSurface** — Wraps `xterm.js`; attaches to a PTY handle's output stream and sends keystrokes back through the CommandBridge. One instance per panel.

8. **PaneLayout** *(deep module)* — Owns the split state and geometry of up to two panels within a workspace. Computes ratios, handles drag-resize, and enforces the two-panel maximum and minimum sizes. Pure layout logic, testable without a live terminal.

9. **WorkspaceShell** — The workspace switcher UI: list of workspaces, create/rename/delete/close actions.

10. **SshConnectDialog** — UI for entering or selecting an SSH target and opening a remote panel.

### Key data flows
- **Keystrokes:** xterm.js → CommandBridge → `PtyService.write` → PTY.
- **Output:** PTY → PtyService output stream → OscParser (inspect) → CommandBridge event → xterm.js; in parallel, OscParser notification events → NotificationService → desktop.
- **Persistence:** WorkspaceStore writes on workspace/panel/layout changes.

### Technology-specific constraints
- Target display server is **Wayland**. X11 support is explicitly out of scope for the MVP.
- Completion detection relies on AI CLI tools emitting OSC 9;9 / OSC 99 / OSC 777 sequences (Claude Code emits these automatically). No process polling or output pattern matching is required.

---

## Validation Strategy

### Per-user-story verification
Each user story above maps to one or more acceptance checks. Most are verified by Adam running umux locally on Ubuntu (Wayland); the deep modules are additionally covered by automated tests.

### Component "done" criteria
- **OscParser:** Unit tests cover each supported OSC protocol, including sequences split across byte-chunk boundaries and unrelated escape sequences passing through unmodified.
- **PtyService:** A panel opens an interactive shell, accepts input, produces output, resizes cleanly, and leaves no orphan process after close.
- **PaneLayout:** Unit tests cover split creation, the two-panel maximum, drag-resize clamping at minimum sizes, and panel-close filling behavior.
- **SshManager:** A panel connects to a remote host using the local SSH agent, supports input/output/resize, and surfaces a clear error on failure.
- **NotificationService:** A desktop notification appears when a simulated completion sequence is injected, and mute works.
- **WorkspaceStore:** Workspaces survive an application restart, and a corrupted config falls back to defaults without crashing.

### Quality criteria
- The application stays responsive under continuous output.
- Closing a panel never leaks shell processes.
- Normal terminal output is byte-identical whether or not the OSC parser is active.

### Acceptance threshold
The MVP is accepted when Adam can, on his own Ubuntu (Wayland) machine: create workspaces, split into two resizable panels, run shells and an SSH session, receive a desktop notification when Claude Code finishes generating, and reopen the app with his workspaces intact.

---

## Out of Scope

- Browser integration of any kind.
- Git integration (status, diffs, commits, etc.) inside the application.
- Support for the X11 display server (Wayland only for the MVP).
- More than two panels per workspace.
- Custom theming / advanced appearance customization beyond a sensible default.
- Synchronization of workspaces across machines.
- Mobile or non-Linux platforms.

---

## Further Notes

- umux is open-source and hosted publicly on GitHub. There is no commercial revenue model and no hard deadline; success is validated by Adam testing locally.
- Adam is the product owner and does not write code; implementation is performed by Claude Code, with Adam reviewing output and testing locally. Explanations should therefore be step-by-step and beginner-friendly.
- Planning artifacts live in `./plans/`. This PRD is followed by an implementation plan (`umux-plan.md`, produced via `/carve`) and then decomposed into GitHub issues (via `/dispatch`).
