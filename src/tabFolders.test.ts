// tabFolders — folder lines for workspace rows (v1.6.0 / #81).
//
// Assumptions encoded:
//  - Input: a WorkspaceState with open workspaces, the live per-panel status
//    map (same shape the workspace-row chips read), the session-restore flag
//    (a panel without restore has no saved cwd to show), and the agent-status
//    flag. The panel speaking for a tab is its focused panel when the tab is
//    active, otherwise its first panel (the same focus rule the branch
//    labels use). No agent → status 'idle'.
//  - Output per open workspace, TWO modes (quickupdate round 2, Adam):
//    agent status ON (default) → EXACTLY one line per tab, nothing merges —
//    each line carries its tab's live chip; agent status OFF → one line per
//    DISTINCT folder (tabs sharing a folder merge at the folder's first tab
//    position), chip-only lines never merge.
//  - SSH-backed panels have no local folder → folder null (chip-only line).
//  - NOT tested here: rendering (WorkspaceShell) and live cwd snapshots.

import { describe, it, expect } from 'vitest'
import { tabFolderLines, formatFolderTail } from './tabFolders'
import type { Workspace, WorkspaceState } from './workspaces'

const state = (workspaces: WorkspaceState['workspaces']): WorkspaceState =>
  ({
    workspaces,
    activeId: 'ws-1',
    openIds: ['ws-1'],
    activeTabId: { 'ws-1': 't-1' },
    activePanelId: { 'ws-1': 'p-1' },
  }) as unknown as WorkspaceState

const ws1: Workspace = {
  id: 'ws-1',
  name: 'alpha',
  panels: [
    { id: 'p-1', workingDirectory: '/repo' },
    { id: 'p-2', workingDirectory: '/repo' },
    { id: 'p-3', workingDirectory: '/srv', sshTarget: 'a@h' },
    { id: 'p-4' },
  ],
  tabs: [
    { id: 't-1', name: 'one', layout: { kind: 'leaf', id: 'p-1' } },
    { id: 't-2', name: 'two', layout: { kind: 'leaf', id: 'p-2' } },
    { id: 't-3', name: 'three', layout: { kind: 'leaf', id: 'p-3' } },
    { id: 't-4', name: 'four', layout: { kind: 'leaf', id: 'p-4' } },
  ],
}

describe('tabFolderLines', () => {
  // Agent status ON (the default): every tab keeps its own line and chip.
  it('renders exactly one line per tab when agent status is ON (nothing merges)', () => {
    const lines = tabFolderLines(state([ws1]), {}, true)['ws-1']

    expect(lines).toHaveLength(4)
    expect(lines.filter((l) => l.folder === '/repo')).toHaveLength(2)
  })

  it('pairs each line with the tab and the panel speaking for it', () => {
    const lines = tabFolderLines(state([ws1]), {}, true)['ws-1']

    expect(lines.map((l) => l.tabId)).toEqual(['t-1', 't-2', 't-3', 't-4'])
    expect(lines[0].panelId).toBe('p-1')
  })

  it('a tab without an agent still gets its line, status idle', () => {
    const lines = tabFolderLines(state([ws1]), {}, true)['ws-1']

    expect(lines[0].status).toBe('idle')
  })

  it('carries the live status of the speaking panel', () => {
    const lines = tabFolderLines(state([ws1]), { 'p-1': 'working' }, true)['ws-1']

    expect(lines[0].status).toBe('working')
  })

  it('an SSH panel yields a chip-only line (no local folder)', () => {
    const lines = tabFolderLines(state([ws1]), {}, true)['ws-1']

    expect(lines[2]).toMatchObject({ tabId: 't-3', folder: null })
  })

  it('a panel without a recorded cwd yields folder null', () => {
    const lines = tabFolderLines(state([ws1]), {}, true)['ws-1']

    expect(lines[3]).toMatchObject({ tabId: 't-4', folder: null })
  })

  // Agent status OFF: pure folder names — duplicates merge (quickupdate
  // round 2, supersedes never-merge).
  it('merges tabs sharing a folder into ONE line when agent status is OFF', () => {
    const lines = tabFolderLines(state([ws1]), {}, true, false)['ws-1']

    // t-1 and t-2 both sit in /repo → one /repo line; t-3 (SSH) and t-4 (no
    // cwd) keep their own chip-only lines.
    expect(lines).toHaveLength(3)
    expect(lines.filter((l) => l.folder === '/repo')).toHaveLength(1)
  })

  it('the merged line sits at the folder\'s FIRST tab position', () => {
    const lines = tabFolderLines(state([ws1]), {}, true, false)['ws-1']

    expect(lines.map((l) => l.tabId)).toEqual(['t-1', 't-3', 't-4'])
    expect(lines[0]).toMatchObject({ tabId: 't-1', panelId: 'p-1', folder: '/repo' })
  })

  it('chip-only lines never merge even when folders dedupe', () => {
    const lines = tabFolderLines(state([ws1]), {}, true, false)['ws-1']

    expect(lines[1]).toMatchObject({ tabId: 't-3', folder: null })
    expect(lines[2]).toMatchObject({ tabId: 't-4', folder: null })
  })

  it('with session restore off no saved cwd poses as the folder', () => {
    const lines = tabFolderLines(state([ws1]), {}, false)['ws-1']

    expect(lines.every((l) => l.folder === null)).toBe(true)
  })

  it('a closed workspace gets no lines', () => {
    const lines = tabFolderLines(
      state([{ ...ws1, id: 'ws-2' }]),
      {},
      true,
    )['ws-2']

    expect(lines).toBeUndefined()
  })
})

// Display form (#81 HITL round): the row shows the TAIL of the path —
// <parent folder>/<target folder> — never the full path from the root; the
// full path stays on the tooltip. A path with a single segment shows just it.
describe('formatFolderTail', () => {
  it('shows parent/target for a deep path', () => {
    expect(formatFolderTail('/Users/panad/Documents/umux')).toBe('Documents/umux')
  })

  it('shows parent/target for a two-segment path', () => {
    expect(formatFolderTail('/repo/sub')).toBe('repo/sub')
  })

  it('shows the bare segment when there is no parent', () => {
    expect(formatFolderTail('/repo')).toBe('repo')
    expect(formatFolderTail('repo')).toBe('repo')
  })

  it('ignores a trailing slash', () => {
    expect(formatFolderTail('/Users/panad/work/')).toBe('panad/work')
  })

  // #81 Windows fix: the snapshot on Windows records the tab's starting
  // directory as a NATIVE path — `C:\Users\...` with backslashes. The tail
  // must treat `\` as a separator too, or the line shows the whole walk
  // from the drive letter instead of the last two segments.
  it('tails Windows backslash paths to the last two segments', () => {
    expect(formatFolderTail('C:\\Users\\panad\\Documents\\umux')).toBe('Documents\\umux')
    expect(formatFolderTail('C:\\repo\\sub')).toBe('repo\\sub')
  })

  it('shows the bare drive as-is for a Windows root path', () => {
    expect(formatFolderTail('C:\\')).toBe('C:')
  })

  it('returns null for a null folder (chip-only line stays chip-only)', () => {
    expect(formatFolderTail(null)).toBeNull()
  })
})
