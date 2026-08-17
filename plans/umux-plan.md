# Plan: umux 2.0 — v0.2.0 features + v1.0.0 cross-platform launch

> Source PRD: [`plans/umux-prd.md`](./umux-prd.md) (see Roadmap). Discovery decisions:
> [`plans/umux-2.0-plan.md`](./umux-2.0-plan.md). v0.1 is shipped; this plan
> covers v0.2.0 (phases 1–7) and v1.0.0 (phases 8–12). v2.0 (plugins, background
> session server) is intentionally excluded.

## Architectural decisions

Durable decisions that apply across all phases:

- **Architecture style (unchanged from v0.1):** two-process Tauri v2 app — Rust backend
  owns PTY/SSH/OSC/notifications/persistence; React + TS frontend renders terminal and
  workspace UI; they talk via Tauri commands and event channels. Completion detection is
  **OSC-only**; terminal output stays **byte-identical** whether or not the parser is
  active. Notifications behavior on Linux is frozen (portability verified, not redesigned).
- **Panel data model: split tree.** A workspace's layout is a tree. Internal nodes are
  splits (direction: horizontal/vertical, plus a size ratio); leaves are panels (terminals).
  Any panel can be split further; closing a panel merges geometry so siblings fill the
  space. Geometry is computed from node ratios with a per-panel minimum size. This
  replaces v0.1's fixed two-panel model; `PaneLayout` remains the pure, unit-testable
  owner of this logic.
- **Agent status state machine (per panel):** `idle → working → needs-attention`.
  `working` is inferred from PTY byte activity (counting bytes, never inspecting content);
  `needs-attention` is set by an OSC completion event; focusing the panel clears it.
  No process polling, no output pattern matching.
- **Settings:** a `settings.json` file in the same per-OS config directory as
  `workspaces.json` (`~/.config/umux` | `%APPDATA%\umux` | `~/Library/Application Support/umux`),
  same corruption fallback (defaults, no crash). Feature flags are read at startup and on
  change: notifications, agent status, anonymous analytics.
- **Session restore:** WorkspaceStore additionally snapshots *runtime* state (workspace →
  panel tree → per-leaf cwd + shell kind + SSH target) on change and on graceful close.
  Restore re-spawns fresh PTYs in the saved cwds; running processes are never resumed.
- **Analytics:** Aptabase initialized from the Rust backend with aggregate events only
  (app open, workspace created, panel split, notification fired). No terminal content.
  The opt-out flag is honored **before** initialization (no network call when disabled).
- **Cross-platform boundaries:** portable-pty stays the PTY layer (ConPTY on Windows);
  default shell is `$SHELL`/`/bin/bash` on Linux and macOS, PowerShell on Windows; SSH is
  Linux+macOS only until v2.0 (hidden, not broken, on Windows).
- **CI:** GitHub Actions triggered on release publish (as today), extended to a matrix —
  Linux `.deb`/`.AppImage`, Windows NSIS `-setup.exe` (x64), macOS universal `.dmg`
  (`universal-apple-darwin`). All builds unsigned (zero-cost policy; README documents
  Gatekeeper/SmartScreen first-run steps).

---

## Phase 1: Unlimited panels — split tree

**User stories**: 15, 16, 17, 18, 19, 20

### What to build

Replace the fixed two-panel layout with the split-tree model. The user can split any
panel horizontally or vertically, any number of times; dividers between neighbors are
draggable with per-panel minimum sizes enforced; closing a panel lets its siblings fill
the freed space; the whole tree persists per workspace and survives restarts. PTY
handling is unchanged — each leaf simply opens a shell as v0.1 panels do.

### Acceptance criteria

- [ ] Splitting one panel 3+ times in mixed directions renders correctly with draggable dividers
- [ ] No panel can be resized below the minimum size, regardless of panel count
- [ ] Closing a middle panel of 4 leaves a clean layout with no gaps
- [ ] Layout round-trips through app restart (tree, ratios, working directories)
- [ ] PaneLayout unit tests cover N-panel geometry, min-size clamping, and close-fill
- [ ] HITL: Adam builds a 4-panel layout (mixed H/V splits), resizes, restarts, layout returns

---

## Phase 2: Agent status per panel

**User stories**: 39, 40 (41 toggle lands in Phase 3)

### What to build

Each panel shows a live status indicator with three states: idle, working, needs-attention.
The backend already parses OSC events per PTY stream; route those events (plus a
content-agnostic byte-activity signal) to the frontend per panel, and render an
`AgentStatusIndicator` in the panel chrome driven by the state machine. An OSC completion
moves the panel to needs-attention (alongside the existing notification, which is
unchanged); focusing or typing in the panel clears it. Accuracy is a first-class
requirement: no false "needs-attention" when the agent is still working.

