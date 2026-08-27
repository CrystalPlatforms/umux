// Assumptions encoded by these tests (v1.0 Phase 14 / Issue #41, HITL rework
// 2026-08-27: branch PER PANEL, not one per tab):
//  - A split tab lists an entry for EVERY panel, in tree order — as many
//    branches as splits; entries with dir=null render nothing but keep the
//    panel's identity (no placeholder — the original #41 rule stands).
//  - Exactly ONE entry per tab has focused=true: the FOCUSED panel of the
//    workspace's ACTIVE tab (via activePanelOf's fallback chain — so a boot
//    state without a focus record marks the FIRST panel), never any entry of
//    an inactive tab.
//  - "Starting" = persisted panels[].workingDirectory config; live `cd` stays
//    out of scope (#41); SSH panels and restore-off panels read null.
//  - Boundary NOT tested here: real repo parsing (cargo fixtures) and the
//    invoke wire (glue in WorkspaceShell.tsx).

import { describe, it, expect } from 'vitest'
import {
  emptyState,
  createWorkspace,
  splitPanel,
  focusPanel,
  addTab,
  type WorkspaceState,
} from './workspaces'
import { branchDirsByTab, branchQueryDirs } from './tabBranch'

/// Deterministic id generator over a fixed sequence (tests stay pure).
function seq(...ids: string[]): () => string {
  let i = 0
  return () => ids[i++]
}

/// One workspace ("proj") whose first tab splits into TWO known panels:
/// leaf-a (/repo/ui) and leaf-b (/repo/api). Splitting leaves leaf-b focused
/// (the split hands it focus), so the explicit-focus cases start meaningful.
function twoPanelState(): WorkspaceState {
  const base = createWorkspace(emptyState, 'proj', seq('ws-1', 'tab-1', 'leaf-a'))
  const split = splitPanel(base, 'ws-1', 'horizontal', seq('split-1', 'leaf-b'))
  return [
    ['leaf-a', '/repo/ui'],
    ['leaf-b', '/repo/api'],
  ].reduce(
    (st, [pid, cwd]) => ({ ...st, workspaces: upsert(st, pid, cwd) }),
    split,
  )
}

/// Append/update a panels[] workingDirectory entry on ws-1, mirroring
/// upsertPanelCwd without importing it (keeps this suite's surface small).
function upsert(state: WorkspaceState, panelId: string, cwd: string) {
  return state.workspaces.map((w) =>
    w.id !== 'ws-1'
      ? w
      : {
          ...w,
          panels: [...(w.panels ?? []).filter((p) => p.id !== panelId), { id: panelId, workingDirectory: cwd }],
        },
  )
}

