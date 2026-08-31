# umux v1.2.0 — PRD (import & CLI foundations)

**Status:** shipped 2026-08-31 *(as tags v1.0.3 + v1.0.4 — release tags drifted from roadmap numbering; issues #58–#66 all closed; historical document)*
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**Discovery:** decisions of 2026-08-28 ("umux upgrade v1.2.0" session) are merged into the master PRD; that discovery file was removed in the 2026-08-31 plans/ cleanup.

## Problem Statement

Developers on cmux have no way to bring their workspaces into umux without rebuilding them by hand. Everything umux can do lives behind its GUI — scripts, shell hooks, and AI agents cannot read or manage workspace definitions. Installers ship no command-line companion, and users have to reinstall manually to update the app.

## Solution

v1.2.0 ships the offline foundations: a **one-time import wizard** for cmux (scope choice + preview + automatic collision suffix), an **offline `umux` CLI** sharing one store implementation with the app (list/new/rm/rename/split/import/export/config/notify), **CLI binaries installed by every installer**, and an **in-app update check + one-click update** sourced from GitHub Releases.

## User Stories

*(story numbers match the master PRD; all shipped)*

- **51.** As a developer, I want a `umux` CLI that ships with the app, so that I can drive umux from scripts, shells, and tool hooks. *(v1.2.0 — offline subset)*
- **52.** As a developer, I want `umux notify "text"` to raise a desktop notification without the app running, so that tools without OSC support (or explicit hooks) can alert me.
- **61.** As a developer moving from cmux, I want a one-time import wizard in Settings, so that my existing workspaces appear in umux without rebuilding them by hand.
- **62.** As a developer, I want to choose what gets imported (workspace names and order, grouping, working directories, tabs and panel layout), so that I bring only what I need. Grouping present in the source tool is recreated as umux workspace groups; a flat source imports flat.
- **63.** As a developer, I want a preview of the import plan before anything is written, so that I see exactly which workspaces will be created.
- **64.** As a developer, I want name collisions resolved automatically with a `from cmux` suffix, so that an import can never silently overwrite my existing workspaces.
- **66.** As a user, I want importers to treat the source tool's files strictly read-only, so that umux can never damage an existing cmux setup.
- **67.** As a developer, I want the desktop installers (NSIS, DMG, .deb) to put a `umux` binary on PATH (AppImage limitation documented), so that the CLI works in every terminal without extra setup.
- **68.** As a developer, I want `umux` with no arguments to print help listing every command, so that I can discover the CLI without opening docs.
- **69.** As a developer, I want `umux list --desk` to print the saved workspaces, tabs, and panels of the desktop store, so that I can inspect definitions without launching the app.
- **70.** As a developer, I want `umux new/rm/rename/split` to manage workspaces and edit panel layouts in the saved definitions, so that umux is fully manageable from a plain terminal.
- **71.** As a developer, I want `umux import cmux` and `umux export` mirroring the wizard, so that imports and exports are scriptable.
- **72.** As a developer, I want `umux config get/set` (including the default-launch-mode setting), so that I can change settings without the GUI.
- **73.** As a developer, I want store-targeting commands to require `--desk/--desktop` or `--term/--terminal` and to print a hint when the flag is missing, so that I never touch the wrong store by accident.
- **74.** As a developer, I want all CLI writes to go through the same store library the app uses, so that CLI and desktop can never corrupt each other's files.
- **85.** As a user, I want umux to check for updates and tell me when a new version is available (in Settings, and on startup), so that I never run an old version without knowing.
- **86.** As a user, I want to download and apply the update from inside the app — GitHub Releases as the source — so that updating is one click instead of a manual reinstall.

*(Story #65 — the herdr importer — is v1.3.0, see [`umux-v1.3.0-prd.md`](./umux-v1.3.0-prd.md).)*

## Implementation Decisions

- **StoreCore** *(deep)* — workspace/settings persistence extracted into a shared library used by both the webview app and the CLI: load → model, apply mutations, atomic saves, corruption fallback, export/import exchange format. One implementation of the file format.
- **CmuxImporter** *(deep, pure)* — pipeline **parse → plan → apply**; the plan is what the user previews. `parse(text) → ImportPlan`; read-only toward cmux files; collision names get a `from cmux` suffix (numbered when taken). Two parser implementations (TypeScript in-app, Rust in the CLI) tested against the **same** committed fixtures captured from the PO's real machine.
- **umux CLI** — separate binary target in the same Rust codebase; offline only (no socket, no daemon). Store-touching commands require `--desk/--desktop` or `--term/--terminal`; missing flag exits non-zero with a hint. `umux --term` announces the TUI arrives in v1.3.0.
- **Distribution** — NSIS adds the install directory to PATH; `.deb` ships `/usr/bin/umux`; the DMG places the CLI beside the app and documents a one-line PATH setup; the AppImage workaround is documented. An "install the CLI" section (curl command) lives on the landing page. Windows ships **without** import (source files are macOS/Linux).
- **Auto-update** — Tauri updater plugin against GitHub Releases (`latest.json` as the single source); bundles signed with Tauri's built-in free signing key, so the zero-cost policy holds.
- **Existing import stays untouched** — the wizard was added beside the working one-shot import path; the live import path never changed.

## Assumptions

- cmux's `cmux.json` (and, where needed, its session store) contains workspace layouts the importer can map — verified against a real fixture captured from the PO's machine before implementation.
- cmux/herdr users accept a **one-time** import; live synchronization is not expected by anyone.
- Concurrent CLI edits while the desktop app is open are rare; atomic writes without cross-process locking (last write wins) suffice.
- Zero-cost distribution holds: GitHub Releases + unsigned/free-signing builds remain the only channels.

## Tradeoffs Considered

- **Live CLI commands in v1.2.0** — rejected: they need the socket gateway; v1.2.0 stays small (definitions + notify only).
- **CLI with duplicated store code** — rejected: two implementations of file writes invite drift and corruption; shared StoreCore instead.
- **herdr importer in v1.2.0** — rejected: unofficial format; ships with v1.3.0 alongside the TUI.
- **Interactive collision resolution in the wizard** — rejected by the PO: automatic `from cmux` suffix, no per-name questions.
- **Replacing the existing one-shot import with the wizard** — rejected: the working path stayed untouched; the wizard is additive.
- **Paid update infrastructure** — rejected: GitHub Releases is free and the single source of truth.

## Validation Strategy

- **StoreCore:** round-trip tests (load → mutate → save → load), corrupted-file fallback, atomic-write verification, export/import round-trip equality.
- **CmuxImporter:** unit tests against committed fixture files — happy path, missing/extra fields, collisions (suffix applied), malformed input (clear error), and proof the source file is never written.
- **umux CLI:** integration tests per command against a temporary store; missing `--desk/--term` flag exits non-zero with a hint; bare `umux` prints every command; `umux --version` reports the app version.
- **Updater:** the app detects a newly published release and applies it from Settings.
- **Acceptance threshold (as shipped):** on his Mac, Adam imported his real cmux workspaces through the wizard (preview shown, collisions suffixed, cmux files untouched — verified by checksums); every offline CLI command edited the desktop store correctly; `umux notify` fired with the app closed; after each installer a fresh terminal's `umux --version` worked (Windows PATH, macOS DMG, Linux .deb); the landing page shows the curl command; the app detected and applied a new release in-app.

## Out of Scope

- Live control of a running app (`umux status/send`, splits on a live app) and the socket API — v1.3.0.
- The herdr importer — v1.3.0.
- Headless/daemon operation — v2.0.

## Further Notes

- Scope history: in-app updates (issue #55) and native menus (issue #56) were added to the roadmap on 2026-08-29; the menus landed in v1.4.0 instead, the updates shipped here.
- This version's carve plan (`umux-v1.2.0-plan.md`, issues #58–#66) was removed in the 2026-08-31 plans/ cleanup; the work is shipped and documented in the master PRD.