### Acceptance criteria

- [ ] Running Claude Code shows "working" while output streams, flips to "needs-attention" on completion
- [ ] Focusing the panel clears needs-attention
- [ ] Idle shell shows no activity indicator noise
- [ ] Status never inspects terminal content — only OSC events and byte-activity counters
- [ ] v0.1 notifications fire exactly as before (no behavior change)
- [ ] HITL: Adam runs a long Claude Code task, switches away, sees the dot change, notification arrives as before

---

## Phase 3: Settings screen with feature toggles

**User stories**: 30, 41, 42, 43

### What to build

A Settings screen (gear in the workspace chrome) with on/off switches for optional
features: desktop notifications (supersedes/extending the existing mute button), agent
status indicators, and — placeholder for Phase 6 — anonymous usage analytics. Choices
persist to `settings.json` in the per-OS config directory with the same corruption
fallback as workspaces, and take effect immediately (agent status dots appear/disappear
live; notifications gate the NotificationService path).

### Acceptance criteria

- [ ] Settings screen opens from the main UI and shows the toggles
- [ ] Toggling agent status hides/shows indicators without restart
- [ ] Toggling notifications off stops them app-wide (and the old mute button stays consistent)
- [ ] Settings persist across restarts; corrupt file falls back to defaults without crashing
- [ ] HITL: Adam flips each toggle, restarts, verifies persistence and immediate effect

---

## Phase 4: Safe panel closing

**User stories**: 13, 46

### What to build

One consistent rule for every close path (X button, keyboard shortcut, workspace close):
if the panel's PTY child process is still alive, always ask for confirmation naming the
risk ("this panel has a running process"); if it is idle/exited, close immediately
without asking. The backend exposes the alive state per handle; the frontend renders a
single shared confirmation dialog used by all close paths.

### Acceptance criteria

- [ ] Closing a panel with a live process (e.g. `sleep 300`) always asks, from every close path
- [ ] Closing an idle panel never asks, from every close path
- [ ] Confirming still terminates the shell cleanly (no orphans — v0.1 behavior kept)
- [ ] HITL: Adam closes busy vs idle panels via mouse and keyboard; behavior is identical and predictable

---

## Phase 5: Session restore

**User stories**: 8, 44, 45

### What to build

