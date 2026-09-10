// tabFolders — per-tab folder lines for workspace rows (v1.6.0 / #81).
//
// Pure module, sibling of tabBranch: the Settings switch `showTabFolders`
// adds ONE line per tab under the workspace's name — that tab's agent chip
// followed by the folder its shell sits in. Explicit decisions pinned here:
//   - duplicates are NEVER merged — one line per tab keeps the tab↔folder
//     mapping unambiguous (2026-08-31 decision);
//   - the panel speaking for a tab is its focused panel when the tab is
//     active, otherwise its first panel — the same rule the branch labels
//     use (tabBranch.branchEntriesOfTab);
//   - SSH-backed panels have no local folder → a chip-only line;
//   - with session restore off (#27) the saved cwd must not pose as the
//     folder, so every line is chip-only;
//   - a tab without an agent still gets its line (status 'idle').

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

/// Every open workspace's tabs mapped to their folder line (workspace id →
/// lines in tab order). `statuses` is the live per-panel status map the
/// workspace-row chips already render from.
export function tabFolderLines(
  state: WorkspaceState,
  statuses: Record<string, AgentStatus>,
  sessionRestoreEnabled = true,
): Record<string, TabFolderLine[]> {
  const byTab = branchDirsByTab(state, sessionRestoreEnabled)
  const byWs: Record<string, TabFolderLine[]> = {}
  for (const ws of state.workspaces) {
    if (!state.openIds.includes(ws.id)) continue
    byWs[ws.id] = (ws.tabs ?? []).map((tab) => {
      const entries = byTab[tab.id] ?? []
      const entry =
        entries.find((e) => e.focused) ?? (entries.length > 0 ? entries[0] : null)
      return {
        tabId: tab.id,
        panelId: entry?.panelId ?? '',
        folder: entry?.dir ?? null,
        status: (entry != null ? statuses[entry.panelId] : undefined) ?? 'idle',
      }
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
