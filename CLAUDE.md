# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What umux is

umux is an open-source terminal workspace manager (a "cmux alternative") for **Linux (Ubuntu/Wayland), Windows, and macOS**. It's a single Tauri v2 desktop app with its own embedded terminal surface — not a tool that manipulates external windows. Users group terminals into named **workspaces** (typically one per project), each holding **terminal tabs** (separate terminal windows, since issue #37's rework) that split into **any number of resizable panels** (unlimited from v0.2.0; originally capped at two). The app inspects the terminal byte stream for AI-CLI completion signals (OSC 9;9 / OSC 99 / OSC 777 escape sequences, which Claude Code emits automatically) and fires a native desktop notification when a long-running task finishes.

The formal spec is the master PRD at `plans/umux-prd.md` (user stories, constraints, Roadmap); `README.md` is the user-facing guide (install, build). Keep both in sync when scope changes. Each roadmap version also has a per-version PRD extract at `plans/umux-vX.Y.Z-prd.md` (v1.5.0 through v2.5.0). **Implementation status: shipped through v1.6.1** (2026-09-11 — Linux, Windows, macOS; unlimited panels, agent status, Settings, session restore, cmux import, offline `umux` CLI, in-app updates, the eight-color palette, the cross-platform shell picker with two default-off metadata switches, the sidebar drag-resize fix; note: the v1.5.0 package shipped as tag **v1.5.1** — no v1.5.0 tag exists). **Roadmap renumbered 2026-09-12 (PO decision; version numbers change ONLY on Adam's explicit call — never renumber or propose renumbering on your own):** **v1.7.0 = umux Core** — the optional daemon (off by default) pulled out of the ecosystem: SessionCore in-process/daemon-client drivers, `umux attach`, a Core section in Settings, autostart, headless CLI, shipped inside normal installers; built on branch **`core`**, released from `main`; plus the landing-page revamp (docs section and more — scope set at v1.7.0 planning, story #136); HITL order: **macOS + Windows first, Ubuntu after** → **v1.8.0** live CLI + local socket at full app parity (the socket protocol is Core's) → **v1.9.0** umux Terminal (TUI; release gate: works on all three platforms) → **v2.0.0** Agents View + unread markers (#56/#57) + the [Local][Agents][SSH] sidebar switcher (renameable labels; Agents placement setting: off default / docked split / separate tab) → **v2.1.0** herdr importer + lucide-react icons + spring press feedback + the global accent color (8 palette colors / custom picker / default blue; color-less items follow it; Custom entry in the color menu — stories #139–#140, supersedes v1.5.0's palette-only rule) → **v2.2.0** command palette + shortcut editor (the ActionRegistry is born here) → **v2.3.0** native menus + pinned tabs (menus render from the registry; standing rule #84) → **v2.4.0** teammates/subagents as native panes → **v2.5.0** SSH View (machine list, remote umux control ≥ v1.8.0 over plain SSH — full remote control, NO embedded terminal and NO live panel-output preview; remote-install offer CLI-or-full-app; remote update via the existing updater) → **v2.6.0** multi-window (many OS windows, each its own full view — own sidebar + own workspace set; one process, one store, one Settings; per-window setup persists — stories #137–#138) → **v2.7.0–v2.9.0 reserved** ("wyjdzie w praniu"). **The umux Ecosystem — umux Bridge, umux Application (PWA), umux NativeApp for Android & iOS (both prototyped 2026-09-12) and the v3.0.0 finale — lives on the `development` branch with its own PRD there; `main` carries no ecosystem scope beyond v1.7.0 (Core).** Standing rule (story #101): every interactive element ships with spring press feedback from day one.

## Commands

Frontend (from repo root):
- `npm run dev` — Vite dev server only (port 5173).
- `npm run build` — `tsc -b && vite build` → outputs to `dist/` (which Tauri bundles).
- `npm test` — Vitest run (single shot). `npm run test:watch` for watch mode.
- Run one test file: `npm test -- EmptyState` (or `npx vitest run src/EmptyState.test.tsx`).

Full app (Rust + frontend together, via Tauri):
- `npm run tauri dev` — launches the desktop window; runs `npm run dev` (configured as `beforeDevCommand` in `tauri.conf.json`) and loads it.
- `npm run tauri build` — production bundle; runs `npm run build` first.
- The Rust backend is normally exercised through Tauri. To iterate on it in isolation: `cd src-tauri && cargo build` / `cargo check` / `cargo test`.

Frontend tests use **Vitest + jsdom + @testing-library/react**. Vitest config lives inline in `vite.config.ts` (globals enabled, setup file `src/setupTests.ts`) — there is no separate vitest config. No Rust tests exist yet.

## Architecture (target, per the PRD)

Two-process Tauri model. The **Rust backend** (`src-tauri/`) owns everything OS- and I/O-bound; the **React + TS frontend** (`src/`) renders the terminal and workspace UI. They communicate via Tauri `invoke` commands (frontend→backend) and Tauri event channels (backend→frontend, primarily the PTY output stream).

Backend modules in `src-tauri/src/` (all implemented as of v0.1):
- **PtyService** *(deep)* — pseudoterminal lifecycle: `open`/`write`/`resize`/`close` plus a per-handle output stream. Owns fork/exec, fd management, signal handling, clean teardown.
- **OscParser** *(deep, pure)* — stateful byte-stream parser: `push(bytes) → (passthrough_bytes, emitted_events)`. Recognizes OSC 9;9/99/777 completion sequences, forwards all other bytes unmodified, and handles sequences split across chunk boundaries. No I/O — trivially unit-testable with fixed byte fixtures.
- **NotificationService** — consumes parsed OSC events → libnotify (`notify-rust`). Debounced, idempotent.
- **SshManager** *(deep)* — PTY-backed shells over SSH using the local agent/keys; reuses PtyService's output-stream abstraction so local and remote panels look identical to the frontend.
- **WorkspaceStore** — persists workspace definitions (names, order, panel layout, working dirs, SSH targets) to `~/.config/umux`. Read on startup, written on change; a corrupted config falls back to defaults rather than crashing.
- **CommandBridge** — the Tauri `invoke` surface + event channel that exposes the above to the frontend and ferries PTY output to the renderer.

Frontend components in `src/` (implemented as of v0.1):
- **TerminalSurface** — wraps `xterm.js` per panel; attaches to a PTY handle's output stream and sends keystrokes back through CommandBridge.
- **PaneLayout** *(deep, pure)* — split state + geometry. v0.1 covers ≤2 panels; **v0.2.0 rewrites it for unlimited panels** (tree of splits, per-panel minimum sizes). Since the #37 rework each **tab** owns one tree (workspaces hold tabs, tabs split into panels). Testable without a live terminal.
- **WorkspaceShell** — workspace switcher (list + create/rename/delete/close).
- **SshConnectDialog** — SSH target entry/selection.

Key data flows (design these against):
- **Keystrokes:** xterm.js → CommandBridge → `PtyService.write` → PTY.
- **Output:** PTY → PtyService stream → OscParser (inspect, never alter) → CommandBridge event → xterm.js; in parallel, parsed notification events → NotificationService → desktop.
- **Persistence:** WorkspaceStore writes on workspace/panel/layout changes.

The modules marked *(deep)* are intended to have small interfaces hiding large, OS-specific implementations, and to be unit-testable in isolation. Preserve those interfaces when implementing.

## Hard constraints (from the PRD — do not drift without a decision)

- **Platforms:** Linux (Ubuntu/Wayland is the reference), Windows 10+, macOS 11+ (universal binary). X11 sessions untested. Windows/macOS builds land at v1.0.0.
- **Panels:** unlimited per workspace (decided Aug 2026 — supersedes the original two-panel cap). PaneLayout enforces a sensible per-panel minimum. **Tabs:** terminal windows inside a workspace (issue #37 rework, Adam's correction — tabs are NOT workspaces); agent-status chips stay on the workspace rows in the sidebar, never on tabs.
- Completion detection relies **solely on OSC escape sequences** — no process polling or output pattern matching.
- **Normal terminal output must be byte-identical** whether or not the OSC parser is active (OscParser only passes bytes through and extracts events; it never mutates terminal output).
- **Notifications are stable on Linux** — don't change their behavior; only verify portability to Windows/macOS.
- **Zero-cost policy:** builds stay unsigned (no paid certificates); Gatekeeper/SmartScreen first-run workarounds are documented in the README.
- Out of scope on `main`: browser integration, git integration, custom theming, cross-machine sync. Plugins are Beyond the Ecosystem; the background session server is **v1.7.0 (umux Core)**; mobile (NativeApps) and remote access live in the Ecosystem on the `development` branch (see PRD Roadmap).

## Repo layout notes

- Frontend sources: `src/` — entry `main.tsx` → `App.tsx` (currently renders `<EmptyState />`).
- Backend sources: `src-tauri/src/` — `main.rs` calls `app_lib::run()` in `lib.rs`. The Cargo package is named `app` (lib `app_lib`).
- Planning artifacts: `plans/` — `umux-prd.md` is the master PRD (source of truth) and each roadmap version has a per-version extract `umux-vX.Y.Z-prd.md` (v0.2.0 → v2.0). Implementation plans from `/carve` land in `plans/` when created. Discovery decisions are merged into the PRDs — the old standalone discovery/plan files were removed in the 2026-08-31 cleanup. The PRD drives `/carve` and `/dispatch` — **check what is actually implemented before assuming any planned component exists.**
- `dist/`, `vite.config.js`, and `*.tsbuildinfo` are generated artifacts and gitignored — don't hand-edit them.

## Working on this repo

The product owner (Adam) does not write code; implementation is done by Claude Code, with Adam reviewing output and testing locally on Ubuntu/Wayland. Keep explanations and next-steps step-by-step and beginner-friendly, and frame acceptance against Adam actually running the app on his machine.
