# umux v1.7.6 — (COMPLETED)

> Created 2026-10-10 from a /ask discovery + /blueprint session with the PO (Adam).
> Ship order agreed by the PO: the bug package first, the feature package second.

## Problem Statement

On Windows, umux and umux Storestation misbehave in three connected ways:

1. **A console window appears at every login.** The Storestation autostart entry
   launches the daemon and a Windows Terminal window flashes/stays open showing
   daemon logs. The daemon's own `--hidden` console-hide does not work when
   Windows Terminal is the default terminal host.
2. **The app dies ~7 seconds after launch.** Whenever a Storestation daemon is
   already alive, umux crashes with an abort (Event Log exception 0xc0000409,
   faulting module umux-app.exe). Root-caused during discovery: the autostart
   entry points at a **stale dev-build daemon (v1.7.1)** recorded during earlier
   dev-mode testing, while the installed app is v1.7.5 — the app crashes when it
   meets the old daemon. Killing the stale daemon makes the app start normally
   and spawn its own bundled daemon (observed by the PO, reproducible at will).
3. **The `umux` CLI lies.** A manually copied, ancient CLI (1.0.2, from an old
   cargo install) wins PATH resolution over the bundled CLI, so `umux --version`
   reports 1.0.2 and `umux attach` does not work.

Additionally, three UX gaps were agreed into this release: scrollbars render as
persistent classic Windows bars in every scrollable region; tabs cannot be
multi-selected; Settings does not show which app version is installed.

## Solution

umux v1.7.6 makes the Windows Storestation experience match the macOS one:

- The daemon **never opens a window** — at login, when double-clicked, or when
  spawned from the app — while terminal commands (`run`, `stop`, `core on|off`)
  keep printing normally when invoked from a real terminal.
- The app **never crashes on daemon version skew**: it detects an
  incompatible/older daemon after the handshake, replaces it with its bundled
  daemon automatically, and degrades to a clear Settings message instead of
  aborting.
- Autostart becomes **dev-proof and self-healing**: dev builds cannot write the
  OS autostart entry, and a release install repairs an entry that points
  anywhere other than the installed daemon.
- The PO's machine is repaired in place: the stale daemon entry and the stale
  CLI copy are removed as part of the HITL walk.
- Scrollbars appear only while scrolling (macOS-style overlay) everywhere
  except the terminal; tabs support Ctrl/Shift+click multi-selection with
  close/drag-group/move-to-workspace actions; Settings shows the app version
  next to the update status.

## User Stories

### Bug package

1. As a Windows user, I want the Storestation daemon to never open any console
   window (autostart at login, manual launch, or spawn from the app), so my
   desktop stays clean.
2. As a CLI user, when I run `umux-storestation run/stop/core` from a terminal,
   I want its output printed in that terminal, so manual operation still works
   exactly as documented.
3. As a Windows user, I want umux to start and keep running when a Storestation
   daemon is already alive, so the app never disappears seconds after launch.
4. As a Windows user, when the running daemon is older or incompatible with my
   app, I want the app to replace it with its bundled daemon automatically, so
   version skew self-heals without my involvement.
5. As a Windows user, when the daemon cannot be made compatible, I want a clear
   error message in the Settings Storestation section instead of a crash, so I
   know what happened and what to do.
6. As a developer running dev builds, I want autostart installation to refuse
   in dev builds, so dev paths can never poison the OS autostart again.
7. As a Windows user, I want the app to repair an autostart entry that points
   anywhere other than the installed daemon, so the stale entry from this bug
   disappears with the update — no manual registry editing.
8. As a Windows user, I want enabling Storestation (and umux Core) from
   Settings to flash no console window, so the toggle feels native.
9. As a CLI user, I want `umux --version` to report the real installed version
   and `umux attach` to work, so the CLI can be trusted.
10. As a Windows user with Storestation enabled, after closing all umux windows
    and reopening the app, I want all my sessions fully restored, so the
    Storestation promise holds on Windows the way it does on macOS.

### Feature package

11. As a user, I want scrollbars in the app UI to appear only while scrolling
    (thin macOS-style overlay), so the interface looks clean and modern.
12. As a user, I want to select multiple tabs with Ctrl+click (toggle one) and
    Shift+click (range), so I can act on many tabs at once.
13. As a user, I want to close all selected tabs in one action, so bulk cleanup
    is quick.
14. As a user, I want to drag selected tabs together to reorder them, keeping
    their relative order, so a group stays coherent while I organize.
15. As a user, I want to move all selected tabs to another workspace from a
    context menu, so restructuring workspaces needs no per-tab work.
