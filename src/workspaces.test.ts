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
  addTab,
  closeTab,
  switchTab,
  renameTab,
  setTabPinned,
  activeTabOf,
  moveWorkspace,
  setWorkspacePinned,
  splitPanel,
  resizePanel,
  closePanel,
  focusPanel,
  activePanelOf,
  bootState,
  panelIdsOf,
  upsertPanelCwd,
  toggleZoom,
  zoomedPanelOf,
} from './workspaces'
import { leafIds, type LayoutNode } from './PaneLayout'

// v0.2 Phase 1 / #25 — the layout moved INTO the Workspace (a persisted split
// tree; leaf ids ARE panel ids). WorkspaceState no longer carries runtime
// layouts/panelIds records: panel identity derives from workspaces[].layout.

/// Deterministic id generator over a fixed sequence (tests stay pure).
function seq(...ids: string[]): () => string {
  let i = 0
  return () => ids[i++]
}

/// The first workspace's ACTIVE TAB's layout — the post-rework location of
/// what these tests used to read as `workspaces[0].layout`.
function layoutOf(state: Parameters<typeof activeTabOf>[0]): LayoutNode {
  const tab = activeTabOf(state, 'ws-1')
  if (tab == null) throw new Error('no tab in test state')
  return tab.layout
}

describe('workspace state', () => {
  describe('createWorkspace', () => {
    it('adds the workspace and makes it the active one', () => {
      const state = emptyState

      const next = createWorkspace(state, 'my-project', seq('ws-1', 'tab-1', 'p-1'))

      // #37 rework: the seed tree hangs off the workspace's FIRST TAB.
      expect(next.workspaces).toEqual([
        {
          id: 'ws-1',
          name: 'my-project',
          panels: [],
          tabs: [{ id: 'tab-1', layout: { kind: 'leaf', id: 'p-1' }, name: 'Tab 1' }],
        },
      ])
      expect(next.activeId).toBe('ws-1')
      expect(next.activeTabId['ws-1']).toBe('tab-1')
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

    // v0.2 / #25 — a fresh workspace starts with a single-leaf layout tree
    // PERSISTED in the workspace itself (genId: ws id, then panel id).
    it('seeds a single-leaf layout tree inside the new workspace', () => {
      const ids = ['ws-1', 'tab-1', 'p-1']
      let i = 0
      const next = createWorkspace(emptyState, 'my-project', () => ids[i++])

      expect(layoutOf(next)).toEqual({ kind: 'leaf', id: 'p-1' })
      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-1'])
    })
  })

  describe('splitPanel', () => {
    // v0.2 / #25: splitPanel splits the ACTIVE panel's leaf; genId yields the
    // split-node id then the new leaf id.
    function single(): ReturnType<typeof createWorkspace> {
      const ids = ['ws-1', 'tab-1', 'p-1']
      let i = 0
      return createWorkspace(emptyState, 'my-project', () => ids[i++])
    }

    it('splits the active panel into a horizontal split tree', () => {
      const gen = seq('s-1', 'p-2')
      const next = splitPanel(single(), 'ws-1', 'horizontal', gen)

      expect(layoutOf(next)).toEqual({
        kind: 'split',
        id: 's-1',
        orientation: 'horizontal',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'p-1' },
        second: { kind: 'leaf', id: 'p-2' },
      } satisfies LayoutNode)
    })

    it('splits into a vertical split tree', () => {
      const gen = seq('s-1', 'p-2')
      const next = splitPanel(single(), 'ws-1', 'vertical', gen)

      expect(layoutOf(next)).toEqual({
        kind: 'split',
        id: 's-1',
        orientation: 'vertical',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'p-1' },
        second: { kind: 'leaf', id: 'p-2' },
      } satisfies LayoutNode)
    })

    // T-AC v0.2 — unlimited panels: a second split of the (now active) new
    // panel nests instead of being rejected. The v0.1 two-panel cap is gone.
    it('splits again with no panel cap (mixed directions)', () => {
      const ids = ['s-1', 'p-2', 's-2', 'p-3']
      let i = 0
      const gen = () => ids[i++]
      const once = splitPanel(single(), 'ws-1', 'horizontal', gen)
      const twice = splitPanel(once, 'ws-1', 'vertical', gen)

      expect(leafIds(layoutOf(twice)!)).toEqual(['p-1', 'p-2', 'p-3'])
      expect(layoutOf(twice)).toEqual({
        kind: 'split',
        id: 's-1',
        orientation: 'horizontal',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'p-1' },
        second: {
          kind: 'split',
          id: 's-2',
          orientation: 'vertical',
          ratio: 0.5,
          first: { kind: 'leaf', id: 'p-2' },
          second: { kind: 'leaf', id: 'p-3' },
        },
      } satisfies LayoutNode)
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', () => 'ws-1')

      expect(splitPanel(state, 'nope', 'horizontal')).toBe(state)
    })
  })

  describe('panel identity (split keeps shells, close keeps the survivor)', () => {
    it('derives panel ids from the layout tree', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))

      expect(panelIdsOf(state, 'ws-1')).toEqual(['p-1'])
    })

    it('keeps the existing panel id and adds a new one on split', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))

      const next = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-1', 'p-B'])
    })

    it('keeps the surviving panel id when one panel is closed', () => {
      let state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))
      state = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      const next = closePanel(state, 'ws-1', 'p-B')

      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-1'])
      expect(layoutOf(next)).toEqual({ kind: 'leaf', id: 'p-1' })
    })

    it('keeps the other panel when the first one is closed', () => {
      let state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))
      state = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      const next = closePanel(state, 'ws-1', 'p-1')

      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-B'])
    })
  })

  describe('resizePanel', () => {
    // v0.2 / #25: the divider is addressed by its SPLIT id; the container is
    // that split node's own measured extent.
    function splitOnce(): ReturnType<typeof splitPanel> {
      return splitPanel(
        createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1')),
        'ws-1',
        'horizontal',
        seq('s-1', 'p-2'),
      )
    }

    it('updates the targeted split ratio, clamped by the container and minimum', () => {
      const next = resizePanel(splitOnce(), 'ws-1', 's-1', 0.25, { width: 100, height: 50 }, 10)

      expect(layoutOf(next)).toEqual({
        kind: 'split',
        id: 's-1',
        orientation: 'horizontal',
        ratio: 0.25,
        first: { kind: 'leaf', id: 'p-1' },
        second: { kind: 'leaf', id: 'p-2' },
      } satisfies LayoutNode)
    })

    it('is a no-op for an unknown split id (state object untouched)', () => {
      const state = splitOnce()

      expect(resizePanel(state, 'ws-1', 'nope', 0.3, { width: 100, height: 50 })).toBe(state)
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))

      expect(resizePanel(state, 'nope', 's-1', 0.3, { width: 100, height: 50 })).toBe(state)
    })
  })

  describe('closePanel', () => {
    it('collapses a split workspace back to a single panel', () => {
      let state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))
      state = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      const next = closePanel(state, 'ws-1', 'p-B')

      expect(layoutOf(next)).toEqual({ kind: 'leaf', id: 'p-1' })
    })

    it('is a no-op when the workspace only has one panel', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))

      expect(closePanel(state, 'ws-1', 'p-1')).toBe(state)
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))

      expect(closePanel(state, 'nope', 'whatever')).toBe(state)
    })
  })

  // --- Pane zoom (#40 / story 48) --------------------------------------------
  //
  // Zoom is a pure VIEW state: the focused panel of the ACTIVE tab expands to
  // fill the tab area; the same toggle restores the exact previous layout.
  // Assumptions encoded:
  //  - Zoom state is keyed by TAB id in a runtime-only record (like
  //    activeTabId/activePanelId — never persisted, empty after bootState).
  //  - toggleZoom targets the FOCUSED panel of the active tab; an explicit
  //    panelId (the UI button) zooms that panel instead.
  //  - A tab with a single panel is a no-op (nothing to cover).
  //  - The split tree is NEVER touched by zoom — restore is exact because
  //    nothing was mutated.

  describe('pane zoom', () => {
    // A split workspace with stable ids: ws-1 / tab-1 hold p-1 | p-2, and the
    // focused panel after the split is the NEW one (p-2).
    function splitState(): ReturnType<typeof splitPanel> {
      const gen = seq('ws-1', 'tab-1', 'p-1', 's-1', 'p-2')
      const state = createWorkspace(emptyState, 'my-project', gen)
      return splitPanel(state, 'ws-1', 'horizontal', gen)
    }

    it('zooms the focused panel of the active tab', () => {
      const state = splitState()

      const next = toggleZoom(state, 'ws-1')

      expect(zoomedPanelOf(next, 'tab-1')).toBe('p-2')
    })

    it('zooms an explicitly given panel (the UI button path)', () => {
      const state = splitState()

      const next = toggleZoom(state, 'ws-1', 'p-1')

      expect(zoomedPanelOf(next, 'tab-1')).toBe('p-1')
    })

    it('is a no-op on a tab with a single panel', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))

      expect(toggleZoom(state, 'ws-1')).toBe(state)
      expect(toggleZoom(state, 'ws-1', 'p-1')).toBe(state)
    })

    it('is a no-op for an unknown workspace', () => {
      const state = splitState()

      expect(toggleZoom(state, 'nope')).toBe(state)
    })

    it('unzooms on the second toggle and leaves the layout tree untouched', () => {
      const state = splitState()
      const treeBefore = layoutOf(state)

      const zoomed = toggleZoom(state, 'ws-1')
      const restored = toggleZoom(zoomed, 'ws-1')

      expect(zoomedPanelOf(restored, 'tab-1')).toBeNull()
      // The split tree was never mutated — restore is EXACT (same object).
      expect(layoutOf(restored)).toBe(treeBefore)
      expect(restored.workspaces).toBe(state.workspaces)
    })

    it('keeps zoom per tab: another tab is not zoomed', () => {
      // genId: tab-2 + its seed leaf p-3 for addTab, then switch focus back.
      const gen = seq('ws-1', 'tab-1', 'p-1', 's-1', 'p-2', 'tab-2', 'p-3')
      let state = createWorkspace(emptyState, 'my-project', gen)
      state = splitPanel(state, 'ws-1', 'horizontal', gen)
      state = toggleZoom(state, 'ws-1')
      state = addTab(state, 'ws-1', gen)

      // tab-2 is now active and NOT zoomed; tab-1 still is.
      expect(zoomedPanelOf(state, 'tab-2')).toBeNull()
      expect(zoomedPanelOf(state, 'tab-1')).toBe('p-2')
    })

    it('never reaches the persisted config (round-trip has no zoom keys)', () => {
      const state = splitState()
      const configBefore = JSON.stringify(state.workspaces)

      const zoomed = toggleZoom(state, 'ws-1')

      expect(JSON.stringify(zoomed.workspaces)).toBe(configBefore)
      expect(JSON.stringify(zoomed)).not.toContain('"zoom"')
    })

    it('boots with no zoom: every tab renders its normal layout', () => {
      const state = bootState([
        { id: 'ws-1', name: 'a', panels: [], tabs: [{ id: 'tab-1', layout: { kind: 'leaf', id: 'p-1' }, name: 'Tab 1' }] },
      ])

      expect(state.zoomedPanelId).toEqual({})
      expect(zoomedPanelOf(state, 'tab-1')).toBeNull()
    })

    it('exits zoom when the zoomed panel closes (siblings fill normally)', () => {
      const state = splitState()
      const zoomed = toggleZoom(state, 'ws-1')

      const next = closePanel(zoomed, 'ws-1', 'p-2')

      expect(zoomedPanelOf(next, 'tab-1')).toBeNull()
      expect(layoutOf(next)).toEqual({ kind: 'leaf', id: 'p-1' })
    })

    it('keeps zoom when a covered panel closes', () => {
      const state = splitState()
      const zoomed = toggleZoom(state, 'ws-1')

      const next = closePanel(zoomed, 'ws-1', 'p-1')

      expect(zoomedPanelOf(next, 'tab-1')).toBe('p-2')
      expect(layoutOf(next)).toEqual({ kind: 'leaf', id: 'p-2' })
    })

    it('drops the zoom record with the tab (closeTab) and its workspace (deleteWorkspace)', () => {
      // Two tabs, the first one zoomed; gen yields tab-2 + p-3 for addTab.
      const gen = seq('ws-1', 'tab-1', 'p-1', 's-1', 'p-2', 'tab-2', 'p-3')
      let state = createWorkspace(emptyState, 'my-project', gen)
      state = splitPanel(state, 'ws-1', 'horizontal', gen)
      state = toggleZoom(state, 'ws-1')
      state = addTab(state, 'ws-1', gen)

      // Active tab is tab-2 now; close the ZOOMED tab-1 by id.
      const closed = closeTab(state, 'ws-1', 'tab-1')
      expect('tab-1' in closed.zoomedPanelId).toBe(false)

      // And deleting the workspace clears its tabs' records too.
      const deleted = deleteWorkspace(closed, 'ws-1')
      expect(deleted.zoomedPanelId).toEqual({})
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

  // #37 — pinned workspaces. The array IS the display order (the sidebar
  // list and the tab bar both read it directly), so the pinned group is kept
  // as a prefix of the array by MOVING definitions on toggle:
  //   - pinning appends to the END of the pinned block
  //   - unpinning inserts at the HEAD of the unpinned block
  // Assumptions encoded:
  //  - `pinned` is optional on Workspace; absent = unpinned, and an unpinned
  //    workspace carries NO key at all (old saves must not gain one).
  //  - Toggling never touches activeId/openIds (definitions-only, like
  //    moveWorkspace).
  describe('setWorkspacePinned', () => {
    it('moves a pinned workspace to the top of the list', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')

      const next = setWorkspacePinned(state, 'ws-3', true)

      expect(listWorkspaces(next).map((w) => w.id)).toEqual([
        'ws-3',
        'ws-1',
        'ws-2',
      ])
      expect(listWorkspaces(next)[0].pinned).toBe(true)
    })

    it('appends to the end of the pinned block when others are pinned', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')
      state = setWorkspacePinned(state, 'ws-1', true)

      const next = setWorkspacePinned(state, 'ws-3', true)

      // ws-1 pins first, ws-3 joins AFTER it — pinned group keeps a stable,
      // predictable order (the order in which workspaces were pinned).
      expect(listWorkspaces(next).map((w) => w.id)).toEqual([
        'ws-1',
        'ws-3',
        'ws-2',
      ])
    })

    it('keeps the pinned group first as an invariant across several toggles', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')
      state = createWorkspace(state, 'delta', () => 'ws-4')

      state = setWorkspacePinned(state, 'ws-4', true)
      state = setWorkspacePinned(state, 'ws-2', true)
      state = setWorkspacePinned(state, 'ws-3', true)

      const ids = listWorkspaces(state).map((w) => w.id)
      expect(ids.slice(0, 3)).toEqual(['ws-4', 'ws-2', 'ws-3'])
      expect(ids[3]).toBe('ws-1')
      for (const w of listWorkspaces(state).slice(0, 3)) {
        expect(w.pinned).toBe(true)
      }
      expect(listWorkspaces(state)[3].pinned).toBeUndefined()
    })

    it('unpins into the head of the unpinned block, right behind the pinned group', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const pinned = setWorkspacePinned(state, 'ws-2', true)
      const unpinned = setWorkspacePinned(pinned, 'ws-2', false)

      // Unpinning leaves the workspace leading the unpinned block (first
      // slot after the — now empty — pinned group): the list never jumps,
      // the workspace simply stops carrying the pin. And the unpinned
      // workspace carries no `pinned` key at all.
      expect(listWorkspaces(unpinned).map((w) => w.id)).toEqual([
        'ws-2',
        'ws-1',
      ])
      expect('pinned' in listWorkspaces(unpinned)[0]).toBe(false)
    })

    it('moves an unpinned workspace behind the whole pinned block on unpin', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')
      state = setWorkspacePinned(state, 'ws-3', true)
      state = setWorkspacePinned(state, 'ws-1', true)

      // Pinned block is [ws-3, ws-1], gamma pinned FIRST.
      const next = setWorkspacePinned(state, 'ws-3', false)

      expect(listWorkspaces(next).map((w) => w.id)).toEqual([
        'ws-1',
        'ws-3',
        'ws-2',
      ])
    })

    it('does not touch activeId or openIds (definitions-only)', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = setWorkspacePinned(state, 'ws-2', true)

      expect(next.activeId).toBe('ws-2')
      expect(next.openIds).toEqual(['ws-1', 'ws-2'])
    })

    it('is a no-op for an unknown id or an already-matching flag', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')

      expect(setWorkspacePinned(state, 'nope', true)).toBe(state)
      expect(setWorkspacePinned(state, 'ws-1', false)).toBe(state)

      const pinned = setWorkspacePinned(state, 'ws-1', true)
      expect(setWorkspacePinned(pinned, 'ws-1', true)).toBe(pinned)
    })
  })

  // Phase 11 / #12 — active panel focus (story 34). Each workspace tracks
  // which of its panels is focused; the renderer draws a ring on it. The
  // active panel is runtime-only (NOT persisted), like activeId.
  describe('active panel focus', () => {
    // A helper that builds a split workspace with two known panel ids, so the
    // focus tests have stable panel ids to point at. genId yields ws-1, p-1
    // for createWorkspace's workspace+seed-panel, then s-1, p-2 for the split.
    function splitState(): ReturnType<typeof splitPanel> {
      const gen = seq('ws-1', 'tab-1', 'p-1', 's-1', 'p-2')
      const state = createWorkspace(emptyState, 'my-project', gen)
      return splitPanel(state, 'ws-1', 'horizontal', gen)
    }

    it('defaults to the first panel when none has been focused', () => {
      // A workspace with a tree but no activePanelId entry — e.g. state built
      // before focus was ever set. activePanelOf must fall back to leaf #1
      // (of the first tab — no activeTabId entry either).
      const tree: LayoutNode = {
        kind: 'split',
        id: 's-1',
        orientation: 'horizontal',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'p-1' },
        second: { kind: 'leaf', id: 'p-2' },
      }
      const state = {
        ...emptyState,
        workspaces: [
          { id: 'ws-1', name: 'x', panels: [], tabs: [{ id: 't-1', layout: tree }] },
        ],
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
      const gen = seq('ws-1', 'tab-1', 'p-1', 's-1', 'p-2')
      const single = createWorkspace(emptyState, 'my-project', gen)

      const next = splitPanel(single, 'ws-1', 'horizontal', gen)

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

  // v0.2 Phase 1 / #25 — seeding app state from the persisted config (AC4:
  // the layout round-trips through restart). bootState keeps a workspace's
  // layout tree when the config has one and grows a fresh single leaf when it
  // does not (pre-v0.2 config).
  describe('bootState', () => {
    it('migrates a legacy top-level layout into the first tab and strips the key', () => {
      const tree: LayoutNode = {
        kind: 'split',
        id: 's-1',
        orientation: 'horizontal',
        ratio: 0.7,
        first: { kind: 'leaf', id: 'p-1' },
        second: { kind: 'leaf', id: 'p-2' },
      }

      const state = bootState([{ id: 'ws-1', name: 'my-project', panels: [], layout: tree }], seq('t-1'))

      // The tree round-trips INSIDE the seeded tab, and the legacy `layout`
      // key is gone — the next save writes the new shape only.
      expect(state.workspaces[0].tabs).toEqual([{ id: 't-1', layout: tree, name: 'Tab 1' }])
      expect('layout' in state.workspaces[0]).toBe(false)
      expect(panelIdsOf(state, 'ws-1')).toEqual(['p-1', 'p-2'])
    })

    it('fills missing tab names on an already-migrated tabs array', () => {
      const tree: LayoutNode = { kind: 'leaf', id: 'p-1' }

      const state = bootState(
        [{ id: 'ws-1', name: 'my-project', panels: [], tabs: [{ id: 't-1', layout: tree }] }],
        seq(),
      )

      // Names are filled with stable positional defaults so a later pin or
      // reorder never reshuffles the displayed numbers (HITL fix).
      expect(state.workspaces[0].tabs).toEqual([
        { id: 't-1', layout: tree, name: 'Tab 1' },
      ])
    })

    it('seeds a fresh single leaf for a workspace without a layout (old config)', () => {
      const state = bootState(
        [{ id: 'ws-1', name: 'my-project', panels: [] }],
        seq('fresh-1', 't-1'),
      )

      expect(state.workspaces[0].tabs).toEqual([
        { id: 't-1', layout: { kind: 'leaf', id: 'fresh-1' }, name: 'Tab 1' },
      ])
    })

    it('opens every workspace, activates the first, and focuses its first leaf', () => {
      const tree: LayoutNode = {
        kind: 'split',
        id: 's-1',
        orientation: 'vertical',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'a' },
        second: { kind: 'leaf', id: 'b' },
      }
      const other: LayoutNode = { kind: 'leaf', id: 'c' }

      const state = bootState(
        [
          { id: 'ws-1', name: 'one', panels: [], layout: tree },
          { id: 'ws-2', name: 'two', panels: [], layout: other },
        ],
        seq('t-1', 't-2'),
      )

      expect(state.openIds).toEqual(['ws-1', 'ws-2'])
      expect(state.activeId).toBe('ws-1')
      expect(state.activeTabId).toEqual({ 'ws-1': 't-1', 'ws-2': 't-2' })
      expect(activePanelOf(state, 'ws-1')).toBe('a')
      expect(activePanelOf(state, 'ws-2')).toBe('c')
    })

    it('handles an empty config with no active workspace', () => {
      const state = bootState([], seq())

      expect(state.activeId).toBeNull()
      expect(state.openIds).toEqual([])
    })
  })

  // #37 rework — terminal tabs inside a workspace. Assumptions encoded:
  //  - addTab appends a single-leaf tab and activates it (genId: tab id,
  //    then leaf id); a definitions change (persisted).
  //  - closeTab refuses to empty a workspace (min one tab), drops the dead
  //    tab's panels[] entries, and hands activation to the neighbor.
  //  - switchTab is runtime-only (activeTabId record, nothing persisted).
  describe('terminal tabs (#37 rework)', () => {
    function one(): ReturnType<typeof createWorkspace> {
      return createWorkspace(emptyState, 'my-project', seq('ws-1', 'tab-1', 'p-1'))
    }

    it('addTab appends a fresh single-leaf tab and activates it', () => {
      const next = addTab(one(), 'ws-1', seq('tab-2', 'p-2'))

      expect(next.workspaces[0].tabs).toHaveLength(2)
      expect(next.workspaces[0].tabs![1]).toEqual({
        id: 'tab-2',
        layout: { kind: 'leaf', id: 'p-2' },
        name: 'Tab 2',
      })
      expect(next.activeTabId['ws-1']).toBe('tab-2')
      expect(activePanelOf(next, 'ws-1')).toBe('p-2')
      // Panel identity spans tabs.
      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-1', 'p-2'])
    })

    it('closeTab removes the tab, its panels entries, and activates the neighbor', () => {
      let state = one()
      state = addTab(state, 'ws-1', seq('tab-2', 'p-2'))
      // Give the first tab's panel a cwd entry so orphan-dropping is visible.
      state = upsertPanelCwd(state, 'p-1', '/home/adam/proj')

      const next = closeTab(state, 'ws-1', 'tab-1')

      expect(next.workspaces[0].tabs).toHaveLength(1)
      expect(next.workspaces[0].tabs![0].id).toBe('tab-2')
      expect(next.workspaces[0].panels).toEqual([])
      expect(next.activeTabId['ws-1']).toBe('tab-2')
      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-2'])
    })

    it('closeTab refuses to close the last tab of a workspace', () => {
      const state = one()

      const next = closeTab(state, 'ws-1', 'tab-1')

      expect(next).toBe(state)
    })

    it('closeTab is a no-op for an unknown tab id', () => {
      const state = one()

      expect(closeTab(state, 'ws-1', 'nope')).toBe(state)
    })

    it('switchTab changes the active tab without touching definitions', () => {
      let state = one()
      state = addTab(state, 'ws-1', seq('tab-2', 'p-2'))

      const next = switchTab(state, 'ws-1', 'tab-1')

      expect(next.activeTabId['ws-1']).toBe('tab-1')
      expect(next.workspaces[0].tabs).toHaveLength(2)
      expect(next).not.toBe(state)
      // Unknown ids are a no-op.
      expect(switchTab(state, 'ws-1', 'nope')).toBe(state)
    })

    it('splitting always lands in the active tab', () => {
      let state = one()
      state = addTab(state, 'ws-1', seq('tab-2', 'p-2'))
      // Switch back to the FIRST tab, then split: the split must nest inside
      // tab-1's tree, not the active-tab-default tab-2.
      state = switchTab(state, 'ws-1', 'tab-1')

      const next = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      expect(next.workspaces[0].tabs![0].layout).toEqual({
        kind: 'split',
        id: 's-1',
        orientation: 'horizontal',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'p-1' },
        second: { kind: 'leaf', id: 'p-B' },
      })
      expect(next.workspaces[0].tabs![1].layout).toEqual({ kind: 'leaf', id: 'p-2' })
    })

    // Adam's follow-up: tab names (persisted) and tab pinning (pinned tabs
    // lead the bar as a group — the tab-level twin of setWorkspacePinned).
    it('renameTab sets the name; an empty commit drops the key', () => {
      let state = one()
      state = addTab(state, 'ws-1', seq('tab-2', 'p-2'))

      const named = renameTab(state, 'ws-1', 'tab-2', '  build  ')

      expect(named.workspaces[0].tabs![1].name).toBe('build')

      const cleared = renameTab(named, 'ws-1', 'tab-2', '   ')

      expect('name' in cleared.workspaces[0].tabs![1]).toBe(false)
      // Unknown ids leave the state object untouched.
      expect(renameTab(state, 'ws-1', 'nope', 'x')).toBe(state)
    })

    it('setTabPinned moves the tab to the top group and unpin drops the key', () => {
      let state = one()
      state = addTab(state, 'ws-1', seq('tab-2', 'p-2'))
      state = addTab(state, 'ws-1', seq('tab-3', 'p-3'))

      const pinned = setTabPinned(state, 'ws-1', 'tab-2', true)

      expect(pinned.workspaces[0].tabs!.map((t) => t.id)).toEqual([
        'tab-2',
        'tab-1',
        'tab-3',
      ])
      expect(pinned.workspaces[0].tabs![0].pinned).toBe(true)

      // Unpinning lands right behind the (now empty) pinned group and the
      // tab persists WITHOUT the key.
      const unpinned = setTabPinned(pinned, 'ws-1', 'tab-2', false)
      expect(unpinned.workspaces[0].tabs!.map((t) => t.id)).toEqual([
        'tab-2',
        'tab-1',
        'tab-3',
      ])
      expect('pinned' in unpinned.workspaces[0].tabs![0]).toBe(false)
      // No-ops: unknown tab, already-matching flag.
      expect(setTabPinned(state, 'ws-1', 'nope', true)).toBe(state)
      expect(setTabPinned(state, 'ws-1', 'tab-1', false)).toBe(state)
    })
  })
})

