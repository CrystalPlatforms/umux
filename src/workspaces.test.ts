// Assumptions encoded by these tests (Phase 6 / Issue #7):
//  - Workspace shape: { id: string; name: string }. Order of workspaces is the
//    array order in `state.workspaces`.
//  - WorkspaceState: { workspaces: Workspace[]; activeId: string | null }.
//  - `activeId` is runtime-only — NOT persisted (it points at a live shell).
//  - createWorkspace takes an injectable id generator (`genId`) so randomness
//    stays a system boundary and tests are deterministic.
//  - Boundary NOT tested in this file: disk persistence (Rust WorkspaceStore),
//    terminal rendering, PTY I/O.
//
// Phase 7 / Issue #8 additions (open/closed model):
//  - WorkspaceState gains `openIds: string[]` — runtime-only (NOT persisted,
//    same as activeId). A workspace is "open" while it has a live, mounted
//    panel (and thus a live shell); "closed" keeps its definition but unmounts
//    the panel, killing the shell.
//  - A newly created workspace is open from the start (in openIds) and active.
//  - Deleting or closing the active workspace hands activation to the next open
//    sibling, else the previous open one, else null (EmptyState).

import { describe, it, expect } from 'vitest'
import {
  emptyState,
  createWorkspace,
  listWorkspaces,
  renameWorkspace,
  switchWorkspace,
  deleteWorkspace,
  closeWorkspace,
  openWorkspace,
  moveWorkspace,
  splitPanel,
} from './workspaces'

