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
  moveTab,
  switchTab,
  renameTab,
  setTabPinned,
  activeTabOf,
  moveNode,
  moveNodes,
  moveToNewGroup,
  closeWorkspaces,
  deleteNodes,
  setNodesPinned,
  batchDeleteWorkspaceCount,
  createGroup,
  renameGroup,
  deleteGroup,
  isGroupEmpty,
  flattenSidebar,
  setWorkspacePinned,
  setGroupPinned,
  toggleCollapse,
  unpackGroup,
  deleteGroupSubtree,
  groupSubtreeIds,
  activeAgentCount,
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
  type WorkspaceState,
  setWorkspaceColor,
  COLOR_PALETTE,
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

/// The DISPLAY order of the sidebar's node ids (#48): groups and workspaces
/// interleaved, straight from the tree. The pre-tree tests asserted the
/// workspaces ARRAY as the display order — since #48 the shared `order` is
/// the source of truth, so display assertions go through here.
function displayIds(state: WorkspaceState): string[] {
  return flattenSidebar(state).map((e) =>
    e.kind === 'workspace' ? e.workspace.id : e.group.id,
  )
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

  // --- Sidebar tree (#48): groups exist, rename, delete (empty only) --------
  //
  // Assumptions encoded:
  //  - WorkspaceState carries `groups: Group[]` and ONE shared `order` of
  //    every node id; sibling order = relative position in `order` among
  //    entries sharing a parent.
  //  - createGroup appends at the END of the top level; the fresh group
  //    carries NO collapsed/pinned keys (they are Phase 4/6 slots).
  //  - Only an EMPTY group deletes (Phase 5 brings the destructive delete);
  //    a deleted group leaves both `groups` and `order`.
  //  - bootState is the migration surface: a pre-groups (flat) config loads
  //    with zero groups and every workspace at top level, a stale groupId
  //    strips back to top level, and `order` is normalized (unknown ids
  //    drop, missing ids append) so a hand-edited file loses nothing.
  describe('workspace groups (#48)', () => {
    function groupedState() {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createGroup(state, 'projekty', () => 'g-1')
      return state
    }

    it('createGroup appends an empty group node at top level', () => {
      const state = groupedState()

      expect(state.groups.map((g) => g.id)).toEqual(['g-1'])
      expect(state.groups[0].name).toBe('projekty')
      expect(displayIds(state)).toEqual(['ws-1', 'ws-2', 'g-1'])
      // Definitions untouched: creating a group never touches workspaces,
      // activation, or the open set.
      expect(state.workspaces.map((w) => w.id)).toEqual(['ws-1', 'ws-2'])
      expect(state.activeId).toBe('ws-2')
      expect(state.openIds).toEqual(['ws-1', 'ws-2'])
    })

    it('a fresh group carries no collapsed/pinned keys', () => {
      const state = groupedState()

      expect('collapsed' in state.groups[0]).toBe(false)
      expect('pinned' in state.groups[0]).toBe(false)
    })

    it('renameGroup renames; blank names and unknown ids are no-ops', () => {
      const state = groupedState()

      const renamed = renameGroup(state, 'g-1', 'klienty')
      expect(renamed.groups[0].name).toBe('klienty')

      expect(renameGroup(state, 'g-1', '   ')).toBe(state)
      expect(renameGroup(state, 'g-x', 'nope')).toBe(state)
    })

    it('deleteGroup removes an EMPTY group from groups and order', () => {
      const state = groupedState()
      expect(isGroupEmpty(state, 'g-1')).toBe(true)

      const next = deleteGroup(state, 'g-1')

      expect(next.groups).toEqual([])
      expect(next.order).toEqual(['ws-1', 'ws-2'])
    })

    it('deleteGroup refuses a NON-EMPTY group (destructive delete is Phase 5)', () => {
      let state = groupedState()
      state = moveNode(state, 'ws-1', { parentId: 'g-1' })
      expect(isGroupEmpty(state, 'g-1')).toBe(false)

      const next = deleteGroup(state, 'g-1')

      expect(next).toBe(state)
      expect(next.groups.map((g) => g.id)).toEqual(['g-1'])
    })

    it('bootState: a pre-groups FLAT config loads flat (zero groups, all top level)', () => {
      const loaded = [
        { id: 'ws-1', name: 'alpha' },
        { id: 'ws-2', name: 'beta' },
      ]

      const state = bootState(loaded)

      expect(state.groups).toEqual([])
      expect(state.order).toEqual(['ws-1', 'ws-2'])
      for (const w of state.workspaces) expect(w.groupId).toBeUndefined()
    })

    it('bootState: a stale groupId (unknown group) strips back to top level', () => {
      const loaded = [{ id: 'ws-1', name: 'alpha', groupId: 'g-gone' }]

      const state = bootState(loaded)

      expect('groupId' in state.workspaces[0]).toBe(false)
      expect(state.groups).toEqual([])
      expect(state.order).toEqual(['ws-1'])
    })

    it('bootState: order normalization drops unknown ids, dedupes, appends missing', () => {
      const loaded = [
        { id: 'ws-1', name: 'alpha' },
        { id: 'ws-2', name: 'beta' },
      ]
      const groups = [{ id: 'g-1', name: 'projekty' }]

      const state = bootState(loaded, seq(), groups, [
        'g-1',
        'ghost',
        'ws-2',
        'ws-2',
      ])

      // 'ghost' drops (unknown), the duplicate ws-2 drops, and ws-1 — which
      // the config forgot to rank — appends last.
      expect(state.order).toEqual(['g-1', 'ws-2', 'ws-1'])
    })

    it('flattenSidebar interleaves groups and workspaces with depth', () => {
      let state = groupedState()
      state = moveNode(state, 'ws-1', { parentId: 'g-1' })

      const entries = flattenSidebar(state)

      expect(entries.map((e) => (e.kind === 'group' ? `g:${e.group.name}` : `w:${e.workspace.id}`))).toEqual([
        'w:ws-2',
        'g:projekty',
        'w:ws-1',
      ])
      expect(entries.map((e) => e.depth)).toEqual([0, 0, 1])
    })
  })

  // --- moveNode (#49): filing workspaces into groups -------------------------
  //
  // Assumptions encoded:
  //  - move into a group APPENDS at the end of that group's children (the
  //    group's block, not the end of the whole list);
  //  - move to top-level space restores top level AND drops the `groupId`
  //    key (the persisted payload matches one that was never grouped);
  //  - reorder via `beforeId` keeps the relative order of the other
  //    siblings; a `beforeId` that is not a sibling of the target parent
  //    falls back to append;
  //  - a GROUP may nest into any group EXCEPT its own subtree (#51) — a cycle
//    landing is a no-op;
  //  - unknown nodes and unknown target groups are no-ops (same object back).
  describe('moveNode (#49)', () => {
    function twoInGroup() {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createGroup(state, 'projekty', () => 'g-1')
      state = moveNode(state, 'ws-1', { parentId: 'g-1' })
      return state // order: [ws-2, g-1, ws-1]
    }

    it('move into a group appends at the END of the target', () => {
      const state = twoInGroup()
      const next = moveNode(state, 'ws-2', { parentId: 'g-1' })

      // The newcomer lands after the group's existing child, not at the end
      // of the whole list.
      expect(next.order).toEqual(['g-1', 'ws-1', 'ws-2'])
      expect(displayIds(next)).toEqual(['g-1', 'ws-1', 'ws-2'])
      expect(next.workspaces.find((w) => w.id === 'ws-2')?.groupId).toBe('g-1')
    })

    it('move to top-level space restores top level and drops the groupId key', () => {
      const state = twoInGroup()

      const next = moveNode(state, 'ws-1', { parentId: null })

      expect(next.order).toEqual(['ws-2', 'g-1', 'ws-1'])
      const moved = next.workspaces.find((w) => w.id === 'ws-1')!
      expect('groupId' in moved).toBe(false)
    })

    it('reorder inside a group preserves the other children’s relative order', () => {
      let state = twoInGroup()
      state = moveNode(state, 'ws-2', { parentId: 'g-1' })
      // order: [g-1, ws-1, ws-2] — swap the children.
      const next = moveNode(state, 'ws-2', { parentId: 'g-1', beforeId: 'ws-1' })

      expect(next.order).toEqual(['g-1', 'ws-2', 'ws-1'])
    })

    it('a group nests into an unrelated group (#51)', () => {
      let state = twoInGroup()
      state = createGroup(state, 'inne', () => 'g-2')

      const next = moveNode(state, 'g-1', { parentId: 'g-2' })

      expect(next).not.toBe(state)
      expect(next.groups.find((g) => g.id === 'g-1')?.parentId).toBe('g-2')
      // The moved group (with its subtree) renders below g-2, one level deep.
      expect(displayIds(next)).toEqual(['ws-2', 'g-2', 'g-1', 'ws-1'])
      expect(flattenSidebar(next).map((e) => e.depth)).toEqual([0, 0, 1, 2])
    })

    it('moving a group into its OWN SUBTREE is rejected (#51)', () => {
      // order: [ws-2, g-2, g-1, ws-1] with g-1 inside g-2; dropping g-2 into
      // its own child (g-1) — or into itself — would orphan a cycle.
      let state = twoInGroup()
      state = createGroup(state, 'inne', () => 'g-2')
      state = moveNode(state, 'g-1', { parentId: 'g-2' })

      expect(moveNode(state, 'g-2', { parentId: 'g-1' })).toBe(state)
      expect(moveNode(state, 'g-2', { parentId: 'g-2' })).toBe(state)
      // A DESCENDANT further down is rejected too: g-3 goes INSIDE g-1
      // (three levels), and g-2 may not move into its grandchild.
      state = createGroup(state, 'gleboko', () => 'g-3')
      state = moveNode(state, 'g-3', { parentId: 'g-1' })
      expect(moveNode(state, 'g-2', { parentId: 'g-3' })).toBe(state)
    })

    it('unknown node, unknown target group, self-move and non-sibling beforeId are safe', () => {
      const state = twoInGroup()

      expect(moveNode(state, 'nope', { parentId: 'g-1' })).toBe(state)
      expect(moveNode(state, 'ws-2', { parentId: 'g-x' })).toBe(state)
      expect(
        moveNode(state, 'ws-1', { parentId: 'g-1', beforeId: 'ws-1' }),
      ).toBe(state)
      // beforeId='ws-1' sits inside g-1, so it is NOT a top-level sibling —
      // moving ws-2 to top level falls back to append: after the last
      // top-level node (the group g-1), at the head of the top level.
      const appended = moveNode(state, 'ws-2', { parentId: null, beforeId: 'ws-1' })
      expect(appended.order).toEqual(['g-1', 'ws-2', 'ws-1'])
      expect(appended.workspaces.find((w) => w.id === 'ws-2')?.groupId).toBeUndefined()
    })

    it('a move never touches openIds or activeId', () => {
      const state = twoInGroup()

      const next = moveNode(state, 'ws-2', { parentId: 'g-1' })

      expect(next.openIds).toEqual(['ws-1', 'ws-2'])
      expect(next.activeId).toBe('ws-2')
    })
  })

  describe('moveToNewGroup (#49)', () => {
    it('creates the group on the fly and files the workspace into it', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')

      const next = moveToNewGroup(state, 'ws-2', 'klienty', () => 'g-1')

      expect(next.groups.map((g) => g.id)).toEqual(['g-1'])
      expect(next.groups[0].name).toBe('klienty')
      expect(next.workspaces.find((w) => w.id === 'ws-2')?.groupId).toBe('g-1')
      // The fresh group leads no one: it appends at the end of the top
      // level, with the filed workspace as its only child.
      expect(next.order).toEqual(['ws-1', 'g-1', 'ws-2'])
    })

    it('blank name or unknown workspace is a no-op', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')

      expect(moveToNewGroup(state, 'ws-1', '  ')).toBe(state)
      expect(moveToNewGroup(state, 'nope', 'x')).toBe(state)
    })
  })

  describe('createWorkspace stays top-level (#49)', () => {
    it('a new workspace never lands inside a group, even with groups present', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createGroup(state, 'projekty', () => 'g-1')
      state = moveNode(state, 'ws-1', { parentId: 'g-1' })

      const next = createWorkspace(state, 'gamma', () => 'ws-2')

      expect(next.workspaces.find((w) => w.id === 'ws-2')?.groupId).toBeUndefined()
      // Ranked last among the TOP-LEVEL siblings — after the group, not
      // inside it.
      expect(next.order).toEqual(['g-1', 'ws-1', 'ws-2'])
      expect(displayIds(next)).toEqual(['g-1', 'ws-1', 'ws-2'])
      expect(next.activeId).toBe('ws-2')
    })
  })

  // #45 — drag & drop reorder of tabs inside ONE workspace's tab bar.
  // Assumptions encoded:
  //  - Only ORDER changes: everything hangs off the tab's id (name, layout
  //    tree, pinned flag), and the runtime activeTabId record (id-keyed)
  //    follows the moved tab untouched.
  //  - Same splice semantics as moveWorkspace: target index is clamped, and
  //    a drop ON a tab lands the dragged tab at that tab's position.
  //  - No-op for unknown workspace or tab ids (same object back).
  describe('moveTab', () => {
    function twoTabState() {
      let state = createWorkspace(emptyState, 'alpha', seq('ws-1', 'leaf-1'))
      state = addTab(state, 'ws-1', seq('tab-1', 'leaf-2'))
      state = addTab(state, 'ws-1', seq('tab-2', 'leaf-3'))
      return state
    }

    it('moves a tab forward within its workspace', () => {
      const state = twoTabState()
      const ws = () => state.workspaces.find((w) => w.id === 'ws-1')!
      const first = ws().tabs![0].id

      const next = moveTab(state, 'ws-1', first, 2)

      expect(next.workspaces.find((w) => w.id === 'ws-1')!.tabs!.map((t) => t.id)).toEqual([
        ws().tabs![1].id,
        ws().tabs![2].id,
        first,
      ])
    })

    it('moves a tab backward within its workspace', () => {
      const state = twoTabState()
      const tabs = () => state.workspaces.find((w) => w.id === 'ws-1')!.tabs!
      const last = tabs()[2].id

      const next = moveTab(state, 'ws-1', last, 0)

      expect(next.workspaces.find((w) => w.id === 'ws-1')!.tabs!.map((t) => t.id)).toEqual([
        last,
        tabs()[0].id,
        tabs()[1].id,
      ])
    })

    it('keeps the moved tab\'s identity: name, layout, and active-tab record travel with it', () => {
      const state = twoTabState()
      const tabs = () => state.workspaces.find((w) => w.id === 'ws-1')!.tabs!
      const firstId = tabs()[0].id
      const firstName = tabs()[0].name
      const firstLayout = tabs()[0].layout

      const next = moveTab(state, 'ws-1', firstId, 2)

      const moved = next.workspaces.find((w) => w.id === 'ws-1')!.tabs![2]
      expect(moved.id).toBe(firstId)
      expect(moved.name).toBe(firstName)
      expect(moved.layout).toBe(firstLayout)
      // Runtime record is id-keyed — the move leaves activation untouched
      // (whatever tab was active before the drag is active after it).
      expect(next.activeTabId['ws-1']).toBe(state.activeTabId['ws-1'])
    })

    it('clamps an out-of-range index', () => {
      const state = twoTabState()
      const first = state.workspaces.find((w) => w.id === 'ws-1')!.tabs![0].id

      const next = moveTab(state, 'ws-1', first, 99)

      const ids = next.workspaces.find((w) => w.id === 'ws-1')!.tabs!.map((t) => t.id)
      expect(ids[ids.length - 1]).toBe(first)
    })

    it('is a no-op for an unknown workspace or tab id', () => {
      const state = twoTabState()
      const someTab = state.workspaces.find((w) => w.id === 'ws-1')!.tabs![0].id

      expect(moveTab(state, 'nope', someTab, 0)).toBe(state)
      expect(moveTab(state, 'ws-1', 'nope', 0)).toBe(state)
    })
  })

  // #37 — pinned workspaces. Since #48 the SHARED tree order is the display
  // order (the sidebar reads it via flattenSidebar), so the toggle reorders
  // `order` among the target's siblings instead of the definitions array:
  //   - pinning appends to the END of the pinned block
  //   - unpinning inserts at the HEAD of the unpinned block
  // Assumptions encoded:
  //  - `pinned` is optional on Workspace; absent = unpinned, and an unpinned
  //    workspace carries NO key at all (old saves must not gain one).
  //  - Toggling never touches activeId/openIds (definitions-only).
  describe('setWorkspacePinned', () => {
    it('moves a pinned workspace to the top of the list', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')

      const next = setWorkspacePinned(state, 'ws-3', true)

      expect(displayIds(next)).toEqual(['ws-3', 'ws-1', 'ws-2'])
      expect(listWorkspaces(next).find((w) => w.id === 'ws-3')?.pinned).toBe(true)
    })

    it('appends to the end of the pinned block when others are pinned', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')
      state = setWorkspacePinned(state, 'ws-1', true)

      const next = setWorkspacePinned(state, 'ws-3', true)

      // ws-1 pins first, ws-3 joins AFTER it — pinned group keeps a stable,
      // predictable order (the order in which workspaces were pinned).
      expect(displayIds(next)).toEqual(['ws-1', 'ws-3', 'ws-2'])
    })

    it('keeps the pinned group first as an invariant across several toggles', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')
      state = createWorkspace(state, 'delta', () => 'ws-4')

      state = setWorkspacePinned(state, 'ws-4', true)
      state = setWorkspacePinned(state, 'ws-2', true)
      state = setWorkspacePinned(state, 'ws-3', true)

      const ids = displayIds(state)
      expect(ids.slice(0, 3)).toEqual(['ws-4', 'ws-2', 'ws-3'])
      expect(ids[3]).toBe('ws-1')
      for (const id of ['ws-4', 'ws-2', 'ws-3']) {
        expect(listWorkspaces(state).find((w) => w.id === id)?.pinned).toBe(true)
      }
      expect(
        listWorkspaces(state).find((w) => w.id === 'ws-1')?.pinned,
      ).toBeUndefined()
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
      expect(displayIds(unpinned)).toEqual(['ws-2', 'ws-1'])
      expect(
        'pinned' in listWorkspaces(unpinned).find((w) => w.id === 'ws-2')!,
      ).toBe(false)
    })

    it('moves an unpinned workspace behind the whole pinned block on unpin', () => {
      let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
      state = createWorkspace(state, 'beta', () => 'ws-2')
      state = createWorkspace(state, 'gamma', () => 'ws-3')
      state = setWorkspacePinned(state, 'ws-3', true)
      state = setWorkspacePinned(state, 'ws-1', true)

      // Pinned block is [ws-3, ws-1], gamma pinned FIRST.
      const next = setWorkspacePinned(state, 'ws-3', false)

      expect(displayIds(next)).toEqual(['ws-1', 'ws-3', 'ws-2'])
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

// --- Workspace groups Phase 4-6 (#50/#51/#52) -------------------------------
//
// Assumptions encoded:
//  - toggleCollapse (#50) flips the group's `collapsed` flag IN THE TREE
//    (persisted, not transient UI state); expanding DROPS the key so the
//    payload stays byte-identical to a never-collapsed group.
//  - The badge (activeAgentCount, #50) counts ACTIVE AGENT STATUSES: every
//    panel in the subtree whose status is `working` OR `needs-attention`
//    (the agent finished and waits for you — an occupied panel, HITL fix
//    2026-08-28; a collapsed group must not hide a waiting agent). The
//    number GROWS with each active agent (Adam's call): two working panels
//    in one workspace count 2. It clears when the agents exit. Since #51
//    the sum covers the WHOLE subtree.
//  - Nesting (#51): groups nest without depth limit; moveNode rejects any
//    landing inside the moved group's own subtree (cycle); unpackGroup
//    dissolves the group with every child returning to top level; 
//    deleteGroupSubtree removes everything inside, activation falls back per
//    the workspace-delete rules.
//  - Pin per container (#52): a pinned node leads only among its OWN
//    siblings; the flag survives moves (pin is local to the new container).
//  - bootState normalizes nested groups: stale parentIds strip to top level,
//    parent CYCLES break (every group on/behind a cycle loads at top level).
describe('workspace groups: collapse + badge (#50)', () => {
  const seedGrouped = () => {
    let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
    state = createWorkspace(state, 'beta', () => 'ws-2')
    state = createGroup(state, 'projekty', () => 'g-1')
    state = moveNode(state, 'ws-1', { parentId: 'g-1' })
    return state // order: [ws-2, g-1, ws-1]
  }

  it('toggleCollapse flips the flag in the tree, per group', () => {
    let state = seedGrouped()
    state = createGroup(state, 'inne', () => 'g-2')

    const collapsed = toggleCollapse(state, 'g-1')

    expect(collapsed.groups.find((g) => g.id === 'g-1')?.collapsed).toBe(true)
    // The OTHER group is untouched (per-group state).
    expect(collapsed.groups.find((g) => g.id === 'g-2')?.collapsed).toBeUndefined()

    const expanded = toggleCollapse(collapsed, 'g-1')
    expect(expanded.groups.find((g) => g.id === 'g-1')?.collapsed).toBeUndefined()
    expect('collapsed' in (expanded.groups.find((g) => g.id === 'g-1') as object)).toBe(false)
  })

  it('toggleCollapse is a no-op for an unknown id', () => {
    const state = seedGrouped()
    expect(toggleCollapse(state, 'g-x')).toBe(state)
  })

  it('a collapsed group hides its children in flattenSidebar', () => {
    let state = seedGrouped()
    state = toggleCollapse(state, 'g-1')

    expect(displayIds(state)).toEqual(['ws-2', 'g-1'])
    // Expanding reveals the child again, in the shared order.
    expect(displayIds(toggleCollapse(state, 'g-1'))).toEqual(['ws-2', 'g-1', 'ws-1'])
  })

  it('collapsed flags survive a bootState round-trip', () => {
    let state = seedGrouped()
    state = toggleCollapse(state, 'g-1')

    const booted = bootState(
      state.workspaces,
      () => 'fresh',
      state.groups,
      state.order,
    )

    expect(booted.groups.find((g) => g.id === 'g-1')?.collapsed).toBe(true)
  })

  it('activeAgentCount counts WORKING and NEEDS-ATTENTION panels; a plain idle shell does not count', () => {
    const state = seedGrouped()
    const pids = panelIdsOf(state, 'ws-1')
    expect(pids.length).toBeGreaterThan(0)
    const statuses = { [pids[0]]: 'working' } as Record<string, ReturnType<typeof Object.keys> extends never ? never : import('./agentStatus').AgentStatus>

    expect(activeAgentCount(state, 'g-1', statuses)).toBe(1)

    // The agent FINISHED and waits for you -> still an occupied panel, the
    // badge STAYS (HITL fix 2026-08-28: an idle Claude Code sits in NA).
    expect(activeAgentCount(state, 'g-1', { [pids[0]]: 'needs-attention' })).toBe(1)

    // Agent exited -> plain idle shell -> badge clears.
    expect(activeAgentCount(state, 'g-1', { [pids[0]]: 'idle' })).toBe(0)
    expect(activeAgentCount(state, 'g-1', {})).toBe(0)
  })

  it('activeAgentCount grows with EVERY active panel — two working panels count 2 (Adam)', () => {
    let state = seedGrouped()
    state = splitPanel(state, 'ws-1', 'horizontal', () => 's-1')
    const pids = panelIdsOf(state, 'ws-1')
    expect(pids.length).toBe(2)
    const both = Object.fromEntries(pids.map((p) => [p, 'working' as const]))

    expect(activeAgentCount(state, 'g-1', both)).toBe(2)

    // Only ONE of the two panels active -> 1. Plain idle shells never count.
    expect(activeAgentCount(state, 'g-1', { [pids[0]]: 'working' })).toBe(1)
    expect(
      activeAgentCount(state, 'g-1', { [pids[1]]: 'needs-attention' }),
    ).toBe(1)
    expect(
      activeAgentCount(state, 'g-1', { [pids[0]]: 'idle', [pids[1]]: 'idle' }),
    ).toBe(0)
  })

  it('activeAgentCount is 0 for a group without workspaces or without agents', () => {
    let state = seedGrouped()
    state = createGroup(state, 'puste', () => 'g-2')

    expect(activeAgentCount(state, 'g-2', {})).toBe(0)
    expect(activeAgentCount(state, 'g-1', {})).toBe(0)
  })
})

describe('workspace groups: nesting (#51)', () => {
  /// g-outer -> g-inner -> ws-1, with ws-2 at top level.
  const seedNested = () => {
    let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
    state = createWorkspace(state, 'beta', () => 'ws-2')
    state = createGroup(state, 'inner', () => 'g-inner')
    state = createGroup(state, 'outer', () => 'g-outer')
    state = moveNode(state, 'g-inner', { parentId: 'g-outer' })
    state = moveNode(state, 'ws-1', { parentId: 'g-inner' })
    return state
    // order walks: ws-2, g-outer, g-inner, ws-1
  }

  it('nests N levels deep; flattenSidebar renders increasing indentation', () => {
    let state = seedNested()
    state = createGroup(state, 'deepest', () => 'g-3')
    state = moveNode(state, 'g-3', { parentId: 'g-inner' })

    const entries = flattenSidebar(state)
    const byId = entries.map((e) =>
      e.kind === 'group' ? `g:${e.group.id}` : `w:${e.workspace.id}`,
    )
    // g-3 appended at the END of g-inner's children (after ws-1).
    expect(byId).toEqual(['w:ws-2', 'g:g-outer', 'g:g-inner', 'w:ws-1', 'g:g-3'])
    expect(entries.map((e) => e.depth)).toEqual([0, 0, 1, 2, 2])
  })

  it('a group moving to top level drops the parentId key', () => {
    const state = seedNested()

    const next = moveNode(state, 'g-inner', { parentId: null })

    const moved = next.groups.find((g) => g.id === 'g-inner')!
    expect('parentId' in moved).toBe(false)
  })

  it('a nested group keeps its subtree: the workspace follows its parent', () => {
    let state = seedNested()
    state = moveNode(state, 'g-inner', { parentId: null })

    // ws-1 still belongs to g-inner; the flatten still renders it under it.
    expect(state.workspaces.find((w) => w.id === 'ws-1')?.groupId).toBe('g-inner')
    expect(displayIds(state)).toEqual(['ws-2', 'g-outer', 'g-inner', 'ws-1'])
  })

  it('a deep nested tree round-trips through bootState', () => {
    const state = seedNested()

    const booted = bootState(
      state.workspaces,
      () => 'fresh',
      state.groups,
      state.order,
    )

    expect(booted.groups.find((g) => g.id === 'g-inner')?.parentId).toBe('g-outer')
    expect(displayIds(booted)).toEqual(displayIds(state))
    expect(flattenSidebar(booted).map((e) => e.depth)).toEqual([0, 0, 1, 2])
  })

  it('bootState strips a STALE parentId (unknown group) to top level', () => {
    const groups = [
      { id: 'g-1', name: 'osierocony', parentId: 'g-ghost' },
      { id: 'g-2', name: 'ok' },
    ]

    const booted = bootState([], () => 'fresh', groups, ['g-1', 'g-2'])

    const g1 = booted.groups.find((g) => g.id === 'g-1')!
    expect('parentId' in g1).toBe(false)
    expect(booted.groups.find((g) => g.id === 'g-2')?.parentId).toBeUndefined()
  })

  it('bootState BREAKS a parent cycle: every group on/behind it loads at top level', () => {
    // Hand-edited config: g-1 in g-2, g-2 in g-1, g-3 hangs BEHIND the cycle.
    const groups = [
      { id: 'g-1', name: 'a', parentId: 'g-2' },
      { id: 'g-2', name: 'b', parentId: 'g-1' },
      { id: 'g-3', name: 'c', parentId: 'g-1' },
    ]

    const booted = bootState([], () => 'fresh', groups, ['g-1', 'g-2', 'g-3'])

    // Nothing crashes, nothing dangles: every parent chain terminates.
    for (const g of booted.groups) {
      const seen = new Set<string>()
      let cur: string | null | undefined = g.parentId
      while (cur != null) {
        expect(seen.has(cur)).toBe(false)
        seen.add(cur)
        cur = booted.groups.find((x) => x.id === cur)?.parentId
      }
    }
    expect(booted.groups).toHaveLength(3)
  })
})

describe('workspace groups: unpack + destructive delete (#51)', () => {
  const seedForUnpack = () => {
    let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
    state = createWorkspace(state, 'beta', () => 'ws-2')
    state = createWorkspace(state, 'gamma', () => 'ws-3')
    state = createGroup(state, 'sub', () => 'g-sub')
    state = createGroup(state, 'main', () => 'g-main')
    state = moveNode(state, 'g-sub', { parentId: 'g-main' })
    state = moveNode(state, 'ws-1', { parentId: 'g-main' })
    state = moveNode(state, 'ws-2', { parentId: 'g-sub' })
    // order: ws-3, g-main, g-sub, ws-2, ws-1
    return state
  }

  it('unpackGroup dissolves the group: every child returns to TOP level, nothing closes', () => {
    const state = seedForUnpack()

    const next = unpackGroup(state, 'g-main')

    // The group is gone; its children took its top-level slot in order.
    expect(next.groups.some((g) => g.id === 'g-main')).toBe(false)
    expect(displayIds(next)).toEqual(['ws-3', 'g-sub', 'ws-2', 'ws-1'])
    // Every child is now top level: no parentId/groupId keys anywhere.
    expect(next.groups.find((g) => g.id === 'g-sub')?.parentId).toBeUndefined()
    expect(next.workspaces.find((w) => w.id === 'ws-1')?.groupId).toBeUndefined()
    // ws-2 was a child of g-SUB (not of the unpacked g-main): it stays
    // inside g-sub, which itself came to top level.
    expect(next.workspaces.find((w) => w.id === 'ws-2')?.groupId).toBe('g-sub')
    // NOTHING closed: the open set and activation ride along.
    expect(next.openIds).toEqual(['ws-1', 'ws-2', 'ws-3'])
    expect(next.activeId).toBe('ws-3')
  })

  it('unpackGroup keeps DEEPER descendants under their own parents', () => {
    let state = seedForUnpack()
    state = createWorkspace(state, 'delta', () => 'ws-4')
    state = moveNode(state, 'ws-4', { parentId: 'g-sub' })

    const next = unpackGroup(state, 'g-main')

    // g-sub came to top level; ws-4 stayed INSIDE g-sub.
    expect(next.workspaces.find((w) => w.id === 'ws-4')?.groupId).toBe('g-sub')
  })

  it('unpackGroup is a no-op for an unknown id', () => {
    const state = seedForUnpack()
    expect(unpackGroup(state, 'g-x')).toBe(state)
  })

  it('deleteGroupSubtree removes the group with EVERYTHING inside it', () => {
    let state = seedForUnpack()
    state = createWorkspace(state, 'delta', () => 'ws-4')
    state = moveNode(state, 'ws-4', { parentId: 'g-sub' })
    const booted = bootState(state.workspaces, () => 'fresh', state.groups, state.order)

    const next = deleteGroupSubtree(booted, 'g-main')

    expect(next.groups.some((g) => g.id === 'g-main' || g.id === 'g-sub')).toBe(false)
    expect(next.workspaces.map((w) => w.id)).toEqual(['ws-3'])
    expect(next.order).toEqual(['ws-3'])
    // The deleted workspaces' runtime records die with them.
    expect(next.openIds).toEqual(['ws-3'])
    expect(next.activeTabId['ws-1']).toBeUndefined()
    expect(next.activePanelId['ws-2']).toBeUndefined()
  })

  it('deleteGroupSubtree hands activation to a survivor when the active one was inside', () => {
    const state = seedForUnpack() // active: ws-3 (top level survivor)

    const next = deleteGroupSubtree(state, 'g-main')
    expect(next.activeId).toBe('ws-3')

    // Now make a workspace INSIDE the group active and delete the group.
    let state2 = seedForUnpack()
    state2 = { ...state2, activeId: 'ws-2' } // ws-2 lives in g-sub
    const next2 = deleteGroupSubtree(state2, 'g-main')
    // ws-1 is ALSO inside the doomed subtree (it lives in g-main) — #53's
    // excluded-set fix stops activation from landing on a workspace dying in
    // the same operation, so it falls to the one true survivor, ws-3.
    expect(next2.activeId).toBe('ws-3')
  })

  it('deleteGroupSubtree drops the zoom records of the deleted tabs', () => {
    let state = seedForUnpack()
    const tabId = state.workspaces.find((w) => w.id === 'ws-2')?.tabs?.[0]?.id
    state = { ...state, zoomedPanelId: { ...(tabId ? { [tabId]: 'p-x' } : {}) } }

    const next = deleteGroupSubtree(state, 'g-main')

    if (tabId != null) expect(next.zoomedPanelId[tabId]).toBeUndefined()
  })

  it('deleteGroupSubtree is a no-op for an unknown id', () => {
    const state = seedForUnpack()
    expect(deleteGroupSubtree(state, 'g-x')).toBe(state)
  })

  it('groupSubtreeIds covers the whole depth, the group itself included', () => {
    const state = seedForUnpack()

    const ids = groupSubtreeIds(state, 'g-main')

    expect(new Set(ids)).toEqual(new Set(['g-main', 'g-sub', 'ws-1', 'ws-2']))
  })
})

describe('workspace groups: badge subtree sum (#51)', () => {
  it('activeAgentCount sums the WHOLE subtree, not just direct children', () => {
    let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
    state = createWorkspace(state, 'beta', () => 'ws-2')
    state = createGroup(state, 'inner', () => 'g-inner')
    state = createGroup(state, 'outer', () => 'g-outer')
    state = moveNode(state, 'g-inner', { parentId: 'g-outer' })
    state = moveNode(state, 'ws-1', { parentId: 'g-inner' })

    const pid = panelIdsOf(state, 'ws-1')[0]
    const statuses = { [pid]: 'working' } as Record<string, import('./agentStatus').AgentStatus>

    // ws-1 sits TWO levels under g-outer — still counted.
    expect(activeAgentCount(state, 'g-outer', statuses)).toBe(1)
    expect(activeAgentCount(state, 'g-inner', statuses)).toBe(1)
    expect(activeAgentCount(state, 'g-outer', {})).toBe(0)
  })
})

describe('workspace groups: pin per container (#52)', () => {
  const seedLevel = () => {
    let state = createWorkspace(emptyState, 'alpha', () => 'ws-1')
    state = createWorkspace(state, 'beta', () => 'ws-2')
    state = createGroup(state, 'gora', () => 'g-1')
    state = createGroup(state, 'dol', () => 'g-2')
    return state // order: ws-1, ws-2, g-1, g-2
  }

  it('a pinned group leads its level; unpin drops the key', () => {
    let state = seedLevel()
    state = setGroupPinned(state, 'g-2', true)

    // g-2 jumps to the HEAD of the top level: the pinned block leads.
    expect(displayIds(state)).toEqual(['g-2', 'ws-1', 'ws-2', 'g-1'])
    expect(state.groups.find((g) => g.id === 'g-2')?.pinned).toBe(true)

    const unpinned = setGroupPinned(state, 'g-2', false)
    // Unpinning inserts at the HEAD of the unpinned block — where it already
    // stands, so nobody moves; only the flag drops.
    expect(displayIds(unpinned)).toEqual(['g-2', 'ws-1', 'ws-2', 'g-1'])
    expect('pinned' in (unpinned.groups.find((g) => g.id === 'g-2') as object)).toBe(false)
  })

  it('a pinned group leads INSIDE its parent too (per container)', () => {
    let state = seedLevel()
    state = createGroup(state, 'dziecko-a', () => 'g-a')
    state = createGroup(state, 'dziecko-b', () => 'g-b')
    state = moveNode(state, 'g-a', { parentId: 'g-1' })
    state = moveNode(state, 'g-b', { parentId: 'g-1' })

    const next = setGroupPinned(state, 'g-b', true)

    // g-b leads WITHIN g-1's children, not at the top level.
    expect(displayIds(next)).toEqual(['ws-1', 'ws-2', 'g-1', 'g-b', 'g-a', 'g-2'])
  })

  it('pinned workspace + pinned group share ONE leading block at their level', () => {
    let state = seedLevel()
    state = setWorkspacePinned(state, 'ws-2', true)
    state = setGroupPinned(state, 'g-2', true)

    // The pinned block leads and keeps relative order (ws-2 pinned first,
    // g-2 pinned after); the unpinned follow.
    expect(displayIds(state)).toEqual(['ws-2', 'g-2', 'ws-1', 'g-1'])
  })

  it('a pinned WORKSPACE leads only within its group; moving keeps the pin local', () => {
    let state = seedLevel()
    state = moveNode(state, 'ws-1', { parentId: 'g-1' })
    state = moveNode(state, 'ws-2', { parentId: 'g-1' })
    state = setWorkspacePinned(state, 'ws-2', true)

    // ws-2 leads INSIDE g-1 only; the top level keeps its own order.
    expect(displayIds(state)).toEqual(['g-1', 'ws-2', 'ws-1', 'g-2'])

    // Move ws-2 to top level: the flag travels with it (pin stays local,
    // never global), and the move lands it like any other move — an append;
    // nothing re-sorts on a move.
    const moved = moveNode(state, 'ws-2', { parentId: null })
    expect(moved.workspaces.find((w) => w.id === 'ws-2')?.pinned).toBe(true)
    expect(displayIds(moved)).toEqual(['g-1', 'ws-1', 'g-2', 'ws-2'])
    // A fresh toggle in the NEW container applies the per-container order:
    // ws-2 leads the top level's pinned block.
    const repinned = setWorkspacePinned(setWorkspacePinned(moved, 'ws-2', false), 'ws-2', true)
    expect(displayIds(repinned)).toEqual(['ws-2', 'g-1', 'ws-1', 'g-2'])
  })

  it('inside a pinned group, its pinned children still lead (#52 composition)', () => {
    let state = seedLevel()
    state = moveNode(state, 'ws-1', { parentId: 'g-1' })
    state = moveNode(state, 'ws-2', { parentId: 'g-1' })
    state = setWorkspacePinned(state, 'ws-2', true) // ws-2 leads inside g-1
    state = setGroupPinned(state, 'g-1', true) // g-1 leads the top level

    expect(displayIds(state)).toEqual(['g-1', 'ws-2', 'ws-1', 'g-2'])
  })

  it('pin state round-trips through bootState', () => {
    let state = seedLevel()
    state = setGroupPinned(state, 'g-2', true)
    state = setWorkspacePinned(state, 'ws-1', true)

    const booted = bootState(
      state.workspaces,
      () => 'fresh',
      state.groups,
      state.order,
    )

    expect(booted.groups.find((g) => g.id === 'g-2')?.pinned).toBe(true)
    expect(booted.workspaces.find((w) => w.id === 'ws-1')?.pinned).toBe(true)
  })

  it('setGroupPinned is a no-op for an unknown id or an already-matching flag', () => {
    const state = seedLevel()
    expect(setGroupPinned(state, 'g-x', true)).toBe(state)

    const pinned = setGroupPinned(state, 'g-1', true)
    expect(setGroupPinned(pinned, 'g-1', true)).toBe(pinned)
  })
})

describe('batch actions over a multi-selection (#53)', () => {
  // Seed: g-1 { ws-1, ws-2 }, g-2, ws-3 at top level — one group with two
  // children, a second (empty) group as a move TARGET, and one top-level
  // workspace as a mixed-selection member.
  function seedBatch(): WorkspaceState {
    let state = emptyState
    state = createGroup(state, 'one', () => 'g-1')
    state = createWorkspace(state, 'ws one', seq('ws-1', 't-1', 'p-1'))
    state = createWorkspace(state, 'ws two', seq('ws-2', 't-2', 'p-2'))
    state = createGroup(state, 'two', () => 'g-2')
    state = createWorkspace(state, 'ws three', seq('ws-3', 't-3', 'p-3'))
    state = moveNode(state, 'ws-1', { parentId: 'g-1' })
    state = moveNode(state, 'ws-2', { parentId: 'g-1' })
    return state
  }

  describe('moveNodes', () => {
    it('moves a MIXED selection into a group in one operation, preserving sidebar order', () => {
      const state = seedBatch()

      // Deliberately unsorted input: a group AND a top-level workspace.
      const next = moveNodes(state, ['ws-3', 'g-1'], { parentId: 'g-2' })

      // g-1 nests into g-2 with its children following it; ws-3 appends
      // after them — the selection's sidebar order (g-1 before ws-3) holds.
      expect(displayIds(next)).toEqual(['g-2', 'g-1', 'ws-1', 'ws-2', 'ws-3'])
      expect(next.workspaces.find((w) => w.id === 'ws-3')?.groupId).toBe('g-2')
      expect(next.groups.find((g) => g.id === 'g-1')?.parentId).toBe('g-2')
    })

    it('reorders a selection BEFORE a sibling, preserving its relative order', () => {
      const state = seedBatch()

      // Both workspaces insert before g-2; each lands directly before it, so
      // the FIRST sidebar member ends up topmost of the batch.
      const next = moveNodes(state, ['ws-1', 'ws-3'], {
        parentId: null,
        beforeId: 'g-2',
      })

      expect(displayIds(next)).toEqual(['g-1', 'ws-2', 'ws-1', 'ws-3', 'g-2'])
    })

    it('moves the whole selection back to TOP LEVEL', () => {
      const state = seedBatch()

      const next = moveNodes(state, ['ws-1', 'ws-2'], { parentId: null })

      expect(displayIds(next)).toEqual(['g-1', 'g-2', 'ws-3', 'ws-1', 'ws-2'])
      expect(next.workspaces.find((w) => w.id === 'ws-1')?.groupId).toBeUndefined()
      expect(next.workspaces.find((w) => w.id === 'ws-2')?.groupId).toBeUndefined()
    })

    it('skips a cycle member (group into its own subtree) without aborting the rest', () => {
      let state = seedBatch()
      state = moveNode(state, 'g-2', { parentId: 'g-1' }) // g-1 → g-2

      // Drag g-1 (whose subtree now CONTAINS g-2) together with ws-3 into
      // g-2: the group's move is a cycle and is skipped; ws-3 still lands.
      const next = moveNodes(state, ['g-1', 'ws-3'], { parentId: 'g-2' })

      expect(next.groups.find((g) => g.id === 'g-1')?.parentId).toBeUndefined()
      expect(next.workspaces.find((w) => w.id === 'ws-3')?.groupId).toBe('g-2')
    })

    it('ignores unknown ids', () => {
      const state = seedBatch()
      const next = moveNodes(state, ['ws-3', 'ghost'], { parentId: 'g-2' })
      expect(displayIds(next)).toEqual(['g-1', 'ws-1', 'ws-2', 'g-2', 'ws-3'])
    })
  })

  describe('closeWorkspaces', () => {
    it('closes every selected workspace, keeping definitions; groups are ignored', () => {
      const state = seedBatch()

      const next = closeWorkspaces(state, ['ws-1', 'g-1', 'ws-3'])

      expect(next.openIds).toEqual(['ws-2'])
      expect(next.workspaces.map((w) => w.id)).toEqual(['ws-1', 'ws-2', 'ws-3'])
    })

    it('hands activation to a survivor when the active workspace closes', () => {
      const state = seedBatch() // active: ws-3 (last created)

      const next = closeWorkspaces(state, ['ws-3', 'ws-2'])

      expect(next.activeId).toBe('ws-1')
    })
  })

  describe('deleteNodes', () => {
    it('deletes a mixed selection: groups with their subtree, workspaces alone', () => {
      const state = seedBatch()

      // g-1 takes ws-1+ws-2 with it; ws-3 dies alone. Nothing survives.
      const next = deleteNodes(state, ['g-1', 'ws-3'])

      expect(next.workspaces).toEqual([])
      expect(next.groups.map((g) => g.id)).toEqual(['g-2'])
    })

    it('never double-deletes a workspace both selected directly and inside a selected group', () => {
      const state = seedBatch()

      // ws-1 is selected AND lives inside selected g-1.
      const next = deleteNodes(state, ['g-1', 'ws-1'])

      expect(next.workspaces.map((w) => w.id)).toEqual(['ws-3'])
      expect(next.groups.map((g) => g.id)).toEqual(['g-2'])
      expect(displayIds(next)).toEqual(['g-2', 'ws-3'])
    })

    it('cleans activation and open set when the selection removes everything active', () => {
      const state = seedBatch()

      const next = deleteNodes(state, ['ws-3', 'g-1'])

      // ws-3 died directly and g-1 took ws-1+ws-2 with it — NOTHING survives,
      // so activation falls all the way back to null (EmptyState).
      expect(next.workspaces).toEqual([])
      expect(next.activeId).toBeNull()
      expect(next.openIds).toEqual([])
    })
  })

  describe('setNodesPinned', () => {
    it('pins every selected node (groups and workspaces), each leading its own level', () => {
      const state = seedBatch()

      const next = setNodesPinned(state, ['ws-2', 'g-2'], true)

      // ws-2 leads INSIDE g-1 (pinning moves it to the head of its pinned
      // block); g-2 leads the top level.
      expect(displayIds(next)).toEqual(['g-2', 'g-1', 'ws-2', 'ws-1', 'ws-3'])
      expect(next.workspaces.find((w) => w.id === 'ws-2')?.pinned).toBe(true)
      expect(next.groups.find((g) => g.id === 'g-2')?.pinned).toBe(true)
    })

    it('unpins every selected node and drops the flags', () => {
      let state = seedBatch()
      state = setNodesPinned(state, ['ws-2', 'g-2'], true)

      const next = setNodesPinned(state, ['ws-2', 'g-2'], false)

      expect(next.workspaces.find((w) => w.id === 'ws-2')?.pinned).toBeUndefined()
      expect(next.groups.find((g) => g.id === 'g-2')?.pinned).toBeUndefined()
    })

    it('skips members already in the target state without jumping the list', () => {
      let state = seedBatch()
      state = setWorkspacePinned(state, 'ws-1', true) // ws-1 already pinned

      const next = setNodesPinned(state, ['ws-1', 'ws-2'], true)

      // ws-1 stays the head of g-1's pinned block; ws-2 joins behind it.
      expect(displayIds(next)).toEqual(['g-1', 'ws-1', 'ws-2', 'g-2', 'ws-3'])
    })
  })

  describe('batchDeleteWorkspaceCount', () => {
    it('counts selected workspaces plus everything inside selected groups', () => {
      const state = seedBatch()
      expect(batchDeleteWorkspaceCount(state, ['g-1', 'ws-3'])).toBe(3)
    })

    it('dedupes a workspace both selected directly and inside a selected group', () => {
      const state = seedBatch()
      expect(batchDeleteWorkspaceCount(state, ['g-1', 'ws-1'])).toBe(2)
    })

    it('ignores unknown ids and empty groups', () => {
      const state = seedBatch()
      expect(batchDeleteWorkspaceCount(state, ['g-2', 'ghost'])).toBe(0)
    })
  })
})

// --- Workspace colors (#69 / v1.5.0, stories 94–97) --------------------------
//
// Assumptions encoded by these tests:
//  - `color` is an OPTIONAL `string` on the Workspace model — one of the eight
//    fixed palette hexes, or absent (unset). Absent = exactly today's model
//    shape: the persisted payload must not gain the key (same key hygiene as
//    `pinned`/`collapsed`).
//  - `setWorkspaceColor(state, id, color | null)` is the single setter: a hex
//    sets/overwrites, `null` clears (drops the key). Unknown id = no-op.
//  - `COLOR_PALETTE` is the fixed eight, each with a display name — the menu
//    swatches and the tests both read it; no other colors exist in the app.
//  - "Picking the same swatch again unsets" is the UI toggle ON TOP of this
//    setter, tested in the component suite — the model itself is dumb.
//  - NOT tested here: Rust-side persistence (store_core suite), rendering.
describe('workspace colors (#69)', () => {
  const seedOne = (): WorkspaceState => ({
    ...emptyState,
    workspaces: [{ id: 'ws-1', name: 'alpha' }],
  })

  it('sets a color on the workspace and round-trips through bootState (set → persist → load)', () => {
    const withColor = setWorkspaceColor(seedOne(), 'ws-1', '#ec4899')

    expect(withColor.workspaces[0].color).toBe('#ec4899')

    // "Persist → load" is bootState in the pure model: the seeded config
    // must come back with the SAME value, untouched by migration.
    const booted = bootState(withColor.workspaces)
    expect(booted.workspaces[0].color).toBe('#ec4899')
  })

  it('overwrites an existing color with a different swatch', () => {
    let state = seedOne()
    state = setWorkspaceColor(state, 'ws-1', '#ec4899')
    state = setWorkspaceColor(state, 'ws-1', '#eab308')

    expect(state.workspaces[0].color).toBe('#eab308')
  })

  it('clearing drops the color key — byte-identical to a never-colored workspace', () => {
    let state = seedOne()
    state = setWorkspaceColor(state, 'ws-1', '#ec4899')
    state = setWorkspaceColor(state, 'ws-1', null)

    expect(state.workspaces[0]).not.toHaveProperty('color')
    // And a workspace that never had a color re-saves without the key.
    const untouched = bootState(state.workspaces)
    expect(untouched.workspaces[0]).not.toHaveProperty('color')
  })

  it('is a no-op for an unknown workspace id', () => {
    const state = seedOne()
    expect(setWorkspaceColor(state, 'ghost', '#ec4899')).toBe(state)
  })

  it('only touches the named workspace', () => {
    const state: WorkspaceState = {
      ...emptyState,
      workspaces: [
        { id: 'ws-1', name: 'alpha' },
        { id: 'ws-2', name: 'beta' },
      ],
    }
    const next = setWorkspaceColor(state, 'ws-2', '#60a5fa')

    expect(next.workspaces[0].color).toBeUndefined()
    expect(next.workspaces[1].color).toBe('#60a5fa')
  })

  it('COLOR_PALETTE is the fixed eight (dark-theme friendly, PRD hexes)', () => {
    expect(COLOR_PALETTE.map((c) => c.hex)).toEqual([
      '#4ade80', // light green
      '#16a34a', // dark green
      '#60a5fa', // light blue
      '#2563eb', // dark blue
      '#eab308', // yellow
      '#ef4444', // red
      '#ec4899', // pink
      '#a855f7', // purple
    ])
    // Every entry carries a display name for the menu's accessible label.
    for (const c of COLOR_PALETTE) {
      expect(c.name.trim()).not.toBe('')
    }
  })
})
