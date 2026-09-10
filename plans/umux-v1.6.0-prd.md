# umux v1.6.0 — PRD (cross-platform shell picker & sidebar polish)

**Status:** shipped as v1.6.0 on 2026-09-10 · created 2026-08-31 ("versions cleanup" discovery); renumbered v1.9.0 → v1.6.0 and extended to all platforms on 2026-09-07 (swap with the old v1.6.0 icons/press/pinned-tabs package, which moved to v1.9.0). The sidebar drag-resize item (story #91, issue #79) moved to **v1.6.1** on 2026-09-10.
**Source of truth:** master PRD [`umux-prd.md`](./umux-prd.md) — on any conflict the master wins.
**GitHub issue:** #67 (shell support — scope extended from Windows-only to all platforms, 2026-09-07)

## Problem Statement

On Windows, new tabs always open in the OS default shell — a user juggling PowerShell, cmd, Git Bash, and WSL cannot pick per tab or change the default without leaving the app. The same problem exists on Linux and macOS (bash vs zsh vs fish), where the launch shell is equally fixed. On Windows/Linux, the sidebar's drag-to-resize gesture silently does nothing (it works only on macOS). And the sidebar's metadata (git branches, and soon folders) cannot be turned off by users who prefer a minimal sidebar.

## Solution

v1.6.0 adds a **cross-platform shell picker**: a default-shell choice in Settings (auto-detected installed shells + a custom entry) and a small **arrow next to "+ New tab"** opening a dropdown for the shell of that specific tab. The arrow appears **only when more than one shell is detected** — with a single installed shell there is nothing to choose, so the arrow is hidden and "+ New tab" alone remains. It adds two **Settings switches, both defaulting to off**: hide the git branch on tab rows, and show **per-tab working directories on workspace rows** (each tab gets one line combining its agent-status chip with the folder that tab's shell is in). *(The originally planned sidebar drag-resize fix and width persistence moved to v1.6.1 on 2026-09-10 — story #91, issue #79.)*

## User Stories

*(story numbers match the master PRD)*

- **88.** As a user on any platform, I want to pick my default shell in Settings from an auto-detected list (Windows: PowerShell, cmd, Git Bash, WSL; Linux: bash, zsh, fish, …; macOS: zsh, bash, …) or enter a custom command, so that new tabs open in the shell I actually use.
- **89.** As a user on any platform, I want an arrow next to the "+ New tab" button that opens a dropdown for choosing the shell of that specific new tab, so that I can spawn, say, a fish tab without changing my default. **The arrow is visible only when the detector finds more than one shell** — with one shell it is hidden.
- **90.** As a user, I want the shell picker to affect local tabs only — SSH tabs keep today's behavior — so that remote sessions stay predictable. *(Local tabs on all three platforms; SSH excluded — extended from Windows-only, 2026-09-07.)*
- **91.** *(Moved to v1.6.1 on 2026-09-10 — sidebar drag-resize on Windows/Linux + width persistence; see the master PRD and issue #79.)*
- **92.** As a user, I want a Settings switch that hides the git branch on tab rows (default: off), so that the sidebar stays minimal when I don't care about branches.
- **93.** As a developer, I want each workspace row to show, per tab, one line combining that tab's agent-status chip with the folder that tab's shell is in — every tab gets a line (with or without an agent), duplicate folders are not merged — toggled by a Settings switch (default: off), so that I can see at a glance where every terminal sits.

> Per the story #84 standing rule, each new control ships with its menu entry (the v1.8.0 menu registry exists by the time this package is built — the picker's Settings entry may land earlier; exact handling at /carve).

## Implementation Decisions

- **ShellDetector** *(deep, pure)* — turns injected probe results into the installed-shell list (display name + launch command) for both the Settings picker and the "+"-dropdown. Windows: PATH scan + registry checks; **Unix (Linux/macOS): PATH scan + `/etc/shells` + the login shell from the environment**. The pure core does no I/O — detection, ranking, dedup, and the **arrow-visibility rule (hide when ≤1 shell found)** are unit-testable; setups the probes miss land in the custom entry.
- **Picker scope** — local tabs on **all three platforms**; SSH tabs unchanged (extended from Windows-only, 2026-09-07). Clicking "+" itself uses the Settings default; the arrow picks a different shell for just that tab and is rendered only when the detected list has more than one entry.
- **Metadata switches** — the git-branch switch hides only the branch on tab rows (ports tooltip untouched). The folders switch renders one line per tab on its workspace row: agent chip + folder, every tab, duplicates unmerged — data comes from the working directories umux already tracks.
- **Defaults** — both switches are **off** after install (decided 2026-08-31).

## Assumptions

- PATH + registry probing covers standard Windows shell installs; PATH + `/etc/shells` covers standard Unix installs; non-standard setups use the custom entry — accepted by the PO.
- The v1.8.0 menu registry exists by build time (build-order dependency: v1.8.0 later; the picker does not block on it except for menu entries).
- Per-tab folders can render inside the current workspace-row layout without redesign (long-path truncation details at /carve).
- Single-shell installs are common enough on stock Linux/macOS that hiding the arrow is the right default behavior.

## Tradeoffs Considered

- **Merging duplicate folder lines on workspace rows** — rejected (2026-08-31): one line per tab keeps the tab↔folder mapping unambiguous.
- **Windows-only shell picker** — superseded (2026-09-07): the PO extended the scope to local tabs on all platforms; a cross-platform ShellDetector is barely more work than a Windows-only one.
- **Showing the arrow even with one shell** — rejected (2026-09-07): a one-item dropdown is noise; the arrow appears only when there is a real choice.
- **Shell picker for SSH tabs** — rejected for now: remote shells add agent/auth complexity; local-only keeps v1.6.0 small.
- **Hard-coded shell list** — rejected: detection must not assume a specific shell exists; the custom entry covers the rest.
- **Both switches on by default** — rejected by the PO (2026-08-31): a minimal sidebar is the default; metadata is opt-in.

## Validation Strategy

- **ShellDetector:** unit tests against synthetic probe results — standard shells found and ranked per platform, duplicates deduped, nothing found → only the custom entry remains; one shell detected → arrow hidden; two or more → arrow shown.
- **Picker (stories #88–#90):** on Windows, Adam picks a detected default shell in Settings, spawns a Git Bash tab via the "+ New tab" arrow dropdown, and a WSL tab; on Ubuntu he spawns a zsh tab next to his default bash; on macOS the picker lists zsh and bash. An SSH tab still opens exactly as before on every platform. On a machine with a single shell, the arrow is not visible.
- **Switches (stories #92–#93):** with both off, the sidebar shows neither branches nor folders; with them on, tab rows show branches and workspace rows list one folder line per tab next to its agent chip (duplicates visible as separate lines).
- **Menus (story #84):** every new control has its menu entry.
- **Acceptance threshold:** all of the above pass on Adam's Windows machine, his Ubuntu machine, and his Mac.

## Out of Scope

- Shell picking for SSH tabs (deferred, see Tradeoffs).
- Sidebar collapse/expand changes — collapse already works everywhere; the drag-resize fix itself moved to v1.6.1 (2026-09-10, issue #79).
- Git integration beyond the read-only branch display — stays out of scope per the master PRD.

## Further Notes

- Build order: v1.5.x (done) → **v1.6.0 (this package — shipped 2026-09-10)** → v1.6.1 (sidebar resize patch, issue #79) → v1.7.0 → v1.8.0 → v1.9.0. Renumbered from v1.9.0 on 2026-09-07; the icons/press/pinned-tabs package that previously held v1.6.0 moved to v1.9.0 ([`umux-v1.9.0-prd.md`](./umux-v1.9.0-prd.md)).
- Full discovery record: the 2026-08-31 decisions are merged into the master PRD (stories #88–#93, ShellDetector); the standalone discovery file was removed in the same cleanup. The 2026-09-07 cross-platform extension and arrow-visibility rule were decided directly with the PO.
