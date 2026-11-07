// Assumptions encoded by these tests (op-mapping parity suite, #125/#127):
//  - applyGatewayOp is the ENTIRE live layout surface: every op maps onto the
//    SAME pure ops the UI actions use, so "CLI outcome == UI click outcome"
//    reduces to "the mapping calls the op the click calls". Each parity case
//    below runs BOTH paths on equivalent states and compares outcomes.
//  - Addressing: workspaces/groups accept id OR name; tabs/panes are
//    id-addressed. Unknown targets answer the enumerated error, never a
//    silent no-op (a script chaining echoes must not lie).
//  - Results echo the affected entity ids (created/reordered/moved/closed).
//  - genId is injected (deterministic ids in tests).
//  - Boundary NOT tested here: the wire (framing/hello) — that is the Rust
//    gateway suite's job — and PTY I/O (tabs.send resolves its target here;
//    the write itself rides pty_write in the shell).

import { describe, it, expect } from 'vitest'
import { applyGatewayOp, resolveSendTarget, type GatewayError } from './gatewayOps'
import {
  emptyState,
  bootState,
  createWorkspace,
  addTab,
  splitPanel,
  createGroup,
  moveNode,
  deleteGroupSubtree,
  unpackGroup,
  toggleCollapse,
  setWorkspacePinned,
  setWorkspaceColor,
  setGroupPinned,
  setTabPinned,
  setTabColor,
  renameWorkspace,
  renameTab,
  renameGroup,
  moveTabsToWorkspace,
  moveTab,
  separateTab,
  closeTabs,
  closeWorkspace,
  deleteWorkspace,
  openWorkspace,
  focusPanel,
  toggleZoom,
  resizePanel,
  type Tab,
  type Workspace,
  type WorkspaceState,
} from './workspaces'
import { leafIds } from './PaneLayout'

// One generator per FIXTURE BUILD — every id unique within the state (the
// workspaces.ts ops consume genId in pinned order, tests rely on it).
const seq = (): (() => string) => {
  let n = 0
  return () => `g${(n += 1)}`
}

// Fresh generator per applyGatewayOp call so ids never collide across the
// compared halves (both sides of a parity case start at x1 — deterministic).
const freshSeq = (): (() => string) => {
  let n = 0
  return () => `x${(n += 1)}`
}

// A seeded state: two workspaces (one with an extra tab), one group holding
// the second workspace — the addressing surface every op needs.
function seed(): WorkspaceState {
  const gen = seq()
  let s = emptyState
  s = createWorkspace(s, 'projekt', gen)
  s = createWorkspace(s, 'drugi', gen)
  s = createGroup(s, 'grupa', gen)
  const grupa = s.groups[s.groups.length - 1]
  const drugi = s.workspaces.find((w) => w.name === 'drugi') as { id: string }
  s = moveNode(s, drugi.id, { parentId: grupa.id })
  // A second tab on the active workspace.
  s = addTab(s, s.workspaces[0].id, gen)
  return s
}

const idOf = (s: WorkspaceState, name: string): string => {
  const ws = s.workspaces.find((w) => w.name === name)
  if (ws == null) throw new Error(`missing fixture workspace ${name}`)
  return ws.id
}
const gidOf = (s: WorkspaceState, name: string): string => {
  const g = s.groups.find((g) => g.name === name)
  if (g == null) throw new Error(`missing fixture group ${name}`)
  return g.id
}

// The one shape both halves of a parity case return.
function ok(outcome: ReturnType<typeof applyGatewayOp>): {
  state: WorkspaceState
  result: Record<string, unknown>
} {
  if (!outcome.ok) throw new Error(`expected ok, got ${JSON.stringify(outcome.error)}`)
  return outcome
}

function fail(outcome: ReturnType<typeof applyGatewayOp>): GatewayError {
  if (outcome.ok) throw new Error(`expected error, got ${JSON.stringify(outcome.result)}`)
  return outcome.error
}

