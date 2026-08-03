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
  resizePanel,
  closePanel,
  focusPanel,
  activePanelOf,
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
        ratio: 0.5,
      })
    })

    it('splits a workspace into a vertical split layout', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      const next = splitPanel(state, 'ws-1', 'vertical')

      expect(next.layouts['ws-1']).toEqual({
        kind: 'split',
        orientation: 'vertical',
        ratio: 0.5,
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
        ratio: 0.5,
      })
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(splitPanel(state, 'nope', 'horizontal')).toBe(state)
    })
  })

  describe('panel identity (split keeps shells, close keeps the survivor)', () => {
    it('seeds one stable panel id when a workspace is created', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(state.panelIds['ws-1']).toHaveLength(1)
    })

    it('keeps the existing panel id and adds a new one on split', () => {
      let state = createWorkspace(emptyState, 'my-project', () => 'ws-1')
      const firstPanel = state.panelIds['ws-1'][0]
      // Force deterministic ids for the second panel: workspace id is ws-1, so
      // the next genId call here produces the second panel id.
      state = splitPanel(state, 'ws-1', 'horizontal', () => 'panel-B')

      expect(state.panelIds['ws-1']).toEqual([firstPanel, 'panel-B'])
    })

    it('keeps the surviving panel id when one panel is closed', () => {
      let state = createWorkspace(emptyState, 'my-project', () => 'ws-1')
      state = splitPanel(state, 'ws-1', 'horizontal', () => 'panel-B')
      const [survivor] = state.panelIds['ws-1']

      // Close the second panel -> only the survivor remains.
      const next = closePanel(state, 'ws-1', 'panel-B')

      expect(next.panelIds['ws-1']).toEqual([survivor])
      expect(next.layouts['ws-1']).toEqual({ kind: 'single' })
    })

    it('keeps the other panel when the first one is closed', () => {
      let state = createWorkspace(emptyState, 'my-project', () => 'ws-1')
      const firstPanel = state.panelIds['ws-1'][0]
      state = splitPanel(state, 'ws-1', 'horizontal', () => 'panel-B')

      const next = closePanel(state, 'ws-1', firstPanel)

      expect(next.panelIds['ws-1']).toEqual(['panel-B'])
    })
  })

  describe('resizePanel', () => {
    it('updates the split ratio, clamped by the container and minimum', () => {
      const state = splitPanel(
        createWorkspace(emptyState, 'my-project', () => 'ws-1'),
        'ws-1',
        'horizontal',
      )

      const next = resizePanel(state, 'ws-1', 0.25, { width: 100, height: 50 }, 10)

      expect(next.layouts['ws-1']).toEqual({
        kind: 'split',
        orientation: 'horizontal',
        ratio: 0.25,
      })
    })

    it('is a no-op on a single (un-split) workspace', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      // single layout has no divider — resize returns the same state object.
      expect(resizePanel(state, 'ws-1', 0.3, { width: 100, height: 50 })).toBe(state)
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(resizePanel(state, 'nope', 0.3, { width: 100, height: 50 })).toBe(state)
    })
  })

  describe('closePanel', () => {
    it('collapses a split workspace back to a single panel', () => {
      let state = createWorkspace(emptyState, 'my-project', () => 'ws-1')
      state = splitPanel(state, 'ws-1', 'horizontal', () => 'panel-B')

      const next = closePanel(state, 'ws-1', 'panel-B')

      expect(next.layouts['ws-1']).toEqual({ kind: 'single' })
    })

    it('is a no-op when the workspace only has one panel', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(closePanel(state, 'ws-1', state.panelIds['ws-1'][0])).toBe(state)
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(closePanel(state, 'nope', 'whatever')).toBe(state)
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

  // Phase 11 / #12 — active panel focus (story 34). Each workspace tracks
  // which of its panels is focused; the renderer draws a ring on it. The
  // active panel is runtime-only (NOT persisted), like activeId.
  describe('active panel focus', () => {
    // A helper that builds a split workspace with two known panel ids, so the
    // focus tests have stable panel ids to point at. genId yields ws-1 then
    // p-1 for createWorkspace's workspace+seed-panel, then p-2 for the split.
    function splitState(): ReturnType<typeof splitPanel> {
      const ids = ['ws-1', 'p-1', 'p-2']
      let i = 0
      const state = createWorkspace(
        emptyState,
        'my-project',
        () => ids[i++],
      )
      return splitPanel(state, 'ws-1', 'horizontal', () => ids[i++])
    }

    it('defaults to the first panel when none has been focused', () => {
      // A workspace with panels but no activePanelId entry — e.g. state built
      // before focus was ever set. activePanelOf must fall back to panels[0].
      const state = {
        ...emptyState,
        workspaces: [{ id: 'ws-1', name: 'x', panels: [] }],
        panelIds: { 'ws-1': ['p-1', 'p-2'] },
        activePanelId: {},
      }

      expect(activePanelOf(state, 'ws-1')).toBe('p-1')
    })

    it('focusPanel sets the active panel', () => {
      const state = splitState()

      const next = focusPanel(state, 'ws-1', 'p-2')

      expect(activePanelOf(next, 'ws-1')).toBe('p-2')
    })

    it('focusPanel is a no-op for an unknown panel', () => {
      const state = splitState()

      const next = focusPanel(state, 'ws-1', 'nope')

      expect(next).toBe(state)
    })

    it('splitting makes the new panel active (keystrokes follow the split)', () => {
      // Start single, focused on its only panel, then split.
      const ids = ['ws-1', 'p-1', 'p-2']
      let i = 0
      const single = createWorkspace(emptyState, 'my-project', () => ids[i++])

      const next = splitPanel(single, 'ws-1', 'horizontal', () => ids[i++])

      expect(activePanelOf(next, 'ws-1')).toBe('p-2')
    })

    it('closing the active panel hands focus to the survivor', () => {
      const state = splitState()
      // p-2 is active after the split (above); close it -> p-1 becomes active.
      const focused = focusPanel(state, 'ws-1', 'p-2')

      const next = closePanel(focused, 'ws-1', 'p-2')

      expect(activePanelOf(next, 'ws-1')).toBe('p-1')
    })

    it('closing a non-active panel leaves focus where it was', () => {
      const state = splitState()
      // p-2 active after split; close p-1 -> p-2 still active.
      const focused = focusPanel(state, 'ws-1', 'p-2')

      const next = closePanel(focused, 'ws-1', 'p-1')

      expect(activePanelOf(next, 'ws-1')).toBe('p-2')
    })

    it('deleting a workspace drops its active-panel entry', () => {
      const state = splitState()

      const next = deleteWorkspace(state, 'ws-1')

      expect(next.activePanelId['ws-1']).toBeUndefined()
    })
  })
})
