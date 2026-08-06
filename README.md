# umux

A terminal workspace manager for **Ubuntu (Wayland)** — a lightweight, project-oriented alternative to terminal multiplexers like tmux, built as a single native desktop app. Group your terminals into named **workspaces** (one per project), split each into **up to two panels**, connect to remote machines over **SSH**, and get a **desktop notification** when a long-running task finishes.

umux watches the terminal byte stream for the completion signals emitted by AI CLI tools (Claude Code, Aider, etc.) — the standard `OSC 9;9` / `OSC 99` / `OSC 777` escape sequences — and fires a native desktop notification when such a task completes, so you can step away while code is being generated.

> **Platform:** Ubuntu / Wayland only for now. X11 and other platforms are out of scope.
>
> **Stack:** [Tauri v2](https://tauri.app) (Rust backend) + React + TypeScript (frontend), rendering its own embedded terminal via [xterm.js](https://xtermjs.org).

---

## Features

- **Workspaces** — create, rename, delete, close, and reorder named workspace collections. Each workspace typically maps to one project, and your setup **persists across restarts** (stored at `~/.config/umux/workspaces.json`, or `$XDG_CONFIG_HOME/umux/` when set).
- **Panels** — split a workspace's terminal into **up to two** resizable panels (drag the divider). A sensible minimum size is enforced so neither side can collapse to nothing.
- **Embedded terminal** — a real terminal surface (colors, cursor movement, alternate screen) so tools like `vim`, `htop`, and `fzf` render correctly. Each panel opens an interactive shell (your `$SHELL` by default).
- **SSH panels** — open a panel connected to a remote machine over SSH, using your local agent and keys. Remote panels look and behave exactly like local ones.
- **Completion notifications** — when an AI CLI tool signals that a long-running task is done, umux fires a native desktop notification. Notifications can be toggled on/off app-wide.
- **Keyboard-first** — switch workspaces, split/close panels, and more without leaving the keyboard.

---

## Prerequisites

umux builds on Ubuntu. Besides [Node.js](https://nodejs.org/) (18+) and [Rust](https://www.rust-lang.org/) (stable, 1.77.2+), the Tauri v2 backend needs several system libraries.

On a fresh Ubuntu install, run:

```bash
# Rust toolchain (if you don't already have it)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# System libraries required by the Tauri v2 (webkit2gtk-4.1) backend
sudo apt install -y \
  pkg-config \
  libwebkit2gtk-4.1-dev \
  build-essential \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libxdo-dev
```

> Note: Tauri v2 uses **webkit2gtk-4.1** (not 4.0, which was Tauri v1).

Desktop notifications use `notify-send` (part of `libnotify-bin`), which is preinstalled on most Ubuntu desktops:

```bash
sudo apt install -y libnotify-bin
```

---

## Build & run

Install the JavaScript dependencies once:

```bash
npm install
```

### Development

Launch the desktop window with hot-reloading (starts the Vite dev server and the Rust backend together):

```bash
npm run tauri dev
```

To iterate on the frontend alone without the desktop shell:

```bash
npm run dev      # Vite dev server on http://localhost:5173
```

To iterate on the Rust backend in isolation:

```bash
cd src-tauri
cargo check      # fast type-check
cargo test       # backend unit tests
```

### Production / release build

Build an installable Ubuntu package (`.deb` / `.AppImage`):

```bash
npm run tauri build
```

The output artifacts land in `src-tauri/target/release/bundle/`. Install the `.deb` with `sudo apt install ./<file>.deb`, or run the AppImage directly.

### Tests

The frontend test suite uses Vitest + Testing Library:

```bash
npm test             # single run
npm run test:watch   # watch mode
```

---

## How it works

umux is a two-process Tauri app. The **Rust backend** (`src-tauri/src/`) owns everything OS- and I/O-bound:

- **`pty_service`** — pseudoterminal lifecycle (`open`/`write`/`resize`/`close`) and a per-handle output stream.
- **`osc_parser`** — a stateful byte-stream parser that recognizes completion sequences (across chunk boundaries) and passes all other bytes through untouched. Normal terminal output is **byte-identical** whether or not the parser is active.
- **`ssh_manager`** — PTY-backed shells over SSH, reusing the same output-stream abstraction so local and remote panels are indistinguishable to the frontend.
- **`notification_service`** — turns parsed OSC events into desktop notifications (debounced, mute-aware).
- **`workspace_store`** — reads/writes workspace definitions to `~/.config/umux`; a corrupted config falls back to defaults rather than crashing.

The **React + TypeScript frontend** (`src/`) renders the terminal and workspace UI and talks to the backend through Tauri commands and event channels.

---

## Configuration

Workspace definitions (names, order, panel layout, working directories, SSH targets) are persisted to:

```
$XDG_CONFIG_HOME/umux/workspaces.json
~/.config/umux/workspaces.json   (default, when XDG_CONFIG_HOME is unset)
```

If the file is missing, umux starts fresh. If it is corrupt, umux falls back to default workspaces and shows a warning rather than failing to launch.

---

## Contributing

umux is open source and contributions are welcome. To get started:

1. Fork and clone the repository, then follow the **Prerequisites** and **Build & run** steps above.
2. Create your changes on a branch, keeping commits focused.
3. Make sure the test suites pass before opening a PR:
   ```bash
   npm test
   cd src-tauri && cargo test
   ```
4. Open a pull request describing what you changed and why.

Please keep changes within the project's scope (Wayland-only, two panels max per workspace, OSC-sequence-only completion detection). See the full product spec in [`plans/umux-prd.md`](./plans/umux-prd.md) for the design rationale and constraints.

---

## License

See [LICENSE](./LICENSE).