describe('gatewayOps: workspaces (parity with the UI ops)', () => {
  it('workspaces.create mirrors createWorkspace and echoes every id', () => {
    const s = seed()
    const out = ok(applyGatewayOp(s, 'workspaces.create', { name: 'nowy' }, freshSeq()))
    const expected = createWorkspace(s, 'nowy', freshSeq())
    expect(out.state.workspaces.map((w) => w.name)).toEqual(
      expected.workspaces.map((w) => w.name),
    )
    const created = out.result.created as { id: string; tabId: string; panelId: string }
    const ws = out.state.workspaces.find((w) => w.id === created.id)
    expect(ws?.tabs?.[0].id).toBe(created.tabId)
    expect(leafIds(ws?.tabs?.[0].layout as never)[0]).toBe(created.panelId)
  })

  it('workspaces.rename/delete match renameWorkspace/deleteWorkspace by name and id', () => {
    const s = seed()
    const id = idOf(s, 'projekt')
    const renamed = ok(applyGatewayOp(s, 'workspaces.rename', { workspace: 'projekt', to: 'proj' }, seq()))
    expect(renamed.result.renamed).toEqual({ id, from: 'projekt', to: 'proj' })
    expect(renamed.state).toEqual(renameWorkspace(s, id, 'proj'))
    // id addressing: same outcome through the id.
    const byId = ok(applyGatewayOp(renamed.state, 'workspaces.rename', { workspace: id, to: 'proj2' }, seq()))
    expect(byId.state).toEqual(renameWorkspace(renamed.state, id, 'proj2'))
    const deleted = ok(applyGatewayOp(byId.state, 'workspaces.delete', { workspace: 'proj2' }, seq()))
    expect(deleted.state).toEqual(deleteWorkspace(byId.state, id))
    expect(deleted.result.deleted).toEqual({ id, name: 'proj2' })
  })

  it('workspaces.close keeps definitions and echoes ids (multi)', () => {
    const s = seed()
    const a = idOf(s, 'projekt')
    const b = idOf(s, 'drugi')
    const out = ok(applyGatewayOp(s, 'workspaces.close', { workspaces: [a, 'drugi'] }, seq()))
    const expected = closeWorkspace(closeWorkspace(s, a), b)
    expect(out.state).toEqual(expected)
    expect(out.result.closed).toEqual([a, b])
  })

  it('workspaces.pin/unpin/color/move/reorder/open mirror their UI ops', () => {
    const s = seed()
    const a = idOf(s, 'projekt')
    const grupa = gidOf(s, 'grupa')
    expect(ok(applyGatewayOp(s, 'workspaces.pin', { workspaces: ['projekt'] }, seq())).state)
      .toEqual(setWorkspacePinned(s, a, true))
    expect(ok(applyGatewayOp(s, 'workspaces.unpin', { workspaces: ['projekt'] }, seq())).state)
      .toEqual(setWorkspacePinned(s, a, false))
    expect(ok(applyGatewayOp(s, 'workspaces.color', { workspaces: [a], color: 'red' }, seq())).state)
      .toEqual(setWorkspaceColor(s, a, '#ef4444'))
    expect(
      ok(applyGatewayOp(s, 'workspaces.color', { workspaces: [a], color: '#EF4444' }, seq())).state,
    ).toEqual(setWorkspaceColor(s, a, '#ef4444'))
    const cleared = ok(applyGatewayOp(s, 'workspaces.color', { workspaces: [a], color: 'clear' }, seq()))
    expect(cleared.state).toEqual(setWorkspaceColor(s, a, null))
    expect(cleared.result.color).toBeNull()
    // move by NAME into an existing group + move to a NEW group.
    const moved = ok(applyGatewayOp(s, 'workspaces.move', { workspaces: ['projekt'], group: 'grupa' }, seq()))
    expect(moved.state).toEqual(moveNode(s, a, { parentId: grupa }))
    const fresh = ok(applyGatewayOp(s, 'workspaces.move', { workspaces: ['projekt'], newGroup: 'świeża' }, seq()))
    const newGroup = fresh.state.groups.find((g) => g.name === 'świeża')
    expect(newGroup).toBeTruthy()
    expect(fresh.result.newGroup).toEqual({ id: (newGroup as { id: string }).id, name: 'świeża' })
    // reorder: before a sibling; absent before appends.
    const reordered = ok(applyGatewayOp(s, 'workspaces.reorder', { workspace: 'projekt', before: 'drugi' }, seq()))
    expect(reordered.state.order.indexOf(a)).toBeLessThan(reordered.state.order.indexOf(b_id(s)))
    const appended = ok(applyGatewayOp(s, 'workspaces.reorder', { workspace: 'projekt' }, seq()))
    // "Append at the end of its level" means after the LAST top-level
    // sibling in the interleaved order — the group node may sit between.
    const order = appended.state.order
    expect(order.indexOf(a)).toBeGreaterThan(order.indexOf(gidOf(s, 'grupa')))
    const opened = ok(applyGatewayOp(s, 'workspaces.open', { workspace: 'drugi' }, seq()))
    expect(opened.state).toEqual(openWorkspace(s, idOf(s, 'drugi')))
    expect(opened.result.opened).toEqual({ id: idOf(s, 'drugi'), name: 'drugi' })
  })

  const b_id = (s: WorkspaceState): string => idOf(s, 'drugi')

  it('unknown workspace targets answer workspaceNotFound, not a silent no-op', () => {
    const s = seed()
    const err = fail(applyGatewayOp(s, 'workspaces.rename', { workspace: 'nie-ma', to: 'x' }, seq()))
    expect(err.code).toBe('workspaceNotFound')
    expect(err.retryable).toBe(false)
    expect(fail(applyGatewayOp(s, 'workspaces.color', { workspaces: ['nie-ma'], color: 'red' }, seq())).code)
      .toBe('workspaceNotFound')
  })

  it('unknown colors and missing params answer badParams with the palette hint', () => {
    const s = seed()
    const err = fail(applyGatewayOp(s, 'workspaces.color', { workspaces: ['projekt'], color: '#ff0000' }, seq()))
    expect(err.code).toBe('badParams')
    expect(err.next.join(' ')).toContain('red')
    expect(fail(applyGatewayOp(s, 'workspaces.create', {}, seq())).code).toBe('badParams')
    expect(fail(applyGatewayOp(s, 'workspaces.move', { workspaces: ['projekt'] }, seq())).code).toBe('badParams')
  })
})

