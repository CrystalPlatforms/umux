// tabFolders — per-tab folder lines for workspace rows (v1.6.0 / #81).
//
// Assumptions encoded:
//  - Input: a WorkspaceState with open workspaces, the live per-panel status
//    map (same shape the workspace-row chips read), and the
//    session-restore flag (a panel without restore has no saved cwd to show).
//  - Output: per open workspace, EXACTLY one line per tab (never merged —
//    duplicate folders stay separate lines, explicit 2026-08-31 decision):
//    { tabId, panelId, folder, status }. The panel speaking for a tab is its
//    focused panel when the tab is active, otherwise its first panel (the
//    same focus rule the branch labels use). No agent → status 'idle'.
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
  it('renders exactly one line per tab — duplicates NOT merged (#81)', () => {
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

  it('returns null for a null folder (chip-only line stays chip-only)', () => {
    expect(formatFolderTail(null)).toBeNull()
  })
})