// --- v0.2 Phase 5 / #29: per-panel cwd metadata (snapshot + restore) ---------

describe('session snapshot metadata (#29)', () => {
  // One workspace, one tab, one leaf p-1 carrying a cwd + ssh target in
  // panels[] (#37 rework: the tree hangs off the tab).
  function withMeta() {
    return {
      ...emptyState,
      workspaces: [
        {
          id: 'ws-1',
          name: 'alpha',
          panels: [
            { id: 'p-1', workingDirectory: '/home/adam/proj', sshTarget: 'adam@host' },
          ],
          tabs: [{ id: 't-1', layout: { kind: 'leaf', id: 'p-1' } as LayoutNode }],
        },
      ],
      activeId: 'ws-1',
      openIds: ['ws-1'],
      activeTabId: { 'ws-1': 't-1' },
      activePanelId: { 'ws-1': 'p-1' },
      zoomedPanelId: {},
    }
  }

  // T-M1 (AC1/AC2 — a split inherits the source panel's config): splitting a
  //   panel that sits in ~/proj over SSH gives the new leaf the SAME cwd and
  //   ssh target, so the restored sibling respawns in the same place.
  it('splitPanel copies the target panel meta to the new leaf', () => {
    const next = splitPanel(withMeta(), 'ws-1', 'horizontal', seq('s-1', 'p-2'))

    const panels = next.workspaces[0].panels ?? []
    expect(panels).toEqual([
      { id: 'p-1', workingDirectory: '/home/adam/proj', sshTarget: 'adam@host' },
      { id: 'p-2', workingDirectory: '/home/adam/proj', sshTarget: 'adam@host' },
    ])
  })

  // T-M2 (payload compatibility — a workspace with NO panel meta must not
  //   gain a `panels` key just because a split happened; old saves stay
  //   byte-identical):
  it('splitPanel leaves a meta-less workspace untouched', () => {
    const bare = { ...withMeta(), workspaces: [{ ...withMeta().workspaces[0], panels: undefined }] }
    const next = splitPanel(bare, 'ws-1', 'horizontal', seq('s-1', 'p-2'))

    expect(next.workspaces[0].panels).toBeUndefined()
  })

  // T-M3 (closing a panel drops its meta entry — the config never accumulates
  //   orphans for leaves that no longer exist):
  it('closePanel removes the closed leaf meta entry', () => {
    const split = splitPanel(withMeta(), 'ws-1', 'horizontal', seq('s-1', 'p-2'))

    const next = closePanel(split, 'ws-1', 'p-2')

    expect(next.workspaces[0].panels).toEqual([
      { id: 'p-1', workingDirectory: '/home/adam/proj', sshTarget: 'adam@host' },
    ])
  })

  // T-M4 (upsert updates an existing entry in place — a fresh cwd overwrites
  //   the stale one for the SAME leaf):
  it('upsertPanelCwd updates an existing entry', () => {
    const next = upsertPanelCwd(withMeta(), 'p-1', '/home/adam/other')

    expect(next.workspaces[0].panels).toEqual([
      { id: 'p-1', workingDirectory: '/home/adam/other', sshTarget: 'adam@host' },
    ])
  })

  // T-M5 (upsert creates an entry for a leaf that has none yet — panels
  //   created fresh this session start without meta):
  it('upsertPanelCwd creates a missing entry without clobbering siblings', () => {
    const split = splitPanel(withMeta(), 'ws-1', 'horizontal', seq('s-1', 'p-2'))
    const stripped = {
      ...split,
      workspaces: [
        { ...split.workspaces[0], panels: [{ id: 'p-1', workingDirectory: '/x' }] },
      ],
    }

    const next = upsertPanelCwd(stripped, 'p-2', '/home/adam/proj')

    expect(next.workspaces[0].panels).toEqual([
      { id: 'p-1', workingDirectory: '/x' },
      { id: 'p-2', workingDirectory: '/home/adam/proj' },
    ])
  })

  // T-M6 (a cwd read that raced a teardown is a no-op, never a crash —
  //   unknown panel ids and empty strings are dropped silently):
  it('upsertPanelCwd ignores unknown panels and empty cwds', () => {
    const state = withMeta()

    expect(upsertPanelCwd(state, 'nope', '/somewhere')).toBe(state)
    expect(upsertPanelCwd(state, 'p-1', '')).toBe(state)
  })
})

// T-M7 (the periodic snapshot must not rewrite state/disk when nothing
//   moved): upserting the cwd a panel ALREADY has returns the SAME state
//   object (reference equality) — the caller uses that to skip the write.
it('upsertPanelCwd with an unchanged cwd returns the same reference', () => {
  const state = {
    ...emptyState,
    workspaces: [
      {
        id: 'ws-1',
        name: 'alpha',
        panels: [{ id: 'p-1', workingDirectory: '/x' }],
        layout: { kind: 'leaf', id: 'p-1' } as LayoutNode,
      },
    ],
    activeId: 'ws-1',
    openIds: ['ws-1'],
    activePanelId: { 'ws-1': 'p-1' },
    zoomedPanelId: {},
  }

  expect(upsertPanelCwd(state, 'p-1', '/x')).toBe(state)
  expect(upsertPanelCwd(state, 'p-1', '/y')).not.toBe(state)
})
