<p align="center">
  <img src="public-assets/umux-logo-hero.png" alt="umux — terminal workspace manager" width="620">
</p>

A terminal workspace manager for **Linux, Windows, and macOS** — a lightweight, project-oriented alternative to terminal multiplexers like tmux, built as a single native desktop app. Group your terminals into named **workspaces** (one per project), split each into **multiple resizable panels**, connect to remote machines over **SSH**, and get a **desktop notification** when a long-running task finishes.

umux watches the terminal byte stream for the completion signals emitted by AI CLI tools (Claude Code, Aider, etc.) — the standard `OSC 9;9` / `OSC 99` / `OSC 777` escape sequences — and fires a native desktop notification when such a task completes, so you can step away while code is being generated.

> **Platform:** Linux (Ubuntu/Wayland), Windows 10+, and macOS 11+. Releases carry Linux installers and a universal macOS `.dmg`; the Windows installer arrives with the **v1.0.0** release.
>
> **Stack:** [Tauri v2](https://tauri.app) (Rust backend) + React + TypeScript (frontend), rendering its own embedded terminal via [xterm.js](https://xtermjs.org).

---

## Features

- **Workspaces** — create, rename, delete, close, pin (pinned workspaces stay on top), and reorder named workspace collections. Each workspace typically maps to one project, and your setup **persists across restarts** (stored under the per-OS config directory — see [Configuration](#configuration)).
- **Tabs & panels** — every workspace keeps its terminals in **tabs** (separate terminal windows within the project; `Ctrl+Shift+T` adds one, double-click renames, right-click pins), and any tab's area splits into **as many resizable panels as you need** (drag the dividers). A sensible minimum size is enforced so no panel can collapse to nothing. Expand any panel to fill its tab with **pane zoom** — `Ctrl+Shift+Z` or the zoom button in the panel's corner; the same action restores the exact previous layout, and the covered panels keep running underneath.
- **Embedded terminal** — a real terminal surface (colors, cursor movement, alternate screen) so tools like `vim`, `htop`, and `fzf` render correctly. Each panel opens an interactive shell (your `$SHELL` by default; PowerShell on Windows).
- **SSH panels** — open a panel connected to a remote machine over SSH, using your local agent and keys. Remote panels look and behave exactly like local ones. (Linux/macOS; Windows support is planned for v2.0.)
- **Completion notifications** — when an AI CLI tool signals that a long-running task is done, umux fires a native desktop notification. Notifications can be toggled on/off app-wide.
- **Agent status** — each panel shows a live status indicator: **working** while an AI CLI (Claude Code, Codex, Gemini CLI, Aider…) streams or thinks, **needs-attention** the moment it waits for you (opened, finished a task, or asking a question), and idle once you exit it. Detected from OSC completion signals plus the panel's foreground process name — never from terminal content.
- **Settings & toggles** — turn optional features (agent status, notifications) on or off; your choices persist across restarts.
- **Session restore** — reopening umux brings back your workspaces, panels, layout, working directories, and shells.
- **In-app updates** — umux checks quietly on startup and via **Settings → App updates → Check for updates**. When a new release is found, one click downloads, applies, and relaunches. Updates come from GitHub Releases only and are signature-verified — a tampered or unsigned bundle is rejected.
- **Keyboard-first** — switch workspaces, split/close panels, and more without leaving the keyboard.

---

## Installation

umux runs on **Linux (Ubuntu/Wayland)** and **macOS** (11+, universal binary); **Windows** (`.exe` installer) arrives with the v1.0.0 release. There are two ways to install it:

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

#### A.3 — Windows & macOS

Each release ships a universal macOS image (`umux_<version>_universal.dmg`, Apple Silicon + Intel in one file). A Windows installer (`umux_<version>_x64-setup.exe`) arrives with the v1.0.0 release. Download, run the installer / drag to Applications, done.

> **Unsigned builds:** umux is free open source and uses no paid signing certificates. On first run, macOS will report an "unidentified developer" — right-click the app and choose **Open** (or run `xattr -cr /Applications/umux.app` in a terminal). On Windows, SmartScreen may show a blue warning — click **More info → Run anyway**. This only happens once.

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

Artifacts land in `src-tauri/target/release/bundle/`. **Note:** because releases now carry signed update bundles, a local `tauri build` requires the signing key — export `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` first (one-time setup: [Updates → Maintainers](#maintainers-signing-keys-and-secrets)). `tauri dev` needs no key. Install the `.deb` with:

```bash
sudo apt install ./src-tauri/target/release/bundle/deb/*.deb
```

Then launch umux from your application menu, or run `umux` in a terminal.

#### Troubleshooting (build from source)

- **`The pkg-config command could not be found`** — you skipped Step 1. Run the `apt install` line again.
- **Linker errors mentioning `webkit2gtk`** — you installed the `4.0` version instead of `4.1`. Remove it and install `libwebkit2gtk-4.1-dev` (Step 1).
- **`error: failed to run custom build command for ... openssl`** — install `libssl-dev` and `pkg-config` (Step 1).
- **`npm: command not found`** — Node.js isn't installed or your shell didn't pick it up. Re-run Step 3 and open a new terminal.
- **Blank/white window on startup** — make sure you're on a Wayland session (log out, click the gear on the login screen, choose "Ubuntu on Wayland"). X11 sessions are not tested.

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
- **`workspace_store`** — reads/writes workspace definitions to the per-OS config directory; a corrupted config falls back to defaults rather than crashing.

The **React + TypeScript frontend** (`src/`) renders the terminal and workspace UI and talks to the backend through Tauri commands and event channels.

---

## Configuration

Workspace definitions (names, order, panel layout, working directories, SSH targets) are persisted to:

```
Linux:   $XDG_CONFIG_HOME/umux/workspaces.json
         ~/.config/umux/workspaces.json   (default, when XDG_CONFIG_HOME is unset)
Windows: %APPDATA%\umux\workspaces.json   (from v1.0.0)
macOS:   ~/Library/Application Support/umux/workspaces.json
```

On macOS the config directory is `~/Library/Application Support/umux`; on first launch after the change, umux automatically moves `workspaces.json` and `settings.json` from the old `~/.config/umux` location, so nothing is lost.

If the file is missing, umux starts fresh. If it is corrupt, umux falls back to default workspaces and shows a warning rather than failing to launch.

### Enabling completion signals from Claude Code

umux detects finished AI-CLI tasks purely through OSC escape sequences in the terminal stream (OSC 9 / 99 / 777). Claude Code emits those notification sequences **automatically only in Ghostty, Kitty, and iTerm2** — in any other terminal (umux panels included) it stays silent by default, so you get neither the desktop notification nor the *needs-attention* status dot.

To turn the signals on, set Claude Code's notification channel to `iterm2` (it emits the iTerm2-style OSC 9 sequence, which umux parses directly):

```json
// ~/.claude/settings.json
{ "preferredNotifChannel": "iterm2" }
```

This is machine-local and reversible (`"auto"` restores the default). Other AI CLIs that print OSC 99 / 777 notifications work with no configuration.

### Privacy & analytics

umux reports a **single anonymous event** — `app_open` at startup — to [Aptabase](https://aptabase.com), so development is guided by how many people actually installed and use the app (Aptabase counts unique users per event). **Nothing else is ever sent**: no terminal content, commands, workspace names, file paths, or any other user data — the one event carries no payload at all. There is no Settings switch for this (a deliberate product decision: the signal is only useful while always on); to opt out entirely, set `"analyticsEnabled": false` in `settings.json` (same directory as `workspaces.json`) — umux then never initializes the analytics SDK, so no network call is made.

---

## CLI (`umux`)

umux ships a small command-line tool for scripting and offline work. It reads and writes the **same store files as the app** through the same library, so a CLI write and an app write can never disagree about the format.

### `umux` on your PATH

The CLI ships **inside the same installers as the app** — how you get it on your PATH depends on how you installed umux:

| Install method | `umux` availability |
| --- | --- |
| **Windows** (NSIS `.exe`) | **Automatic.** The installer adds the install directory to your user PATH — open a **new** terminal and `umux --version` works. (Terminals that were already open keep their old PATH.) Uninstalling removes the PATH entry. |
| **Linux** (`.deb`) | **Automatic.** The package installs `/usr/bin/umux` — works in any fresh shell. |
| **macOS** (`.dmg`) | One line. The CLI lives beside the app binary inside the bundle; add it to your PATH once: |
| **Linux** (`.AppImage`) | Not persistent by design — an AppImage cannot modify your PATH. See the workaround below. |

macOS PATH setup (then open a new terminal):

```bash
echo 'export PATH="/Applications/umux.app/Contents/MacOS:$PATH"' >> ~/.zshrc
```

AppImage workaround (extract once, link the binary somewhere already on your PATH):

```bash
./umux_*_amd64.AppImage --appimage-extract          # → squashfs-root/
mkdir -p ~/.local/bin
ln -sf "$PWD/squashfs-root/usr/bin/umux" ~/.local/bin/umux
# make sure ~/.local/bin is on your PATH (add to ~/.bashrc if needed):
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
```

Build it from source instead (it is part of the Cargo workspace). Or install it **without the app** straight from the latest release (macOS, Linux x86_64 — review with `-- --dry-run` appended):

```bash
curl -fsSL https://raw.githubusercontent.com/CrystalPlatforms/umux/main/install.sh | sh
```

Note for contributors: `src-tauri/binaries/` (the sidecar copy used by the installers) is a gitignored build artifact — if `tauri dev` ever complains about it, generate it once with `node scripts/bundle-cli.mjs`:

```bash
cd src-tauri
cargo build --release --package umux
# → src-tauri/target/release/umux
```

Every command that touches a store needs a target: `--desk` (the desktop app's store) or `--term` (the terminal-UI store, whose TUI ships in v1.3.0).

```bash
umux list --desk                    # saved workspaces as JSON
umux new myproject --desk           # create an empty workspace
umux rename old new --desk          # rename a workspace
umux rm myproject --desk            # delete a workspace
umux split myproject --desk         # split its layout side-by-side (add --vertical for top/bottom)
umux config get --desk              # all settings as JSON
umux config set notifications-enabled true --desk

umux export --desk -o backup.json   # export the store (see Exchange format below)
umux export --desk                  # …or print the document to stdout

umux import cmux                    # import from the cmux app's saved files
umux import cmux --dry-run          #   …preview the plan, write nothing
umux import umux backup.json --desk # restore an export (REPLACES the store)

umux notify "build finished"        # desktop notification, no app needed
```

`umux notify` uses the same notification mechanism as the app (`notify-send` on Linux, Notification Center via `osascript` on macOS, a Windows toast). If the platform's notification system is unavailable, it prints a clear error and exits non-zero. Point `UMUX_CONFIG_DIR` at a directory to move the whole store root (handy for testing).

> **macOS note (CLI only):** a banner from `umux notify` is attributed to **Script Editor** — macOS attributes every AppleScript-sent notification to its runtime, and a bare CLI process cannot use the native Notification Center (the system refuses unbundled processes). Title and text are correct; on Linux the sender shows as **umux**. **The umux app itself shows proper umux attribution on macOS** (native Notification Center from the bundled app, with an automatic `osascript` fallback during `tauri dev`). Shipping a tiny helper `.app` for the CLI (the terminal-notifier approach) was evaluated and deliberately deferred — it adds a second bundle to every install path plus a permission prompt for a cosmetic gain.

### Exchange format

`umux export` writes a **neutral, self-describing JSON document** — the format `umux import umux` restores and Desktop↔Terminal transfer (v1.3.0) reads back. Settings are deliberately not part of it: they are per-app configuration, not state you move between surfaces.

```json
{
  "format": "umux-exchange",
  "version": 1,
  "kind": "workspaces",
  "data": { "workspaces": [ ... ], "groups": [ ... ], "order": [ ... ] }
}
```

| Field      | Meaning                                                                                                              |
| ---------- | -------------------------------------------------------------------------------------------------------------------- |
| `format`   | Always `"umux-exchange"` — anything else is not an umux exchange document.                                            |
| `version`  | Format version, currently `1`. **Readers must refuse unknown versions with a clear message** — never guess.            |
| `kind`     | What `data` holds: `"workspaces"` (the store's workspace state: workspaces, groups, order).                            |
| `data`     | The store's own serialized shape — the exact JSON `workspaces.json` uses — so export → import round-trips unchanged.   |

`umux import umux <file> [--desk|--term]` **replaces** the chosen store with the document's state — export → import into a fresh store reproduces the original state exactly (id for id). Use `--dry-run` to preview. Malformed documents and unknown versions are refused with a clear error and the store is left untouched.

`umux import cmux` reads the cmux app's saved files (its `cmux.json` config and live session store) and imports them into the chosen store: workspaces, sidebar order, groups with membership, working directories, and one named tab per cmux surface. Nothing is ever overwritten — a name that already exists gets a ` from cmux` suffix (numbered further when taken: `X from cmux`, `X from cmux 2`, …). The cmux files are read strictly read-only. `--dry-run` prints the plan (the collision-resolved tree) and writes nothing. Not available on Windows in v1.2.0. The same import powers the in-app wizard (Settings → Import from cmux); a shared-fixture test suite keeps the two implementations at parity.

---

## Updates

umux keeps itself current through **GitHub Releases only** — there is no update server (zero-cost policy). The flow:

1. **Startup check (quiet).** On launch, umux asks the release feed whether a newer version exists. Nothing is downloaded without your consent; offline, a release without update metadata, or a missing configuration never surfaces as an error at startup.
2. **Manual check.** **Settings → App updates → Check for updates** runs the same check on demand and reports honestly: up to date, update available, offline, a missing feed ("No update information published yet"), or — until the signing key below is set up — **"Updates are not configured yet"** (the expected state before the maintainer step runs; the app never mistakes that for being offline).
3. **One-click install.** When an update is found, a small banner appears and the Settings row offers **Download & restart**. One click downloads, applies, and relaunches the app at the new version.

Signature verification is **enforced**: every update bundle is signed, and umux refuses to install anything whose signature does not verify against its built-in public key. This is why update bundles can be trusted even though the regular installers are unsigned (first-run warnings still apply to those — see [A.3](#a3--windows--macos)).

Supported update channels per platform: **macOS** (universal `.app` bundle), **Windows** (NSIS installer), **Linux** (`.AppImage`). The `.deb` package updates the classic way — install the newer `.deb` over the old one.

### Maintainers: signing keys and secrets

The release pipeline signs update bundles automatically. One-time setup (done once per machine that builds releases):

1. Generate the free signing key (no paid certificate involved):

   ```bash
   npm run tauri signer generate -- -w ~/.tauri/umux.key
   ```

   This prints a **public key** and writes the private key to `~/.tauri/umux.key` (the public key is saved right next to it as `~/.tauri/umux.key.pub`, so it is never lost). The password is optional — an empty password keeps CI simpler. Re-running the command needs `--force` (Tauri refuses to overwrite an existing key).

2. Paste the **public key** into `src-tauri/tauri.conf.json` under `plugins.updater.pubkey` (it replaces the `PASTE_PUBLIC_KEY_FROM_TAURI_SIGNER_GENERATE` placeholder).

3. Add two repository secrets on GitHub (**Settings → Secrets and variables → Actions**):

   - `TAURI_SIGNING_PRIVATE_KEY` — the *contents* of `~/.tauri/umux.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password (empty is fine, set it to the empty string's secret)

   Until these exist, the release workflow fails at the signing step — that is intentional; an unsigned update bundle would be rejected by every client.

4. **Local `tauri build` also needs the key** (bundling now produces signed update artifacts). Export both variables in your shell before building locally:

   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/umux.key)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   ```

   `npm run tauri dev` needs no key — development never bundles.

### Staged-release test (how to verify an update end-to-end)

With the secrets in place: publish release `vX`, then cut `vX+1` as a draft/prerelease and install `vX` from it. The `vX` app should detect `vX+1` on startup and from Settings, apply it in one click, and relaunch showing `vX+1` (About/window title). Going offline and pressing **Check for updates** must show a clear offline message, not a crash.

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

Please keep changes within the project's scope (Linux/Windows/macOS desktop app, OSC-sequence-only completion detection; see the Roadmap in the spec for what is planned next). See the full product spec in [`plans/umux-prd.md`](./plans/umux-prd.md) for the design rationale and constraints.

---

## License

See [LICENSE](./LICENSE).
