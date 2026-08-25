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
  bootState,
  panelIdsOf,
  upsertPanelCwd,
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

describe('workspace state', () => {
  describe('createWorkspace', () => {
    it('adds the workspace and makes it the active one', () => {
      const state = emptyState

      const next = createWorkspace(state, 'my-project', seq('ws-1', 'p-1'))

      expect(next.workspaces).toEqual([
        { id: 'ws-1', name: 'my-project', panels: [], layout: { kind: 'leaf', id: 'p-1' } },
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

    // v0.2 / #25 — a fresh workspace starts with a single-leaf layout tree
    // PERSISTED in the workspace itself (genId: ws id, then panel id).
    it('seeds a single-leaf layout tree inside the new workspace', () => {
      const ids = ['ws-1', 'p-1']
      let i = 0
      const next = createWorkspace(emptyState, 'my-project', () => ids[i++])

      expect(next.workspaces[0].layout).toEqual({ kind: 'leaf', id: 'p-1' })
      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-1'])
    })
  })

  describe('splitPanel', () => {
    // v0.2 / #25: splitPanel splits the ACTIVE panel's leaf; genId yields the
    // split-node id then the new leaf id.
    function single(): ReturnType<typeof createWorkspace> {
      const ids = ['ws-1', 'p-1']
      let i = 0
      return createWorkspace(emptyState, 'my-project', () => ids[i++])
    }

    it('splits the active panel into a horizontal split tree', () => {
      const gen = seq('s-1', 'p-2')
      const next = splitPanel(single(), 'ws-1', 'horizontal', gen)

      expect(next.workspaces[0].layout).toEqual({
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

      expect(next.workspaces[0].layout).toEqual({
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

      expect(leafIds(twice.workspaces[0].layout!)).toEqual(['p-1', 'p-2', 'p-3'])
      expect(twice.workspaces[0].layout).toEqual({
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
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))

      expect(panelIdsOf(state, 'ws-1')).toEqual(['p-1'])
    })

    it('keeps the existing panel id and adds a new one on split', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))

      const next = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-1', 'p-B'])
    })

    it('keeps the surviving panel id when one panel is closed', () => {
      let state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))
      state = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      const next = closePanel(state, 'ws-1', 'p-B')

      expect(panelIdsOf(next, 'ws-1')).toEqual(['p-1'])
      expect(next.workspaces[0].layout).toEqual({ kind: 'leaf', id: 'p-1' })
    })

    it('keeps the other panel when the first one is closed', () => {
      let state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))
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
        createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1')),
        'ws-1',
        'horizontal',
        seq('s-1', 'p-2'),
      )
    }

    it('updates the targeted split ratio, clamped by the container and minimum', () => {
      const next = resizePanel(splitOnce(), 'ws-1', 's-1', 0.25, { width: 100, height: 50 }, 10)

      expect(next.workspaces[0].layout).toEqual({
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
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))

      expect(resizePanel(state, 'nope', 's-1', 0.3, { width: 100, height: 50 })).toBe(state)
    })
  })

  describe('closePanel', () => {
    it('collapses a split workspace back to a single panel', () => {
      let state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))
      state = splitPanel(state, 'ws-1', 'horizontal', seq('s-1', 'p-B'))

      const next = closePanel(state, 'ws-1', 'p-B')

      expect(next.workspaces[0].layout).toEqual({ kind: 'leaf', id: 'p-1' })
    })

    it('is a no-op when the workspace only has one panel', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))

      expect(closePanel(state, 'ws-1', 'p-1')).toBe(state)
    })

    it('is a no-op for an unknown workspace id', () => {
      const state = createWorkspace(emptyState, 'my-project', seq('ws-1', 'p-1'))

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
    // focus tests have stable panel ids to point at. genId yields ws-1, p-1
    // for createWorkspace's workspace+seed-panel, then s-1, p-2 for the split.
    function splitState(): ReturnType<typeof splitPanel> {
      const gen = seq('ws-1', 'p-1', 's-1', 'p-2')
      const state = createWorkspace(emptyState, 'my-project', gen)
      return splitPanel(state, 'ws-1', 'horizontal', gen)
    }

    it('defaults to the first panel when none has been focused', () => {
      // A workspace with a tree but no activePanelId entry — e.g. state built
      // before focus was ever set. activePanelOf must fall back to leaf #1.
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
        workspaces: [{ id: 'ws-1', name: 'x', panels: [], layout: tree }],
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
      const gen = seq('ws-1', 'p-1', 's-1', 'p-2')
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
    it('keeps the persisted layout tree untouched (round-trips restart)', () => {
      const tree: LayoutNode = {
        kind: 'split',
        id: 's-1',
        orientation: 'horizontal',
        ratio: 0.7,
        first: { kind: 'leaf', id: 'p-1' },
        second: { kind: 'leaf', id: 'p-2' },
      }

      const state = bootState([{ id: 'ws-1', name: 'my-project', panels: [], layout: tree }], seq())

      expect(state.workspaces[0].layout).toEqual(tree)
      expect(panelIdsOf(state, 'ws-1')).toEqual(['p-1', 'p-2'])
    })

    it('seeds a fresh single leaf for a workspace without a layout (old config)', () => {
      const state = bootState([{ id: 'ws-1', name: 'my-project', panels: [] }], seq('fresh-1'))

      expect(state.workspaces[0].layout).toEqual({ kind: 'leaf', id: 'fresh-1' })
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
        seq(),
      )

      expect(state.openIds).toEqual(['ws-1', 'ws-2'])
      expect(state.activeId).toBe('ws-1')
      expect(activePanelOf(state, 'ws-1')).toBe('a')
      expect(activePanelOf(state, 'ws-2')).toBe('c')
    })

    it('handles an empty config with no active workspace', () => {
      const state = bootState([], seq())

      expect(state.activeId).toBeNull()
      expect(state.openIds).toEqual([])
    })
  })
})

// --- v0.2 Phase 5 / #29: per-panel cwd metadata (snapshot + restore) ---------

describe('session snapshot metadata (#29)', () => {
  // One workspace, one leaf p-1, carrying a cwd + ssh target in panels[].
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
          layout: { kind: 'leaf', id: 'p-1' } as LayoutNode,
        },
      ],
      activeId: 'ws-1',
      openIds: ['ws-1'],
      activePanelId: { 'ws-1': 'p-1' },
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
  }

  expect(upsertPanelCwd(state, 'p-1', '/x')).toBe(state)
  expect(upsertPanelCwd(state, 'p-1', '/y')).not.toBe(state)
})