describe('gatewayOps: groups (parity with the UI ops)', () => {
  it('create/rename/unpack/collapse mirror the group ops (nesting included)', () => {
    const s = seed()
    const created = ok(applyGatewayOp(s, 'groups.create', { name: 'nowa', parent: 'grupa' }, seq()))
    const parent = gidOf(s, 'grupa')
    const nested = created.state.groups.find((g) => g.name === 'nowa') as { id: string; parentId?: string }
    expect(nested.parentId).toBe(parent)
    const plain = ok(applyGatewayOp(s, 'groups.create', { name: 'sama' }, seq()))
    const last = plain.state.groups[plain.state.groups.length - 1] as { parentId?: string }
    expect(last.parentId).toBeUndefined()
    const gid = gidOf(s, 'grupa')
    expect(ok(applyGatewayOp(s, 'groups.rename', { group: 'grupa', to: 'nowa-nazwa' }, seq())).state)
      .toEqual(renameGroup(s, gid, 'nowa-nazwa'))
    const unpacked = ok(applyGatewayOp(s, 'groups.unpack', { group: 'grupa' }, seq()))
    expect(unpacked.state).toEqual(unpackGroup(s, gid))
    expect(unpacked.result.unpacked).toEqual({ id: gid })
    const withSama = plain.state
    const collapsed = ok(applyGatewayOp(withSama, 'groups.collapse', { group: 'sama' }, seq()))
    const sama = collapsed.state.groups.find((g) => g.name === 'sama')
    expect(sama?.collapsed).toBe(true)
    const expanded = ok(applyGatewayOp(collapsed.state, 'groups.expand', { group: 'sama' }, seq()))
    expect(expanded.state).toEqual(toggleCollapse(collapsed.state, (sama as { id: string }).id))
    expect(expanded.result.expanded).toBe((sama as { id: string }).id)
  })

  it('groups.delete refuses a non-empty group without subtree and matches deleteGroupSubtree with it', () => {
    const s = seed()
    const gid = gidOf(s, 'grupa')
    const refused = fail(applyGatewayOp(s, 'groups.delete', { group: 'grupa' }, seq()))
    expect(refused.code).toBe('groupNotEmpty')
    expect(refused.message).toContain('--subtree')
    const subtree = ok(applyGatewayOp(s, 'groups.delete', { group: 'grupa', subtree: true }, seq()))
    expect(subtree.state).toEqual(deleteGroupSubtree(s, gid))
    expect(subtree.result.deleted).toEqual({
      groups: expect.arrayContaining([gid]),
      workspaces: [idOf(s, 'drugi')],
    })
    // An EMPTY group deletes without the flag (the UI's bare confirm).
    const fresh = ok(applyGatewayOp(seed(), 'groups.create', { name: 'pusta' }, seq()))
    const emptyId = (fresh.state.groups[fresh.state.groups.length - 1] as { id: string }).id
    const gone = ok(applyGatewayOp(fresh.state, 'groups.delete', { group: 'pusta' }, seq()))
    expect(gone.state.groups.some((g) => g.id === emptyId)).toBe(false)
  })

  it('groups.pin/color/move mirror their UI ops and answer groupNotFound for strangers', () => {
    const s = seed()
    const gid = gidOf(s, 'grupa')
    expect(ok(applyGatewayOp(s, 'groups.pin', { groups: ['grupa'] }, seq())).state)
      .toEqual(setGroupPinned(s, gid, true))
    expect(ok(applyGatewayOp(s, 'groups.color', { groups: ['grupa'], color: 'purple' }, seq())).result.color)
      .toBe('#a855f7')
    const err = fail(applyGatewayOp(s, 'groups.rename', { group: 'nie-ma', to: 'x' }, seq()))
    expect(err.code).toBe('groupNotFound')
  })
})