describe('workspace state', () => {
  describe('createWorkspace', () => {
    it('adds the workspace and makes it the active one', () => {
      const state = emptyState

      const next = createWorkspace(state, 'my-project', () => 'ws-1')

      expect(next.workspaces).toEqual([
        { id: 'ws-1', name: 'my-project', panels: [] },
      ])
      expect(next.activeId).toBe('ws-1')
    })

    it('opens the new workspace (adds it to openIds)', () => {
      const next = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(next.openIds).toEqual(['ws-1'])
    })

    // Phase 8 / #9 — forward-compat schema: a fresh workspace starts with an
    // empty panels slot so the persisted shape already matches the model
    // Phase 9 will populate (and stays byte-identical to the Rust Workspace).
    it('creates the workspace with an empty panels slot', () => {
      const next = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(next.workspaces[0].panels).toEqual([])
    })

    // Phase 9 / #10 — a fresh workspace starts with a single-panel layout so
    // the shell area is filled immediately. Runtime-only (NOT persisted, like
    // openIds): a reload re-seeds single (persistence is story 37).
    it('seeds a single-panel layout for the new workspace', () => {
      const next = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(next.layouts['ws-1']).toEqual({ kind: 'single' })
    })
  })

  describe('splitPanel', () => {
    // T-AC story 15/17 — splitting a single workspace sets a split layout.
    it('splits a workspace into a horizontal split layout', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      const next = splitPanel(state, 'ws-1', 'horizontal')

      expect(next.layouts['ws-1']).toEqual({
        kind: 'split',
        orientation: 'horizontal',
      })
    })

    it('splits a workspace into a vertical split layout', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      const next = splitPanel(state, 'ws-1', 'vertical')

      expect(next.layouts['ws-1']).toEqual({
        kind: 'split',
        orientation: 'vertical',
      })
    })

    // T-AC story 16 — two-panel max: a second split is rejected (no-op).
    it('is a no-op when the workspace is already split', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')
      const once = splitPanel(state, 'ws-1', 'horizontal')

      const twice = splitPanel(once, 'ws-1', 'vertical')

      expect(twice).toBe(once)
      expect(twice.layouts['ws-1']).toEqual({
        kind: 'split',
        orientation: 'horizontal',
      })
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(splitPanel(state, 'nope', 'horizontal')).toBe(state)
    })
  })

  describe('listWorkspaces', () => {
    it('returns every workspace in array order', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      expect(listWorkspaces(state).map((w) => w.name)).toEqual(['alpha', 'beta'])
    })
  })

  describe('renameWorkspace', () => {
    it('changes the name of the matching workspace and keeps the rest', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = renameWorkspace(state, 'ws-1', 'alpha-renamed')

      expect(listWorkspaces(next).map((w) => w.name)).toEqual([
        'alpha-renamed',
        'beta',
      ])
    })

    it('is a no-op for an unknown id', () => {
      const state = createWorkspace(emptyState, 'alpha', () => 'ws-1')

      const next = renameWorkspace(state, 'nope', 'x')

      expect(next).toBe(state)
    })
  })

  describe('switchWorkspace', () => {
    it('changes the active workspace without touching the list', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      // creating beta made it active; switch back to alpha

      const next = switchWorkspace(state, 'ws-1')

      expect(next.activeId).toBe('ws-1')
      expect(listWorkspaces(next).map((w) => w.id)).toEqual(['ws-1', 'ws-2'])
    })

    it('is a no-op for an unknown id', () => {
      const state = createWorkspace(emptyState, 'alpha', () => 'ws-1')

      const next = switchWorkspace(state, 'nope')

      expect(next).toBe(state)
    })
  })

  describe('deleteWorkspace', () => {
    it('removes the workspace from definitions and from openIds', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = deleteWorkspace(state, 'ws-1')

      expect(listWorkspaces(next).map((w) => w.id)).toEqual(['ws-2'])
      expect(next.openIds).toEqual(['ws-2'])
    })

    it('hands activation to the next open sibling when the active one is deleted', () => {
      // [alpha(open) beta(open,active) gamma(open)] -- delete beta -> gamma active
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')

      const next = deleteWorkspace(state, 'ws-3')

      expect(next.activeId).toBe('ws-2')
    })

    it('falls back to the previous open sibling when there is no next one', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      // active is beta (ws-2); delete it -> fall back to alpha (ws-1)
      const next = deleteWorkspace(state, 'ws-2')

      expect(next.activeId).toBe('ws-1')
    })

    it('clears activation when the last open workspace is deleted', () => {
      const state = createWorkspace(emptyState, 'solo', () => 'ws-1')

      const next = deleteWorkspace(state, 'ws-1')

      expect(next.activeId).toBeNull()
      expect(next.openIds).toEqual([])
    })

    it('is a no-op for an unknown id', () => {
      const state = createWorkspace(emptyState, 'alpha', () => 'ws-1')

      const next = deleteWorkspace(state, 'nope')

      expect(next).toBe(state)
    })
  })

  describe('closeWorkspace', () => {
    it('keeps the definition but removes the workspace from openIds', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = closeWorkspace(state, 'ws-1')

      expect(listWorkspaces(next).map((w) => w.id)).toEqual(['ws-1', 'ws-2'])
      expect(next.openIds).toEqual(['ws-2'])
    })

    it('hands activation to the next open sibling when the active one is closed', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')

      const next = closeWorkspace(state, 'ws-3')

      expect(next.activeId).toBe('ws-2')
    })

    it('is a no-op for an already-closed workspace', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = closeWorkspace(state, 'ws-1') // alpha now closed but still defined

      const next = closeWorkspace(state, 'ws-1')

      expect(next).toBe(state)
    })

    it('is a no-op for an unknown id', () => {
      const state = createWorkspace(emptyState, 'alpha', () => 'ws-1')

      const next = closeWorkspace(state, 'nope')

      expect(next).toBe(state)
    })
  })

  describe('openWorkspace', () => {
    it('activates an already-open workspace', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      // beta is active; open alpha again -> alpha active, openIds unchanged
      const next = openWorkspace(state, 'ws-1')

      expect(next.activeId).toBe('ws-1')
      expect(next.openIds).toEqual(['ws-1', 'ws-2'])
    })

    it('reopens a closed workspace (idempotent if already open)', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = closeWorkspace(state, 'ws-1') // alpha closed but still defined

      const next = openWorkspace(state, 'ws-1')

      expect(next.activeId).toBe('ws-1')
      expect(next.openIds).toEqual(['ws-2', 'ws-1'])
    })

    it('is a no-op for an unknown id', () => {
      const state = createWorkspace(emptyState, 'alpha', () => 'ws-1')

      const next = openWorkspace(state, 'nope')

      expect(next).toBe(state)
    })
  })

  describe('moveWorkspace', () => {
    it('moves a workspace to a new position in the list', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')

      // drag alpha (index 0) to the end (index 2)
      const next = moveWorkspace(state, 'ws-1', 2)

      expect(listWorkspaces(next).map((w) => w.id)).toEqual([
        'ws-2',
        'ws-3',
        'ws-1',
      ])
    })

    it('clamps an out-of-range index to the end of the list', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = moveWorkspace(state, 'ws-1', 99)

      expect(listWorkspaces(next).map((w) => w.id)).toEqual(['ws-2', 'ws-1'])
    })

    it('does not touch openIds (reorder is definitions-only)', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = moveWorkspace(state, 'ws-1', 1)

      expect(next.openIds).toEqual(['ws-1', 'ws-2'])
    })

    it('is a no-op for an unknown id', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = moveWorkspace(state, 'nope', 1)

      expect(next).toBe(state)
    })
  })
})
