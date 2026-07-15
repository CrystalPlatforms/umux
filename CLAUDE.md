# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What umux is

umux is an open-source terminal workspace manager (a "cmux alternative") for **Ubuntu/Wayland**. It's a single Tauri v2 desktop app with its own embedded terminal surface — not a tool that manipulates external windows, because Wayland restricts foreign-window control. Users group terminals into named **workspaces** (typically one per project), each holding **up to two resizable panels**. The app inspects the terminal byte stream for AI-CLI completion signals (OSC 9;9 / OSC 99 / OSC 777 escape sequences, which Claude Code emits automatically) and fires a native desktop notification when a long-running task finishes.

The authoritative spec is the PRD in `README.md` (duplicated at `plans/umux-prd.md` — only a trailing line differs; treat `README.md` as canonical). **Implementation status is early scaffold**: most of the architecture described below is greenfield target, not existing code. See "Repo layout notes" for what actually exists today.

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

Planned backend modules in `src-tauri/src/` — **`lib.rs` is still the bare `create-tauri-app` scaffold** (only a logging plugin in `run()`), so none of these are built yet:
- **PtyService** *(deep)* — pseudoterminal lifecycle: `open`/`write`/`resize`/`close` plus a per-handle output stream. Owns fork/exec, fd management, signal handling, clean teardown.
- **OscParser** *(deep, pure)* — stateful byte-stream parser: `push(bytes) → (passthrough_bytes, emitted_events)`. Recognizes OSC 9;9/99/777 completion sequences, forwards all other bytes unmodified, and handles sequences split across chunk boundaries. No I/O — trivially unit-testable with fixed byte fixtures.
- **NotificationService** — consumes parsed OSC events → libnotify (`notify-rust`). Debounced, idempotent.
- **SshManager** *(deep)* — PTY-backed shells over SSH using the local agent/keys; reuses PtyService's output-stream abstraction so local and remote panels look identical to the frontend.
- **WorkspaceStore** — persists workspace definitions (names, order, panel layout, working dirs, SSH targets) to `~/.config/umux`. Read on startup, written on change; a corrupted config falls back to defaults rather than crashing.
- **CommandBridge** — the Tauri `invoke` surface + event channel that exposes the above to the frontend and ferries PTY output to the renderer.

Planned frontend components in `src/` — only `EmptyState` exists today:
- **TerminalSurface** — wraps `xterm.js` per panel; attaches to a PTY handle's output stream and sends keystrokes back through CommandBridge.
- **PaneLayout** *(deep, pure)* — split state + geometry for ≤2 panels: ratios, drag-resize clamping at minimum sizes, two-panel maximum. Testable without a live terminal.
- **WorkspaceShell** — workspace switcher (list + create/rename/delete/close).
- **SshConnectDialog** — SSH target entry/selection.

Key data flows (design these against):
- **Keystrokes:** xterm.js → CommandBridge → `PtyService.write` → PTY.
- **Output:** PTY → PtyService stream → OscParser (inspect, never alter) → CommandBridge event → xterm.js; in parallel, parsed notification events → NotificationService → desktop.
- **Persistence:** WorkspaceStore writes on workspace/panel/layout changes.

The modules marked *(deep)* are intended to have small interfaces hiding large, OS-specific implementations, and to be unit-testable in isolation. Preserve those interfaces when implementing.

## Hard constraints (from the PRD — do not drift without a decision)

- **Wayland only** for the MVP; X11 is explicitly out of scope.
- **Two panels max** per workspace; PaneLayout must enforce this and a sensible per-side minimum.
- Completion detection relies **solely on OSC escape sequences** — no process polling or output pattern matching.
- **Normal terminal output must be byte-identical** whether or not the OSC parser is active (OscParser only passes bytes through and extracts events; it never mutates terminal output).
- Out of scope: browser integration, git integration, >2 panels, custom theming, cross-machine sync, mobile/non-Linux platforms.

## Repo layout notes

- Frontend sources: `src/` — entry `main.tsx` → `App.tsx` (currently renders `<EmptyState />`).
- Backend sources: `src-tauri/src/` — `main.rs` calls `app_lib::run()` in `lib.rs`. The Cargo package is named `app` (lib `app_lib`).
- Planning artifacts: `plans/` (only `umux-prd.md` today). The PRD anticipates an implementation plan (`umux-plan.md`, via `/carve`) and GitHub issues (via `/dispatch`) that may not exist yet — `plans/` is expected to outpace the code, so **check what is actually implemented before assuming any planned component exists.**
- `dist/`, `vite.config.js`, and `*.tsbuildinfo` are generated artifacts and gitignored — don't hand-edit them.

## Working on this repo

The product owner (Adam) does not write code; implementation is done by Claude Code, with Adam reviewing output and testing locally on Ubuntu/Wayland. Keep explanations and next-steps step-by-step and beginner-friendly, and frame acceptance against Adam actually running the app on his machine.