describe('gatewayOps: tabs (parity with the UI ops)', () => {
  it('tabs.create honors shell + name and echoes the chainable ids', () => {
    const s = seed()
    const wsId = idOf(s, 'projekt')
    const out = ok(
      applyGatewayOp(s, 'tabs.create', { workspace: 'projekt', shell: '/bin/zsh', name: 'build' }, freshSeq()),
    )
    const created = out.result.created as { id: string; panelId: string; name: string | null }
    const ws = out.state.workspaces.find((w) => w.id === wsId) as { tabs: { id: string; name?: string; shell?: string }[] }
    const tab = ws.tabs.find((t) => t.id === created.id) as { name?: string; shell?: string }
    expect(tab.shell).toBe('/bin/zsh')
    expect(tab.name).toBe('build')
    expect(created.name).toBe('build')
    expect(created.panelId).toBeTruthy()
    // Unknown workspace → workspaceNotFound (scripts must not silently misfire).
    expect(fail(applyGatewayOp(s, 'tabs.create', { workspace: 'nie-ma' }, seq())).code).toBe('workspaceNotFound')
  })

  it('tabs.close/rename/pin/color mirror their UI ops on id-addressed tabs', () => {
    const s = seed()
    const wsId = idOf(s, 'projekt')
    const ws = s.workspaces.find((w) => w.id === wsId) as { tabs: { id: string }[] }
    const extraTab = ws.tabs[1].id
    const pinned = ok(applyGatewayOp(s, 'tabs.pin', { tabs: [extraTab] }, seq()))
    expect(pinned.state).toEqual(setTabPinned(s, wsId, extraTab, true))
    const colored = ok(applyGatewayOp(pinned.state, 'tabs.color', { tabs: [extraTab], color: 'yellow' }, seq()))
    expect(colored.state).toEqual(setTabColor(pinned.state, wsId, extraTab, '#eab308'))
    const renamed = ok(applyGatewayOp(colored.state, 'tabs.rename', { tab: extraTab, to: 'serwer' }, seq()))
    expect(renamed.state).toEqual(renameTab(colored.state, wsId, extraTab, 'serwer'))
    expect(renamed.result.renamed).toEqual({ id: extraTab, to: 'serwer' })
    // closing the LAST tab of a workspace is a no-op (close the workspace).
    const single = createWorkspace(emptyState, 'solo', seq())
    const soloTab = (single.workspaces[0].tabs as { id: string }[])[0].id
    const closed = ok(applyGatewayOp(single, 'tabs.close', { tabs: [soloTab] }, seq()))
    expect((closed.state.workspaces[0].tabs as unknown[]).length).toBe(1)
    // multi close across ids mirrors closeTabs.
    const both = s.workspaces.find((w) => w.id === wsId) as { tabs: { id: string }[] }
    const t0 = both.tabs[0].id
    const out = ok(applyGatewayOp(s, 'tabs.close', { tabs: [t0, extraTab] }, seq()))
    // The last survivor stays — closeTabs's guard, applied sequentially.
    expect(out.state).toEqual(closeTabs(s, wsId, [t0, extraTab]))
    expect(fail(applyGatewayOp(s, 'tabs.close', { tabs: ['nie-takiego'] }, seq())).code).toBe('tabNotFound')
  })

  it('tabs.reorder/move/separate/open mirror their UI ops', () => {
    const s = seed()
    const wsId = idOf(s, 'projekt')
    const tabs = (s.workspaces.find((w) => w.id === wsId) as { tabs: { id: string }[] }).tabs
    const out = ok(applyGatewayOp(s, 'tabs.reorder', { tab: tabs[1].id, index: 0 }, seq()))
    expect(out.state).toEqual(moveTab(s, wsId, tabs[1].id, 0))
    expect(out.result.reordered).toEqual({ id: tabs[1].id, index: 0 })
    // move to the other workspace (by id AND name).
    const target = idOf(s, 'drugi')
    const moved = ok(applyGatewayOp(s, 'tabs.move', { tabs: [tabs[1].id], workspace: 'drugi' }, seq()))
    expect(moved.state).toEqual(moveTabsToWorkspace(s, wsId, [tabs[1].id], target))
    // cross-workspace batch refuses (no UI counterpart): one tab from the
    // project, one from the other workspace.
    const otherTab = (s.workspaces.find((w) => w.id === target) as { tabs: { id: string }[] }).tabs[0].id
    expect(fail(applyGatewayOp(s, 'tabs.move', { tabs: [tabs[0].id, otherTab], workspace: 'drugi' }, seq())).code)
      .toBe('badParams')
    // separate: needs a SPLIT tab. Each half mints from a FRESH generator —
    // same start ids, so the parity claim is about the op, not the counter.
    const split = splitPanel(s, wsId, 'horizontal', seq())
    const activeTab = split.activeTabId[wsId]
    const sep = ok(applyGatewayOp(split, 'tabs.separate', { tab: activeTab }, seq()))
    expect(sep.state).toEqual(separateTab(split, wsId, activeTab, seq()))
    const createdIds = (sep.result.separated as { created: string[] }).created
    expect(createdIds.length).toBeGreaterThan(0)
    // tabs.open activates owner workspace + tab.
    const opened = ok(applyGatewayOp(s, 'tabs.open', { tab: tabs[1].id }, seq()))
    expect(opened.state.activeId).toBe(wsId)
    expect(opened.state.activeTabId[wsId]).toBe(tabs[1].id)
  })
})