describe('tab branch label mapping', () => {
  describe('branchDirsByTab', () => {
    // One entry PER PANEL in tree order; the focused one flagged (here the
    // split handed focus to leaf-b).
    it('lists every panel of the active tab, focused flagged', () => {
      const state = twoPanelState()
      expect(branchDirsByTab(state)).toEqual({
        'tab-1': [
          { panelId: 'leaf-a', dir: '/repo/ui', focused: false },
          { panelId: 'leaf-b', dir: '/repo/api', focused: true },
        ],
      })
    })

    // AC: switching focused panel within the tab MOVES the bold flag (the
    // number of entries stays).
    it('follows focus when it moves back to the first panel', () => {
      const state = focusPanel(twoPanelState(), 'ws-1', 'leaf-a')
      expect(branchDirsByTab(state)).toEqual({
        'tab-1': [
          { panelId: 'leaf-a', dir: '/repo/ui', focused: true },
          { panelId: 'leaf-b', dir: '/repo/api', focused: false },
        ],
      })
    })

    // Fresh boot / stale record: activePanelOf's first-panel fallback marks
    // the FIRST entry focused (exactly one per tab, always).
    it('falls back to marking the first panel when no explicit focus exists', () => {
      const state = { ...twoPanelState(), activePanelId: {} }
      const entries = branchDirsByTab(state)['tab-1'] ?? []
      expect(entries.filter((e) => e.focused)).toEqual([
        { panelId: 'leaf-a', dir: '/repo/ui', focused: true },
      ])
    })

    // Focus records are scoped to the ACTIVE tab: an inactive tab still lists
    // all its panels but carries NO focused flag at all.
    it('non-active tabs list panels with nothing focused', () => {
      const st = twoPanelState()
      const second = addTab(st, 'ws-1', seq('tab-2', 'leaf-c'))
      const state = {
        ...second,
        workspaces: upsert({ ...second }, 'leaf-c', '/repo/api'),
      }
      const dirs = branchDirsByTab(state)
      // The just-added tab IS active now: its new single panel is focused…
      expect(dirs['tab-2']).toEqual([
        { panelId: 'leaf-c', dir: '/repo/api', focused: true },
      ])
      // …while the first tab keeps its list but loses every flag.
      expect(dirs['tab-1']).toEqual([
        { panelId: 'leaf-a', dir: '/repo/ui', focused: false },
        { panelId: 'leaf-b', dir: '/repo/api', focused: false },
      ])
    })

    // AC: SSH-backed panels contribute a null-dir entry (renders nothing),
    // even when a stale cwd is recorded on the same entry.
    it('skips ssh-backed panels via a null dir', () => {
      const st = twoPanelState()
      const state = {
        ...st,
        workspaces: st.workspaces.map((w) =>
          w.id !== 'ws-1'
            ? w
            : {
                ...w,
                panels: (w.panels ?? []).map((p) =>
                  p.id === 'leaf-b' ? { ...p, sshTarget: 'adam@box' } : p,
                ),
              },
        ),
      }
      expect(branchDirsByTab(state)).toEqual({
        'tab-1': [
          { panelId: 'leaf-a', dir: '/repo/ui', focused: false },
          { panelId: 'leaf-b', dir: null, focused: true },
        ],
      })
    })

    // No configured starting directory → null dir for that panel (never the
    // app's own cwd).
    it('yields null dirs for panels without working-directory config', () => {
      const bare = createWorkspace(emptyState, 'proj', seq('ws-1', 'tab-1', 'leaf-a'))
      expect(branchDirsByTab(bare)).toEqual({
        'tab-1': [{ panelId: 'leaf-a', dir: null, focused: true }],
      })
    })

    // Session-restore off (#27 follow-up): surfaces open WITHOUT the saved
    // cwd, so no saved path may pose as a start directory — every dir nulls.
    it('nulls every dir while session restore is off', () => {
      const st = twoPanelState()
      expect(branchDirsByTab(st, false)).toEqual({
        'tab-1': [
          { panelId: 'leaf-a', dir: null, focused: false },
          { panelId: 'leaf-b', dir: null, focused: true },
        ],
      })
    })

    // AC: tabs of CLOSED workspaces are not queried at all.
    it('ignores closed workspaces', () => {
      const st = twoPanelState()
      const closedState = { ...st, openIds: [] }
      expect(branchDirsByTab(closedState)).toEqual({})
    })
  })

  describe('branchQueryDirs', () => {
    // One invoke payload for every panel of every open tab at once: distinct
    // directories only, nulls dropped.
    it('deduplicates directories across tabs and drops null entries', () => {
      const st = twoPanelState()
      const second = addTab(st, 'ws-1', seq('tab-2', 'leaf-c'))
      let state = second
      for (const [pid, cwd] of [
        ['leaf-a', '/repo/ui'],
        ['leaf-b', '/repo/api'],
        ['leaf-c', '/repo/api'],
      ] as [string, string][]) {
        state = { ...state, workspaces: upsert(state, pid, cwd) }
      }
      // Order follows first appearance walking tabs/panels.
      expect(branchQueryDirs(state)).toEqual(['/repo/ui', '/repo/api'])
    })

    it('query list drops everything while session restore is off', () => {
      const st = twoPanelState()
      expect(branchQueryDirs(st, false)).toEqual([])
    })
  })
})