On close (and on change), snapshot the runtime session: for each workspace, the split
tree with per-leaf cwd, shell kind, and SSH target. On next launch, restore it — panels
re-spawn shells (local or SSH) in the saved cwds, so a restart costs seconds. Running
processes are not resumed (that is v2.0's background server). Restoring must be visibly
fast and must not block the UI.

### Acceptance criteria

- [ ] After closing the app with 2 workspaces / 4 panels (mixed local + SSH), reopening restores the full layout
- [ ] Restored local panels start in their saved working directories
- [ ] Corrupt/missing snapshot falls back to the v0.1 behavior (stored definitions, fresh shells)
- [ ] Restore does not freeze the UI while PTYs spawn
- [ ] HITL: Adam sets up a multi-panel session across workspaces, restarts, verifies everything returns

---

## Phase 6: Anonymous analytics (Aptabase)

**User stories**: 47

### What to build

Initialize Aptabase from the Rust backend (free tier, official Tauri SDK) and emit
aggregate events only: app open, workspace created, panel split, notification fired.
The analytics toggle from Phase 3 gates everything — when off, the SDK is never
initialized (no network call). No terminal content, workspace names, or paths leave the
machine.

### Acceptance criteria

- [ ] Events appear on the Aptabase dashboard with the toggles on
- [ ] Toggling off in Settings stops all events, including after restart (checked before init)
- [ ] No user content (commands, output, names, paths) is present in any event payload
- [ ] Analytics failure (offline, API change) never affects app behavior
- [ ] HITL: Adam verifies dashboard events appear with opt-in and disappear with opt-out

---

## Phase 7: Release v0.2.0 (Linux)

**User stories**: v0.2.0 acceptance threshold from the PRD

### What to build

Ship the feature release on Linux: bump the version to 0.2.0, update README (move the
*(planned v0.2.0)* markers to shipped), run the full test suites, walk Adam through the
complete HITL checklist for phases 1–6, then tag and publish the release so the existing
Linux CI produces `.deb` + `.AppImage`.

### Acceptance criteria

- [ ] All automated suites pass (frontend + Rust)
- [ ] Adam completes the v0.2.0 HITL checklist on Ubuntu and signs off
- [ ] README features list updated (no more "planned" markers for v0.2 items)
- [ ] Release v0.2.0 published with Linux artifacts attached
- [ ] Release notes summarize the new features in user-facing language

---

## Phase 8: macOS foundations

**User stories**: 9–14, 26–30, 42–43 (as experienced on macOS)

### What to build

Make umux a first-class macOS app, verified on Adam's Mac. Config moves to
`~/Library/Application Support/umux`; the default shell comes from the user's `$SHELL`;
desktop notifications are verified through notify-rust's macOS path; the universal
`.dmg` (Apple Silicon + Intel) builds locally via `npm run tauri build`. Everything from
phases 1–6 (split tree, status, settings, safe close, restore, analytics) is expected to
work identically.

### Acceptance criteria

- [ ] Local build on Adam's Mac produces a `.dmg` that installs and launches
- [ ] Config is read/written under `~/Library/Application Support/umux` (not `~/.config`)
- [ ] Default shell honors `$SHELL` (zsh on a stock Mac)
- [ ] A Claude Code completion fires a native macOS notification
- [ ] Panels, status dots, settings, session restore all behave as on Linux
- [ ] HITL: Adam walks the v0.2 checklist on macOS

---

## Phase 9: Windows support

**User stories**: 9–14, 42–43 (as experienced on Windows)

### What to build

Make umux work on Adam's Windows machine: PTYs through portable-pty's ConPTY backend,
PowerShell as the default shell, config under `%APPDATA%\umux`, native Windows
notifications verified, and SSH panels hidden from the UI (unsupported until v2.0 — not
broken). Local build produces the NSIS `-setup.exe` installer.

### Acceptance criteria

- [ ] Local build on Adam's Windows machine produces `-setup.exe` that installs and launches
- [ ] Panels open PowerShell by default; ConPTY renders colors/vim/full-screen apps correctly
- [ ] Config lives under `%APPDATA%\umux`; session restore works
- [ ] A Claude Code completion fires a native Windows notification
- [ ] SSH UI is hidden on Windows (no broken entry points)
- [ ] HITL: Adam walks the checklist on Windows

---

## Phase 10: CI matrix for three platforms

**User stories**: PRD technology constraint (CI builds all three platforms on release publish)

### What to build

Extend the existing release workflow with `windows-latest` and `macos-latest` jobs
(macOS builds `universal-apple-darwin`) so a published release automatically carries
`.deb`, `.AppImage`, NSIS `.exe`, and universal `.dmg`. Validate with a dry-run release
before depending on it, and confirm the README's Windows/macOS first-run instructions
match the produced artifacts.

### Acceptance criteria

- [ ] Publishing a (test) release triggers all three platform builds green
- [ ] All five artifact types appear attached to the release
- [ ] `.dmg` is universal (runs on Apple Silicon and Intel Macs)
- [ ] Unsigned first-run instructions in README match what users actually see on Win/mac
- [ ] Linux job unchanged from v0.1 behavior

---

## Phase 11: Landing page

**User stories**: launch/promotion (see decisions doc)

### What to build

A static landing page living in the repo (`landing/`), deployed to Cloudflare Pages at
`umux.pages.dev` (zero cost, Git-connected). Content: hero with Adam's demo GIF, one-line
pitch, platform download buttons linking to GitHub Releases (auto-latest), shields.io
badges (version, license, downloads), three feature highlights (workspaces + panels,
agent status + notifications, session restore), and the GitHub link. GoatCounter for
cookie-free stats. No frameworks or build steps beyond what Cloudflare Pages does
natively.

### Acceptance criteria

- [ ] `umux.pages.dev` loads fast on desktop and mobile
- [ ] Download buttons point at the latest release assets for all three platforms
- [ ] Adam's demo GIF and screenshots are embedded
- [ ] Badges render and update automatically
- [ ] HITL: Adam opens the page on his phone and can reach a download in two taps

---

## Phase 12: Release v1.0.0 + promotion kickoff

**User stories**: v1.0.0 acceptance threshold from the PRD

### What to build

Ship the launch release and start promotion. Bump to 1.0.0, publish the release (CI
produces all artifacts), update the landing page download links, and hand Adam the
promotion kit: a materials checklist (demo GIF, screenshots, OG image via Metashot), a
ready-to-post X/Twitter launch thread in English, and a dev.to article draft telling the
"why we built umux" story (GUI simplicity vs tmux-style tools). Record baseline metrics
(stars, downloads, Aptabase actives) so growth is measurable.

### Acceptance criteria

- [ ] Release v1.0.0 published with all five artifacts from CI
- [ ] Landing page serves the v1.0.0 downloads
- [ ] Adam has the promotion kit (checklist + X thread draft + dev.to draft) in hand
- [ ] Baseline metrics recorded (stars, downloads, Aptabase active usage)
- [ ] HITL: Adam posts the launch thread and the dev.to article goes live
