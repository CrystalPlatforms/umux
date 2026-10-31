// tabFolders — folder lines for workspace rows (v1.6.0 / #81).
//
// Pure module, sibling of tabBranch: the Settings switch `showTabFolders`
// adds lines under the workspace's name — an agent chip followed by the
// folder a terminal panel sits in. Explicit decisions pinned here:
//   - with agent status ON (quickupdate round 2, Adam): EVERY tab keeps its
//     own line and its own chip — nothing merges, because each line carries
//     that tab's live status; #121 refines this to PER PANEL: a split tab
//     contributes one line per terminal, so a workspace shows as many Status
//     Agents as it runs split terminals (each bound to its own panel, in
//     tree order — the same per-panel rule the branch labels use, v1.0);
//   - with agent status OFF: panels sharing a folder merge into ONE line —
//     pure folder names, duplicates are noise (quickupdate 2026-09-12,
//     supersedes the 2026-08-31 never-merge rule). The merged line sits at
//     the folder's FIRST panel position; chip-only lines (no folder) never
//     merge;
//   - SSH-backed panels have no local folder → a chip-only line;
//   - with session restore off (#27) the saved cwd must not pose as the
//     folder, so every line is chip-only;
//   - a panel without an agent still gets its line (status 'idle').

import type { AgentStatus } from './agentStatus'
import { branchDirsByTab } from './tabBranch'
import type { WorkspaceState } from './workspaces'

export type TabFolderLine = {
  tabId: string
  panelId: string
  /// The folder shown after the chip; null = chip-only line (SSH panel, no
  /// recorded cwd, or session restore off).
  folder: string | null
  status: AgentStatus
}

/// Every open workspace's folder lines (workspace id → lines). With agent
/// status ON every PANEL keeps its own line (#121 — a split tab yields as
/// many lines as it holds terminals, each carrying that panel's live chip
/// and folder); with agent status OFF panels sharing a folder merge into one
/// line at the folder's first panel position, and chip-only lines (folder
/// null) stay one per panel. `statuses` is the live per-panel status map the
/// workspace-row chips already render from.
export function tabFolderLines(
  state: WorkspaceState,
  statuses: Record<string, AgentStatus>,
  sessionRestoreEnabled = true,
  agentStatusEnabled = true,
): Record<string, TabFolderLine[]> {
  const byTab = branchDirsByTab(state, sessionRestoreEnabled)
  const byWs: Record<string, TabFolderLine[]> = {}
  for (const ws of state.workspaces) {
    if (!state.openIds.includes(ws.id)) continue
    const lines: TabFolderLine[] = (ws.tabs ?? []).flatMap((tab) =>
      // #121: one line per PANEL, not per tab — the focused-first pick is
      // gone; every split terminal speaks for itself, in tree order.
      (byTab[tab.id] ?? []).map((entry) => ({
        tabId: tab.id,
        panelId: entry.panelId,
        folder: entry.dir ?? null,
        status: statuses[entry.panelId] ?? 'idle',
      })),
    )
    if (agentStatusEnabled) {
      // Agent status ON: every line carries its tab's live chip — merging
      // would hide a tab's status, so nothing merges.
      byWs[ws.id] = lines
      continue
    }
    // Agent status OFF: pure folder names — merge duplicates (quickupdate
    // 2026-09-12, supersedes never-merge). The folder's FIRST occurrence
    // fixes the line's position. Full-path identity — two different folders
    // with the same displayed tail stay two lines.
    const emitted = new Set<string>()
    byWs[ws.id] = lines.filter((line) => {
      if (line.folder == null) return true
      if (emitted.has(line.folder)) return false
      emitted.add(line.folder)
      return true
    })
  }
  return byWs
}

/// The display form of a folder line (#81 HITL round): the TAIL of the path —
/// "<parent folder>/<target folder>" — never the walk from the root (the
/// user wants to SEE the folder the terminals sit in, not /Users/...). The
/// full path stays available for the tooltip. A single-segment path shows
/// just the segment; null stays null (chip-only line).
export function formatFolderTail(folder: string | null): string | null {
  if (folder == null) return null
  // Both separators: Windows snapshots (#81 fix) record native backslash
  // paths, and the tail rule is the same there — last two segments, joined
  // with the path's own separator, never the walk from the drive letter.
  const segments = folder.split(/[\\/]/).filter((s) => s !== '')
  if (segments.length === 0) return folder
  const sep = folder.includes('\\') ? '\\' : '/'
  return segments.slice(-2).join(sep)
}
