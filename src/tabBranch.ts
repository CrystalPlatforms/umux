// tabBranch — git branch label mapping for tab rows (v1.0 Phase 14 / #41;
// HITL rework 2026-08-27: a branch per PANEL, not one per tab).
//
// Pure module: which directories each tab's branch labels are read from, and
// the deduplicated list of directories to send to the backend in ONE invoke.
// The resolution itself (`.git` parsing) lives in Rust (git_branch.rs); this
// side only decides WHICH directory speaks for a panel:
//   - a split tab carries an entry for EVERY panel, in tree order — as many
//     branches as splits; null dirs render nothing but keep panel identity;
//   - exactly one entry per tab is `focused` — the focused panel of the
//     workspace's ACTIVE tab (activePanelOf's own fallback chain covers the
//     boot-state first-panel case); inactive tabs focus nothing;
//   - labels come from the persisted starting workingDirectory config —
//     live `cd` tracking is explicitly out of scope for v1.0.0;
//   - SSH-backed panels are skipped (remote path, nothing to parse locally);
//   - when session restore is off (#27), surfaces open WITHOUT their saved
//     cwd — so the saved path must not pose as this panel's start either.

import {
  activeTabOf,
  activePanelOf,
  type Panel,
  type Tab,
  type WorkspaceState,
} from './workspaces'
import { leafIds } from './PaneLayout'

/// One label slot on a tab row: the panel it belongs to, the starting
/// directory its branch is read from (null = no label for this panel), and
/// whether THIS panel currently owns the row's bold treatment.
export type TabBranchEntry = {
  panelId: string
  dir: string | null
  focused: boolean
}

/// Every open workspace's tabs mapped to their per-panel entries (tab id →
/// list in tree order; nothing when there is no tab or the workspace is closed).
export function branchDirsByTab(
  state: WorkspaceState,
  sessionRestoreEnabled = true,
): Record<string, TabBranchEntry[]> {
  const byTab: Record<string, TabBranchEntry[]> = {}
  for (const ws of state.workspaces) {
    if (!state.openIds.includes(ws.id)) continue
    for (const tab of ws.tabs ?? []) {
      byTab[tab.id] = branchEntriesOfTab(state, ws.id, ws.panels, tab, sessionRestoreEnabled)
    }
  }
  return byTab
}

/// The distinct non-null directories to query — one invoke payload for every
/// panel at once; two panels on the same repo share one query.
export function branchQueryDirs(
  state: WorkspaceState,
  sessionRestoreEnabled = true,
): string[] {
  return [
    ...new Set(
      Object.values(branchDirsByTab(state, sessionRestoreEnabled))
        .flat()
        .map((e) => e.dir)
        .filter((d): d is string => d != null),
    ),
  ]
}

/// Merge one `git_branches` answer batch into the label cache (#44). A
/// branch ADDS or updates its directory's label; a `null` answer REMOVES the
/// directory's cached label — leaving a repository (`cd ..` out, `.git`
/// deleted) must make the row's branch disappear, so "no repo" may not be
/// silently ignored the way it used to be. Answers for directories nothing
/// asked about are still honored (harmless: the render reads only live
/// entries), and a malformed entry is skipped rather than poisoning the map.
export function applyBranchAnswers(
  prev: Record<string, string>,
  answers: Array<{ dir: string; branch: string | null }>,
): Record<string, string> {
  let next = prev
  for (const a of answers) {
    if (a?.dir == null) continue
    if (a.branch != null) {
      if (next[a.dir] !== a.branch) next = { ...next, [a.dir]: a.branch }
    } else if (a.dir in next) {
      const { [a.dir]: _gone, ...rest } = next
      next = rest
    }
  }
  return next
}

// --- internals ---------------------------------------------------------------

function branchEntriesOfTab(
  state: WorkspaceState,
  wsId: string,
  panels: Panel[] | undefined,
  tab: Tab,
  sessionRestoreEnabled: boolean,
): TabBranchEntry[] {
  // Only the ACTIVE tab's focus record applies (it is scoped per workspace
  // and only honored while pointing into that tab); inactive tabs focus none
  // of their entries. activePanelOf's first-panel fallback marks something
  // focused even on a boot state with an empty record — exactly one bold
  // branch per active tab row, always.
  const focusedLeaf =
    activeTabOf(state, wsId)?.id === tab.id ? activePanelOf(state, wsId) : null
  return leafIds(tab.layout).map((panelId) => {
    // Session restore gates only the DIRECTORY (surfaces open without their
    // saved cwd then); the focused FLAG still describes real focus. And the
    // flag is independent of dirs on purpose — background tabs keep listing
    // their branches, just none bold.
    let dir: string | null = null
    if (sessionRestoreEnabled) {
      const meta = panels?.find((p) => p.id === panelId)
      dir = meta == null || meta.sshTarget != null ? null : (meta.workingDirectory ?? null)
    }
    return { panelId, dir, focused: panelId === focusedLeaf }
  })
}
