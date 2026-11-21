# umux v1.9.0 — Implementation Plan (umux Terminal, the TUI)

**Source:** [`umux-v1.9.0-prd.md`](./umux-v1.9.0-prd.md); master PRD [`umux-prd.md`](./umux-prd.md) wins on conflicts (roadmap line: "**v1.9.0 — umux Terminal, TUI**").
**Stories:** #75–#81, #112.
**Branch:** `autom` (base: origin/main @ ae6c54a, v1.8.5).

## Ground truth already in the repo (checked before carving)

- **Term store exists:** `store_core::paths::{term_config_dir, term_config_path, term_settings_path}` — a `term/` sibling under the config root; the CLI's `--desk/--term` targeting already reads/writes it (story #79's separation is pre-built for saved state).
- **`default_launch_mode` exists:** `store_core::settings_store::Settings` carries it (`"gui" | "tui"`, default `"gui"`); `umux config set default-launch-mode` already writes it; nothing launches on it yet — the PRD says `umux --term` "launches nothing yet (v1.9.0) and says so".
- **Shared session engine:** `session_core` (PTY engine moved verbatim from the app, scrollback ring, escape-boundary replay filter, title scanner). The TUI reuses this crate — no backend rewrite (PRD assumption).
- **Agent status is TypeScript-only:** the three-state machine (`idle | working | needs-attention`, OSC events + known-CLI process-presence signal) lives in `src/agentStatus.ts`. The TUI (Rust, no webview) needs a Rust port of the pure machine plus a Rust wiring of the app's `OscParser` + `panel_processes` presence lookup. Screen-content classification stays rejected (PRD constraint).
- **Exchange format exists:** `store_core::exchange` (`umux-exchange`, v1) — the Desktop↔Terminal transfer rides it (`kind: "workspaces"`; Settings are deliberately NOT exchange data — per-surface config).
- **Cargo workspace members:** `.` (app), `store_core`, `cli`, `storestation`, `session_core`. The TUI joins as a new member crate (autom/release rule: member crate versions move with the workspace version; CI builds `--locked`).
- **Socket protocol:** umux Storestation's protocol (v1) is the declared foundation; the TUI attach (#112) goes through `umux-storestation`'s client, the same driver the desktop app uses.

## Issues