describe('gatewayOps: panes (parity with the UI ops)', () => {
  it('panes.split mirrors splitPanel and echoes the new panel id', () => {
    const s = seed()
    const wsId = idOf(s, 'projekt')
    const out = ok(applyGatewayOp(s, 'panes.split', { workspace: 'projekt', orientation: 'vertical' }, freshSeq()))
    expect(out.state).toEqual(splitPanel(s, wsId, 'vertical', freshSeq()))
    expect((out.result.split as { orientation: string }).orientation).toBe('vertical')
    expect(fail(applyGatewayOp(s, 'panes.split', { workspace: 'projekt', orientation: 'skos' }, seq())).code)
      .toBe('badParams')
  })

  it('panes.resize maps the pane id onto its owning split (the drag twin)', () => {
    const s = splitPanel(seed(), idOf(seed(), 'projekt'), 'horizontal', seq())
    const wsId = idOf(s, 'projekt')
    const ws = s.workspaces.find((w) => w.id === wsId) as Workspace
    const activeTabId = s.activeTabId[wsId]
    const activeTab = ws.tabs?.find((t) => t.id === activeTabId) ?? null
    expect(activeTab).toBeTruthy()
    const leaves = leafIds((activeTab as Tab).layout)
    const pane = leaves[0]
    const out = ok(applyGatewayOp(s, 'panes.resize', { pane, ratio: 0.7 }, seq()))
    expect((out.result.resized as { ratio: number }).ratio).toBe(0.7)
    // The persisted ratio lands on the tree — the drag's own outcome.
    const expectedSplitId = findSplitId(out.state, wsId, pane)
    expect(expectedSplitId).toBeTruthy()
    const expected = resizePanel(s, wsId, expectedSplitId as string, 0.7, { width: 1000, height: 1000 })
    expect(out.state).toEqual(expected)
    // A single-leaf pane has nothing to resize.
    const solo = createWorkspace(emptyState, 'solo', seq())
    const soloTab = (solo.workspaces[0].tabs as Tab[])[0]
    const soloPane = leafIds(soloTab.layout)[0]
    expect(fail(applyGatewayOp(solo, 'panes.resize', { pane: soloPane, ratio: 0.7 }, seq())).code).toBe('badParams')
    expect(fail(applyGatewayOp(s, 'panes.resize', { pane, ratio: 3 }, seq())).code).toBe('badParams')
  })

  it('panes.zoom/focus mirror their UI ops; unknown panes refuse', () => {
    const s = splitPanel(seed(), idOf(seed(), 'projekt'), 'horizontal', seq())
    const wsId = idOf(s, 'projekt')
    const activeTabId = s.activeTabId[wsId]
    const ws = s.workspaces.find((w) => w.id === wsId) as Workspace
    const activeTab = (ws.tabs as Tab[]).find((t) => t.id === activeTabId) as Tab
    const leaves = leafIds(activeTab.layout)
    const out = ok(applyGatewayOp(s, 'panes.zoom', { pane: leaves[0] }, seq()))
    expect(out.state).toEqual(toggleZoom(s, wsId, leaves[0]))
    expect((out.result.zoomed as string | null)).toBe(leaves[0])
    const focused = ok(applyGatewayOp(s, 'panes.focus', { pane: leaves[1] }, seq()))
    expect(focused.state).toEqual(focusPanel(s, wsId, leaves[1]))
    expect(fail(applyGatewayOp(s, 'panes.focus', { pane: 'nie-taki' }, seq())).code).toBe('paneNotFound')
  })
})

