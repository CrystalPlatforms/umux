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

## Installation

umux runs on **Ubuntu (Wayland)**. There are two ways to install it:

- **Option A — Download a prebuilt package** (recommended for most users). Grab a ready `.deb` or `.AppImage` from GitHub Releases — no compiler or toolchain needed.
- **Option B — Build from source**. Clone the repo and compile it yourself. Useful if you want the latest unreleased code or want to contribute.

### Option A — Download a prebuilt package (recommended)

Prebuilt packages are published on the GitHub **Releases** page:

👉 **<https://github.com/CrystalPlatforms/umux/releases>**

Each release provides two Linux package formats. Pick one:

#### A.1 — Install via `.deb` (integrates with the system)

The `.deb` is the native Ubuntu package. It installs umux into your application menu, adds the `umux` command, and registers the desktop icon.

1. Download the `umux_<version>_amd64.deb` file from the release.
2. Install it (and its dependency, WebKit):

   ```bash
   sudo apt update
   sudo apt install -y ./umux_*_amd64.deb
   ```

   > Installing via `apt` (instead of `dpkg`) lets the system pull in the required `libwebkit2gtk-4.1` dependency automatically. If you used `dpkg -i` and hit dependency errors, run `sudo apt --fix-broken install`.

3. Launch **umux** from your application menu, or run `umux` in a terminal.

To **update** later, just download the new `.deb` and repeat step 2.

To **uninstall**:

```bash
sudo apt remove umux
```

#### A.2 — Run via `.AppImage` (portable, no install)

The `.AppImage` is a single portable file — no installation, no root permissions. It runs on most modern Linux distributions.

1. Download the `umux_<version>_amd64.AppImage` file from the release.
2. Make it executable:

   ```bash
   chmod +x umux_*_amd64.AppImage
   ```

3. Run it:

   ```bash
   ./umux_*_amd64.AppImage
   ```

   > **First run:** if nothing happens or you see a dialog about "AppImage" support, install AppImageLauncher or run it from a terminal to see the error. On Ubuntu you may also need `libwebkit2gtk-4.1-0` installed (`sudo apt install libwebkit2gtk-4.1-0`).

To **update**, just download the new `.AppImage` and replace the old file. To **uninstall**, simply delete the file.

---

### Option B — Build from source

This is a one-time setup — follow the steps in order. Every command below is run in a terminal. If a step fails, read the error message; the most common problems (missing `pkg-config`, wrong WebKit version) are flagged in the notes.

#### Step 1 — System libraries

The Tauri v2 backend compiles against native WebKit/GTK libraries, so first install the build dependencies. Update the package index, then install everything in one go:

```bash
sudo apt update
sudo apt install -y \
  pkg-config \
  libwebkit2gtk-4.1-dev \
  build-essential \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libxdo-dev \
  libnotify-bin
```

What each one is for:

- **`pkg-config`** — lets Rust locate the native libraries below. This is the single most common missing piece on a fresh clone; without it the build fails with `The pkg-config command could not be found`.
- **`libwebkit2gtk-4.1-dev`** — the web engine Tauri renders into. ⚠️ Tauri v2 needs **`4.1`**, *not* `4.0` (that was Tauri v1). Installing the wrong one produces cryptic linker errors.
- **`build-essential`** — C/C++ compiler (`gcc`, `g++`, `make`) needed to compile native crates.
- **`libssl-dev`** — OpenSSL headers for TLS support.
- **`libayatana-appindicator3-dev`** — system-tray support.
- **`librsvg2-dev`** — SVG rendering (app icons).
- **`libxdo-dev`** — input simulation for keyboard shortcuts.
- **`libnotify-bin`** — provides `notify-send`, used for desktop completion notifications. (Usually preinstalled, but listed for completeness.)

#### Step 2 — Rust toolchain

umux needs a stable Rust toolchain (1.77.2 or newer). Check whether you already have it:

```bash
rustc --version
```

If that prints a version ≥ 1.77.2, skip to Step 3. If it says `command not found`, install Rust via [rustup](https://rustup.rs) (the official installer):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Follow the prompts (the defaults are fine), then load Rust into your current shell:

```bash
source "$HOME/.cargo/env"
```

Verify it works:

```bash
rustc --version     # should now print a version number
cargo --version     # Rust's build tool
```

#### Step 3 — Node.js

The frontend needs [Node.js](https://nodejs.org/) 18 or newer. Ubuntu's default `node` is often older, so check first:

```bash
node --version
npm --version
```

If Node is missing or older than 18, install it via [NodeSource](https://github.com/nodesource/distributions) (recommended over the Ubuntu package, which tends to be outdated):

```bash
curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash -
sudo apt install -y nodejs
```

Re-run `node --version` to confirm you're on 18+.

#### Step 4 — Get the source

Clone the repository (or `fork` it first if you plan to contribute):

```bash
git clone https://github.com/CrystalPlatforms/umux.git
cd umux
```

#### Step 5 — Install JavaScript dependencies

Pull in the frontend libraries once:

```bash
npm install
```

This creates `node_modules/`. You only need to re-run it after updating dependencies (`package.json` changes).

#### Step 6 — Build and run

To launch the desktop window with hot-reloading (this compiles the Rust backend and starts the Vite dev server together — the first build can take several minutes):

```bash
npm run tauri dev
```

The umux window should appear. You're set up.

For a **production / release build** (an installable Ubuntu package):

```bash
npm run tauri build
```

Artifacts land in `src-tauri/target/release/bundle/`. Install the `.deb` with:

```bash
sudo apt install ./src-tauri/target/release/bundle/deb/*.deb
```

Then launch umux from your application menu, or run `umux` in a terminal.

#### Troubleshooting (build from source)

- **`The pkg-config command could not be found`** — you skipped Step 1. Run the `apt install` line again.
- **Linker errors mentioning `webkit2gtk`** — you installed the `4.0` version instead of `4.1`. Remove it and install `libwebkit2gtk-4.1-dev` (Step 1).
- **`error: failed to run custom build command for ... openssl`** — install `libssl-dev` and `pkg-config` (Step 1).
- **`npm: command not found`** — Node.js isn't installed or your shell didn't pick it up. Re-run Step 3 and open a new terminal.
- **Blank/white window on startup** — make sure you're on a Wayland session (log out, click the gear on the login screen, choose "Ubuntu on Wayland"). X11 is not supported in the MVP.

---

## Build & run (quick reference)

Already set up? These are the everyday commands:

```bash
npm install             # one-time, after cloning or pulling dependency updates

npm run tauri dev       # development: desktop window with hot-reload
npm run dev             # frontend only (Vite dev server, http://localhost:5173)

cd src-tauri
cargo check             # fast Rust type-check (no binary produced)
cargo test              # backend unit tests
```

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