### Issue B1 — TUI crate skeleton + `umux --term` launcher + launch-mode wiring
**Type:** enhancement
New workspace member crate `src-tauri/tui` (binary `umux-terminal` or wired into the `umux` binary's dispatch — decided at dispatch time by what the CLI already ships; the crate must join the workspace version so member versions move together, and CI keeps building `--locked`). `umux --term` / `--terminal` stops printing the "launches nothing yet" notice and launches the TUI. Bare `umux` keeps printing help by default; when `default_launch_mode = "tui"` it launches the Terminal instead (story #78 — the setting already exists and is already CLI-writable; this issue makes something finally read it). The launcher decision (setting value + flags → action) is a pure function, unit-tested.
**Acceptance criteria:**
- `umux --term` opens the TUI process on the current terminal on all three dev platforms (Linux verified by Adam in HITL; Windows/macOS compile and run a minimal frame).
- With no arguments and `default_launch_mode = "gui"` (default), `umux` prints help byte-identically to today.
- With `umux config set default-launch-mode tui`, bare `umux` launches the TUI (spawned-process test with `UMUX_CONFIG_DIR` pointed at a tempdir, matching the CLI suite's pattern).
- The decision function (`"gui"|"tui" + flags → Help | LaunchTui`) is a pure, unit-tested function with the default, the tui case, and the `--term` override covered.
- The new crate carries the workspace version number and `cargo build --workspace --locked` succeeds.
**Files/scope:** new `src-tauri/tui/` crate; `src-tauri/cli/src/main.rs` (argument dispatch + launcher); `src-tauri/Cargo.toml` workspace members; TUI deps: `ratatui` + `crossterm` (pure-Rust, no webview — PRD decision).

### Issue B2 — TUI session runtime: PTY panels on session_core
**Type:** enhancement
The TUI's engine: raw-mode terminal takeover, per-panel PTY sessions spawned through `session_core::pty_service` (the exact engine the desktop app and daemon use), output rendered into the ratatui grid, keystrokes written back, resize (SIGWINCH-equivalent) propagated to PTY + layout, clean teardown on exit. Session lifetime per PRD: closing the TUI ends its sessions **unless Core (Storestation) is on**, in which case the TUI was only one attached view (that branch is completed by B6; here the OFF path must be exact).
**Acceptance criteria:**
- A panel runs a real shell; typing reaches the PTY; output renders in the pane without corruption (byte-passthrough is session_core's guarantee — unchanged).
- Splitting/ resizing the window resizes the PTYs (`resize` path) and the panes redraw within one frame.
- Closing the TUI terminates its PTY sessions when Core is OFF (integration test: spawned child process is gone after exit).
- Resize handling and the input→PTY write path are factored so the pure parts (key translation, resize arithmetic) are unit-tested without a live PTY.
- Normal output stays byte-identical end to end — the TUI inspects OSC events on a side channel (B5) and never mutates the passthrough.
**Files/scope:** `src-tauri/tui/src/` (runtime module); depends on `session_core` (already a workspace crate).

### Issue B3 — TUI layout: sidebar, tabs, unlimited panes from the term store
**Type:** enhancement
The TUI's UI skeleton reading the **term store** (`term_config_path`): sidebar with workspaces and groups, tabs per workspace, and an unlimited panel tree per tab (a Rust sibling of the frontend PaneLayout: tree of splits, per-panel minimum sizes — pure and unit-testable, same model as `store_core::workspace_store::LayoutNode`). Create/rename/delete workspaces and tabs, split/close/navigate panes; every change persists to the term store through `store_core` only (no direct file I/O — the CLI-write/app-write/TUI-write-one-code-path rule).
**Acceptance criteria:**
- The TUI starts with the workspaces saved in the term store and persists every structural change (created workspace, new tab, split layout) so a restart restores it (round-trip test through `store_core`, tmpdir).
- Unlimited splits with enforced per-panel minimum size: a pure layout module rejects a split that would violate the minimum and is unit-tested on the same cases as the TS PaneLayout suite.
- Sidebar, tab bar, and panes render in a size-adaptive layout down to a small terminal window without panic.
- The TUI never reads or writes the desktop store path — path choice goes through `store_core::paths` only (separation test).
**Files/scope:** `src-tauri/tui/src/` (layout + UI modules); `store_core::workspace_store` model reused as-is.

### Issue B4 — Ctrl+B prefix shortcuts + mouse support
**Type:** enhancement
tmux-style prefix interaction (story #76, prefix **Ctrl+B** confirmed 2026-08-31): a key-sequence state machine (unarmed → armed on Ctrl+B → command key → dispatch, with a disarm/timeout escape) in front of the runtime's input path; armed keystrokes are consumed by the TUI, everything else passes to the focused pane. Commands: new tab, close pane/tab, split horizontal/vertical, focus navigation, workspace switch, zoom pane. Mouse from day one: click to focus pane, wheel scroll into the pane's scrollback, drag on split borders to resize.
**Acceptance criteria:**
- The prefix state machine is a pure module, unit-tested on: arm → command, arm → escape/timeout disarm, and the case where a pane receives a literal Ctrl+B when the next key is not a bound command key within the timeout.
- Split, close, navigate, new tab, and zoom work from prefix chords in the running TUI (manual HITL script provided; state-machine unit tests are the automated gate).
- Mouse click focuses a pane; wheel scrolls its scrollback; dragging a split border resizes it — no prefix needed for any of the three.
- A pane running a program that itself uses Ctrl+B (e.g. an emacs-like shell binding) still receives unbound keys — only the armed chord is intercepted (covered by the state-machine tests).
**Files/scope:** `src-tauri/tui/src/` (input/prefix module over B2's runtime).

### Issue B5 — Agent status in the TUI: Rust port of the status machine + OSC wiring
**Type:** enhancement
Story #77: the same three-state machine (`idle | working | needs-attention`) as the desktop, in Rust: a **pure port** of `src/agentStatus.ts` (same transitions, same injected clock, same presence model v2 semantics — the TS module's header documents every rule and its HITL provenance), fed by (a) the app's `OscParser` completion events over a side channel and (b) the known-CLI process-presence check (`pty_service`'s foreground-name lookup — closed list, never terminal content). Statuses render in the sidebar rows and panel titles. Screen-content classification stays rejected (PRD constraint). This is the TUI's parity risk — the port must mirror the TS machine's documented transitions, including the post-redraw suppression window and the settle/`resumeBytes` rule.
**Acceptance criteria:**
- The Rust machine passes a transition test table mirroring the TS `agentStatus.test.ts` cases (idle→working on chunk, quiet timeout, OSC completion, presence on/off, suppression window, settle window, focus/typing clears).
- Completion detection derives from OSC 9;9/99/777 sequences only (fixture-driven parser tests) — no output pattern matching anywhere in the crate (greppable constraint).
- Sidebar rows and panel titles show the live status for a panel running Claude Code (HITL: statuses match the desktop app for the same sessions — PRD's parity validation).
- The machine is pure (injected clock, no I/O) and lives in a module the desktop app could later adopt, so the two implementations can converge instead of drifting.
**Files/scope:** `src-tauri/tui/src/` (status module); app `OscParser` reused from the app crate (`src-tauri/src/osc_parser.rs` — extract to a shared crate if the dependency direction demands it, decided at dispatch); `src/agentStatus.ts` as the reference spec.

### Issue B6 — Core attach: the TUI as a Storestation view (#112)
**Type:** enhancement
Story #112 completes here: with Core (Storestation) ON, the TUI attaches to the **same living sessions** as the desktop app — through `umux-storestation`'s client/protocol, the same daemon-client driver the desktop uses (PRD: "Desktop/TUI/CLI become views; the UI never knows which driver is active"). With Core OFF, B2's local-session behavior stands unchanged. Closing the TUI with Core ON leaves sessions running.
**Acceptance criteria:**
- With the daemon running and Core enabled, sessions created in the TUI survive TUI exit and are reattachable by the desktop app (`umux attach` / the app's restore path) — integration test in the `storestation` suite's spawned-daemon pattern (`UMUX_CONFIG_DIR` tmpdir).
- The TUI uses the SessionCore driver seam (in-process vs daemon-client) — no TUI-specific socket code; the driver choice is one seam decision at startup.
- With Core OFF, behavior is byte-for-byte the B2 behavior (regression test from B2 still green).
- A dead/unreachable daemon falls back per the existing daemon-client driver's documented fallback (boot-fallback path), never crashing the TUI.
**Files/scope:** `src-tauri/tui/src/` (startup seam); `umux-storestation` client + protocol reused unchanged; `session_core` driver seam.

### Issue B7 — Desktop↔Terminal export/import (#79/#80) + term-store hardening
**Type:** enhancement
Story #80: state moves between Desktop and Terminal in both directions via **CLI commands and UI buttons** (the UI buttons are F1's frontend half; this issue owns the CLI commands and the transfer semantics over the existing `umux-exchange` envelope). Commands follow the v1.8.0 `resource.verb` family (e.g. `umux workspaces export --term ...` / import targeting the other store) and answer `--json` with the existing doc/error-object shapes. Settings are NOT exchange data (per-surface — the exchange module's documented rule); only the workspace model moves. Story #79's separation is re-verified: nothing mixes the two stores on its own.
**Acceptance criteria:**
- Export from `--desk` → import into `--term` and the reverse both round-trip workspaces/groups/order byte-identically (spawned-CLI round-trip tests on tmpdir stores, both directions).
- Unknown exchange `version` is refused with the documented error object — the exchange module's rule holds for the new consumers.
- Import honors the existing collision behavior (`from <kind>` suffix) and never writes to the source store.
- Every new command appears in `COMMAND_TABLE` so `agent-context`/`--help` parity stays green (the parity test is the gate).
- With both stores present, neither mode reads the other's files on its own (path-resolution test: TUI reads only `term_config_path`, desktop only its own).
**Files/scope:** `src-tauri/cli/src/main.rs` (commands + COMMAND_TABLE); `store_core::exchange` reused unchanged if possible; tests under `src-tauri/cli/tests/`.

### Issue F1 — Desktop Settings: launch-mode switch + export/import buttons (#78, #80 UI)
**Type:** enhancement
The frontend half of stories #78 and #80: a Settings control for what plain `umux` launches (GUI / Terminal — reading and writing the `defaultLaunchMode` key the store already carries and the CLI already writes), and Desktop↔Terminal export/import buttons that shell out to the B7 CLI commands (one code path — the buttons invoke the same commands, they do not reimplement the transfer). Follows the existing SettingsScreen/SettingsStore patterns.
**Acceptance criteria:**
- The Settings switch renders the current `defaultLaunchMode` from the store and persists a change through the existing settings round-trip (component test with the store mocked through the same seams SettingsDialog tests use).
- The export/import buttons appear only when the sibling term store command is available (or degrade with a clear message), and each button click produces the same CLI invocation B7's tests pin (invocation-shape test).
- `coerceSettings` carries `defaultLaunchMode` through with default `"gui"` (the existing `src/settings.test.ts` cases stay green — they already cover this key).
- No landing/docs copy changes ride this issue.
**Files/scope:** `src/SettingsDialog.tsx`, `src/settings.ts`, tests alongside; invoke surface already in place.

### Issue R1 — README/docs + workspace config for the TUI crate
**Type:** documentation
README gains the umux Terminal section: what it is, `umux --term` and the `default-launch-mode` setting, the Ctrl+B prefix key table, mouse support, Desktop↔Terminal export/import commands, the Core-attach behavior, and the all-platform note (PRD story #81 gate). The README install/build section learns the new crate. Master-PRD sync duty: the PRD's "Planned" module entry for the TUI flips to shipped wording at release time (the release stage owns that flip; this issue only prepares the README). Landing is untouched — docs-on-landing is v2.7 scope, and every landing edit needs Adam's acceptance before push (standing rule).
**Acceptance criteria:**
- README documents every Terminal-facing command and setting introduced by B1–B7, and no README statement contradicts the PRD (prefix is Ctrl+B; sessions end on close unless Core is on; Settings are not exchanged).
- The README build section still builds verbatim on a fresh clone (`npm run tauri build` path unchanged for desktop users).
- No `landing/` file changes in this version's diff.
**Files/scope:** `README.md`; no `landing/`, no `plans/` content changes beyond this plan file itself.

## Packages (order inside a package: independent → dependent)

### BACKEND (branch `autom`, Rust)
| # | Issue | Depends on |
|---|-------|------------|
| B1 | TUI crate skeleton + `umux --term` launcher + launch-mode wiring | — |
| B7 | Desktop↔Terminal export/import CLI + term-store hardening | — (independent of B1; touches only the CLI crate) |
| B2 | TUI session runtime (PTY on session_core) | B1 |
| B3 | TUI layout: sidebar, tabs, unlimited panes | B1 |
| B4 | Ctrl+B prefix shortcuts + mouse | B2, B3 |
| B5 | Rust agent-status machine + OSC wiring | B2, B3 |
| B6 | Core attach (#112) | B2 (runs against B2's runtime) |

B1 and B7 are the two independent entry points — they can be dispatched first, in either order or in parallel.

### FRONTEND
| # | Issue | Depends on |
|---|-------|------------|
| F1 | Settings: launch-mode switch + export/import buttons (#78, #80 UI) | B7 (the buttons call B7's commands; the switch needs only the existing store key — the B7 dependency is soft and only gates the buttons' acceptance tests) |

### RESZTA (docs, config, landing)
| # | Issue | Depends on |
|---|-------|------------|
| R1 | README/docs + workspace config for the TUI crate | B1–B7, F1 (documents what exists; last) |

## Indicated tests (run at the end of the version, after all packages)

Automated:
- `cargo test --workspace` — all Rust suites, including the new `tui` crate's pure modules (launcher decision, prefix machine, layout minimums, agent-status port, exchange round-trips) and the existing `cli/` and `storestation/` integration suites (B6's attach tests land in the storestation pattern).
- `cargo build --workspace --locked` — CI parity (member crate versions must move together; the lockfile must not drift).
- `cargo clippy --workspace` — lint gate as per repo practice.
- `npm test` — full frontend suite (F1's component tests + the existing `settings.test.ts` / `agentStatus.test.ts` must stay green; the TS machine is untouched — B5 is a Rust port, not a TS refactor).
- `npm run build` — `tsc -b && vite build` type-check + bundle.

HITL (Adam, per the PRD's validation strategy):
- TUI over SSH on Ubuntu: sidebar, tabs, panes, Ctrl+B chords, mouse (story #75).
- Agent parity: TUI statuses match the desktop app for the same sessions (story #77).
- State moves: export/import in both directions; the stores never mix on their own (stories #79–#80).
- Core attach: TUI and desktop show and steer the same live sessions with Core ON (story #112).
- Release gate: the TUI runs on Windows (ConPTY) with the same feature set before the release ships (story #81) — macOS + Windows first, Ubuntu after (the standing HITL order).

## Risks

- **Windows ConPTY TUI rendering is the release gate** (story #81): the whole release waits on it; ratatui/crossterm are portable but untested here. B1's skeleton should smoke-test a frame on Windows early.
- **Agent-status drift:** B5 duplicates the TS machine in Rust; the transition-table test ported from `agentStatus.test.ts` is the drift guard. If the dependency direction allows, extracting the machine to a shared crate beats a copy.
- **`umux` binary growth:** the CLI crate gaining the TUI launcher must not break the spawned-process CLI test suite's assumptions (help output byte-identity is pinned by existing tests — B1 must keep them green).
- **Storestation attach on the TUI** inherits the daemon's dev-vs-installed-binary gotcha: replay/testing changes need `cargo build --workspace` plus `umux-storestation stop` (documented in repo CLAUDE.md).
- **Scope discipline:** icons/press-feedback/pinned-tabs story headings in the master PRD still carry stale "v1.9.0" labels from the 2026-09-07 renumber; the 2026-09-12 roadmap supersedes them (v2.1.0 / v2.3.0). They are NOT in this version.