16. As a user, I want the app version shown next to the update status in
    Settings, so I always know what is installed (e.g. "umux is up to date
    (1.7.6)").

## Implementation Decisions

- **Windowless daemon delivery:** the daemon binary stays a console-subsystem
  program (revised 2026-10-15, PO decision after discovery — see Tradeoffs):
  a terminal `run` must keep the v1.7.5 UX, where the shell waits on the
  daemon and Ctrl+C reaches its handler. Zero windows at login is delivered
  by the autostart artifact wrapping the daemon in `conhost --headless`
  (verified working on Windows 11 26300), and console-less spawns from the
  app are covered by process creation flags. Console-less launches keep the
  CLI verbs panic-free through an output-routing decision (valid handles →
  print; attachable parent console → print through it; neither → discard).
- **Silent spawn from the app:** the app-side daemon spawn adds the
  no-window/no-console process creation flags on Windows (covers the Settings
  toggle and the Core enable path).
- **Autostart hardening:** autostart installation refuses in dev builds (same
  spirit as the existing dev skip for single-instance). Release builds verify
  at startup that the platform autostart artifact (Run key / LaunchAgent /
  systemd unit) resolves to the installed daemon and rewrite it when it does
  not — a self-heal that fixes already-poisoned machines on update.
- **Version gate at one choke point:** the daemon-client driver compares the
  daemon version from the hello handshake against the app version. Older or
  incompatible daemon → graceful stop + respawn of the bundled daemon; if that
  fails, a Settings error card — never a crash. All boot-path code that trusts
  daemon-provided data is audited for hard failures in the same pass.
- **Repro-first debugging slice:** the exact panic behind the 1.7.5-app-meets-
  1.7.1-daemon abort is reproduced with a backtrace and fixed before (or as
  part of) the version-gate work, so the gate is verified against a known
  crash, not a hypothetical one.
- **Multi-select as a pure module:** tab selection state (toggle, range, clear,
  group reorder preserving relative order, move-to-workspace) lives in a
  unit-testable reducer; the UI attaches the context menu and drag behavior to
  it. Platform conventions: Ctrl+click toggle, Shift+click range, Esc or plain
  click clears, actions appear in the tab context menu when ≥ 2 tabs are
  selected.
- **One shared overlay-scrollbar module:** a single styling module plus a tiny
  scroll-flash behavior (thumb visible only while scrolling), applied to the
  sidebar, Settings card, tab bar (horizontal), cmux import preview, and
  dialogs. The terminal pane's own scrollbar is explicitly untouched.
- **Version label:** read the app version through the app's own version API and
  render it beside the updater status row (present in both "up to date" and
  "update available" states).
- **Stale CLI repair:** the machine-level fix is removing the old copy so PATH
  resolves to the bundled CLI; the product-level contribution is that
  `umux attach` failure output names the lookup command so a shadowing copy is
  discoverable by any user.

## Assumptions

- The version skew (newer app + older daemon) is the trigger of the observed
  abort; matched versions demonstrably work. If the backtrace slice points
  elsewhere, the causal claim in this PRD is revised but the version-gate and
  hardening stories still stand.
- A console-less daemon launch (headless conhost host, CREATE_NO_WINDOW app
  spawn) leaves the CLI verbs panic-free: the routing decision parks dead
  streams on the NUL device instead of printing into null handles
  (verified 2026-10-15 on Windows 11 26300).
- `umux-storestation stop` works in every launch mode, and Ctrl+C stops a
  daemon launched from a real terminal — the daemon stays a console child
  there, so delivery is the shipped v1.7.5 behavior. A GUI-subsystem variant
  was built and measured first: Ctrl+C under a live PowerShell prompt never
  reaches even an attached daemon (PSReadLine's raw input mode), which
  falsified the original GUI-subsystem assumption and drove the revision.
- The installer/updater replaces sidecar binaries on upgrade (observed on the
  1.7.5 install) — required for the bundled-daemon respawn story.
- WebView2 (Chromium) honors the custom scrollbar styling; jsdom tests assert
  the style guarantees and HITL confirms the visuals.
- Ctrl+click / Shift+click on tabs do not collide with existing tab
  interactions (plain click activates; existing close affordances unchanged).
- Moving tabs between workspaces preserves each tab's working directory and
  live session state as a single-tab move does today.

## Tradeoffs Considered

- **Console-hide retry loops (repeated ShowWindow)** — rejected: with Windows
  Terminal as host, the visible window belongs to another process; hiding is
  unreliable by design.
- **A GUI-subsystem (no-console) daemon binary** — implemented, then rejected
  2026-10-15 with measurements on the PO's machine: a GUI-subsystem process
  never blocks the shell, and at a live PowerShell prompt PSReadLine reads
  keys in raw mode, so Ctrl+C never becomes a console-wide CTRL_C_EVENT —
  the attached daemon never hears it (verified: the same daemon stops
  cleanly on a generated CTRL_C_EVENT when the prompt is not reading).
  Keeping the binary console-subsystem preserves the Ctrl+C requirement;
  `conhost --headless` in the autostart artifact preserves zero windows.
- **A tiny GUI "launcher" exe that spawns the console daemon hidden** —
  rejected: one more shipped binary and install surface; the windowless daemon
  achieves the same with less.
- **Scheduled Task instead of the Run key for autostart** — rejected: extra
  setup/permissions complexity for no gain once the daemon is windowless.
- **Hard gate refusing to run against an incompatible daemon (dialog first)** —
  rejected in favor of auto-repair: the user should not have to act; a Settings
  error card remains as the last resort.
- **Always-visible thin styled scrollbars** — rejected per PO decision
  (2026-10-10): overlay visible only while scrolling.
- **Multi-select via checkboxes** — rejected: Ctrl/Shift+click matches platform
  conventions and adds no visual chrome.
- **Installer PATH takeover to fix the stale CLI** — rejected: overwriting
  files in foreign PATH directories is invasive; manual repair plus a hint in
  attach's error output is honest and sufficient.
- **Fixing only the specific panic without the version gate** — rejected: the
  gate converts a crash class into self-healing behavior; the panic fix alone
  would leave every future skew crash-prone.

## Validation Strategy

Every story is verified on the PO's Windows 11 machine (HITL) with automated
coverage wherever the logic is pure. npm/cargo test suites must stay at the
known baselines (documented flaky lists excluded).

- **Stories 1–2 (windowless daemon, CLI output):** 10 logins with autostart ON →
  zero console windows appear. From a terminal: `run` prints its banner in that
  terminal; `stop` and `core on|off` print expected text. Automated: unit tests
  for the output-routing decision logic; artifact: the shipped daemon binary
  carries no console subsystem entry.
- **Story 3 (no crash with daemon alive):** with a deliberately older daemon
  running, launch the app 10 consecutive times → zero aborts; Event Log gains
  no new 0xc0000409 entries for umux-app.exe.
- **Story 4 (auto-replace older daemon):** with the stale daemon running, start
  the app → within 15 s the responding daemon reports the app's version
  (observable via `umux-storestation --version` semantics / status output).
- **Story 5 (graceful degradation):** simulated incompatible daemon (gate
  refuses) → Settings shows the error card; the app process stays alive for at
  least 5 minutes of interaction.
- **Story 6 (dev-proof autostart):** dev build + autostart toggle → refuses
  with a clear message; the OS autostart entry is unchanged (HITL once +
  automated guard where testable).
- **Story 7 (self-heal):** pre-seed a wrong Run key → install/start the release
  build → the entry is rewritten to the installed daemon path (registry read as
  the observable artifact).
- **Story 8 (no window flash from the app):** toggle Storestation OFF→ON 5× in
  Settings → zero window flashes (HITL, count = 0).
- **Story 9 (CLI trust):** after the repair, `where umux` resolves to the
  bundled CLI; `umux --version` equals the installed version; `umux attach`
  with the daemon running focuses or starts the app.
- **Story 10 (full restore):** two workspaces × two tabs with live shells →
  close all windows → reopen → all four panels return with their sessions
  (HITL; existing replay/attach contract tests keep passing).
- **Story 11 (overlay scrollbars):** automated: the style guarantees are
  asserted in tests; HITL: scroll each region — the bar appears only during
  scrolling and fades after; the terminal scrollbar behaves as before.
- **Stories 12–15 (multi-select):** automated: reducer unit tests cover toggle,
  range, clear, group reorder preserving relative order, and move-to-workspace;
  HITL: Ctrl+click, Shift+click, drag a group, context-menu move, close-all.
- **Story 16 (version label):** artifact: Settings shows a version string equal
  to the installed binary's version; automated component test.

**Definition of done (bug package):** all HITL scripts above pass on the PO's
machine; the stale dev-daemon autostart entry and stale CLI copy are gone from
the PO's machine.

## Out of Scope

- Ubuntu/Wayland re-verification of these paths (the console-window problem is
  Windows-specific; autostart artifacts on macOS/Linux are unchanged in
  behavior).
- v1.8.0 live-CLI socket-parity work; `umux attach` feature growth beyond
  version-resolution trust.
- Scrollbar restyling inside terminal panes (xterm).
- Tab drag-and-drop redesign beyond group moves (spring feedback and accent
  work remain v2.1.0 scope).
- Any roadmap renumbering (v1.8.0+ definitions untouched).

## Further Notes

- Evidence chain recorded 2026-10-10 (PO's machine): Event Log aborts
  0xc0000409, faulting module umux-app.exe 1.7.5, at 12:07 and 14:13; autostart
  Run key pointing at a dev-tree daemon v1.7.1 built 2026-10-04; installed
  daemon on disk reports 1.7.5; a manually cargo-installed CLI 1.0.2 wins PATH.
- PO decisions locked in discovery: version **v1.7.6** (explicit PO call,
  2026-10-10); daemon never shows a window while terminal commands keep
  printing; scrollbars overlay-only-while-scrolling everywhere except the
  terminal; multi-select actions = close + drag-group + move-to-workspace;
  bugs before features.
- This PRD feeds /carve (implementation plan) and /dispatch (GitHub issues) if
  the PO requests them.