describe('gatewayOps: tabs.send target resolution (#127)', () => {
  it('the active tab sends to its FOCUSED panel; a background tab to its first panel', () => {
    const s = seed()
    const wsId = idOf(s, 'projekt')
    const tabs = (s.workspaces.find((w) => w.id === wsId) as { tabs: { id: string }[] }).tabs
    // Active tab → focused panel (activePanelOf's fallback is the first).
    const active = resolveSendTarget(s, tabs[0].id)
    expect(active?.workspaceId).toBe(wsId)
    expect(active?.panelId).toBeTruthy()
    // Background tab → its first leaf.
    const background = resolveSendTarget(s, tabs[1].id)
    expect(background?.panelId).toBeTruthy()
    expect(active?.panelId).not.toBe(background?.panelId)
    expect(resolveSendTarget(s, 'nie-taki')).toBeNull()
  })
})

describe('gatewayOps: unknown ops', () => {
  it('an op outside the catalog answers unknownOp (additive growth)', () => {
    const err = fail(applyGatewayOp(emptyState, 'system.shutdown', {}, seq()))
    expect(err.code).toBe('unknownOp')
  })
})

// The split node whose DIRECT children hold `pane` — the same lookup
// panes.resize performs; duplicated here so the expectation is independent
// of the implementation under test.
function findSplitId(state: WorkspaceState, wsId: string, pane: string): string | null {
  const ws = state.workspaces.find((w) => w.id === wsId)
  const tabId = state.activeTabId[wsId]
  const tab = ws?.tabs?.find((t) => t.id === tabId)
  if (tab == null) return null
  const walk = (node: any): string | null => {
    if (node == null || typeof node !== 'object') return null
    if (node.first == null) return null
    const inFirst = contains(node.first)
    const inSecond = contains(node.second)
    if (inFirst || inSecond) return node.id as string
    return walk(node.first) ?? walk(node.second)
  }
  const contains = (node: any): boolean => {
    if (node == null || typeof node !== 'object') return false
    if (node.first == null) return node.id === pane
    return contains(node.first) || contains(node.second)
  }
  return walk(tab.layout as any)
}

// bootState parity: the gateway ops operate on BOOTED stores too (the app's
// real entry shape), not just emptyState-built fixtures.
describe('gatewayOps on a booted store', () => {
  it('workspaces.create lands at top level and activates (booted shape)', () => {
    const booted = bootState([
      { id: 'ws-1', name: 'stary', panels: [], tabs: [{ id: 't-1', layout: { kind: 'leaf', id: 'p-1' } }] },
    ])
    const out = ok(applyGatewayOp(booted, 'workspaces.create', { name: 'nowy' }, freshSeq()))
    expect(out.state.order[out.state.order.length - 1]).toBe((out.result.created as { id: string }).id)
    expect(out.state.activeId).toBe((out.result.created as { id: string }).id)
  })
})
