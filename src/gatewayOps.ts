// gatewayOps — the CLI-facing op catalog (#125, v1.8.0 phase 3).
//
// Every layout mutation the socket gateway accepts maps here onto the SAME
// pure ops the UI actions use, so a CLI command and the matching UI click
// cannot diverge: one module, one mapping, both callers. The module is PURE
// — state in, state + result doc out, genId injected — which is what the
// op-mapping parity suite exercises without a live terminal.
//
// Addressing: workspaces and groups accept an id OR a name (first match,
// the CLI's offline convention); tabs and panes are id-addressed (created
// ids are echoed in results so automation can chain commands). Unknown
// targets answer the enumerated error, never a silent no-op — a silent
// no-op would make a scripted chain lie about what it did.
//
// Results echo the affected entity ids ({created: {id, …}}, {closed: […]})
// so the NEXT command can address what this one made.

import {
  addTab,
  activePanelOf,
  activeTabOf,
  closeTabs,
  closeWorkspace,
  createGroup,
  createWorkspace,
  deleteGroup,
  deleteGroupSubtree,
  deleteWorkspace,
  COLOR_PALETTE,
  focusPanel,
  groupSubtreeIds,
  isGroupEmpty,
  moveNode,
  moveTab,
  moveTabsToWorkspace,
  moveToNewGroup,
  moveNodes,
  openWorkspace,
  panelLocationOf,
  renameGroup,
  renameTab,
  renameWorkspace,
  separateTab,
  setGroupColor,
  setNodesPinned,
  setTabColor,
  setTabPinned,
  setWorkspaceColor,
  splitPanel,
  switchTab,
  switchWorkspace,
  toggleCollapse,
  unpackGroup,
  closePanel,
  resizePanel,
  toggleZoom,
  type Group,
  type Tab,
  type Workspace,
  type WorkspaceState,
} from './workspaces'
import { leafIds, type Orientation } from './PaneLayout'

/// The wire error shape — the protocol's `{code, message, next, retryable}`
/// object, mirrored so the frontend answers carry the catalog codes.
export type GatewayError = {
  code: string
  message: string
  next: string[]
  retryable: false
}

const err = (code: string, message: string, next: string[] = []): GatewayError => ({
  code,
  message,
  next,
  retryable: false,
})

export type OpOutcome =
  | { ok: true; state: WorkspaceState; result: Record<string, unknown> }
  | { ok: false; error: GatewayError }

/// Resolve a workspace by id FIRST, then by name (first match) — the CLI's
/// offline convention, so a script can address either spelling live.
function resolveWorkspace(state: WorkspaceState, ref: unknown): Workspace | null {
  if (typeof ref !== 'string' || ref === '') return null
  return (
    state.workspaces.find((w) => w.id === ref) ??
    state.workspaces.find((w) => w.name === ref) ??
    null
  )
}

/// Resolve a group by id first, then by name.
function resolveGroup(state: WorkspaceState, ref: unknown): Group | null {
  if (typeof ref !== 'string' || ref === '') return null
  return state.groups.find((g) => g.id === ref) ?? state.groups.find((g) => g.name === ref) ?? null
}

/// Find a tab by id across every workspace (tab ids are global uuids).
function findTab(state: WorkspaceState, tabId: unknown): { ws: Workspace; tab: Tab } | null {
  if (typeof tabId !== 'string' || tabId === '') return null
  for (const ws of state.workspaces) {
    const tab = ws.tabs?.find((t) => t.id === tabId)
    if (tab != null) return { ws, tab }
  }
  return null
}

/// A palette color by name or exact hex (case-insensitive), or null for
/// "clear". `undefined` = no color param given (caller decides).
function parseColor(value: unknown): { ok: true; color: string | null } | { ok: false; error: GatewayError } {
  if (value === 'clear' || value === null) return { ok: true, color: null }
  if (typeof value !== 'string' || value === '') {
    return {
      ok: false,
      error: err(
        'badParams',
        'params need "color": a palette name, a palette hex, or "clear"',
        [`palette: ${COLOR_PALETTE.map((c) => c.name).join(', ')}`],
      ),
    }
  }
  const lowered = value.toLowerCase()
  const byHex = COLOR_PALETTE.find((c) => c.hex.toLowerCase() === lowered)
  if (byHex != null) return { ok: true, color: byHex.hex }
  const byName = COLOR_PALETTE.find((c) => c.name.toLowerCase() === lowered)
  if (byName != null) return { ok: true, color: byName.hex }
  return {
    ok: false,
    error: err(
      'badParams',
      `unknown color "${value}" — use a palette name, a palette hex, or "clear"`,
      [`palette: ${COLOR_PALETTE.map((c) => `${c.name} (${c.hex})`).join(', ')}`],
    ),
  }
}

const notFound = (kind: string, ref: string, hint: string): GatewayError =>
  err(`${kind}NotFound`, `no ${kind} matching "${ref}"`, [hint])

const listParam = (params: Record<string, unknown>, key: string): string[] => {
  const value = params[key]
  if (typeof value === 'string' && value !== '') return [value]
  if (Array.isArray(value)) return value.filter((v): v is string => typeof v === 'string' && v !== '')
  return []
}

/// Split one tab's tree id reference out of params.
function str(params: Record<string, unknown>, key: string): string | null {
  const v = params[key]
  return typeof v === 'string' && v !== '' ? v : null
}

/// The parent split id of a leaf, or null when the leaf IS the whole tree.
/// `panes resize` addresses the DRAG counterpart by the pane id, so the op
/// maps leaf → owning split here (PaneLayout owns the tree itself).
function splitIdOfLeaf(state: WorkspaceState, wsId: string, paneId: string): string | null {
  const tab = activeTabOf(state, wsId)
  if (tab == null) return null
  const walk = (node: any): string | null => {
    if (node == null || typeof node !== 'object') return null
    if (node.first == null && node.second == null) return null
    const inFirst = containsLeaf(node.first)
    const inSecond = containsLeaf(node.second)
    if (inFirst || inSecond) return node.id
    return walk(node.first) ?? walk(node.second)
  }
  const containsLeaf = (node: any): boolean => {
    if (node == null || typeof node !== 'object') return false
    if (node.first == null && node.second == null) return node.id === paneId
    return containsLeaf(node.first) || containsLeaf(node.second)
  }
  return walk(tab.layout as any)
}

/// The synthetic extent `panes resize` clamps against: the CLI has no
/// measured rect (the drag had one), so the op clamps at the same relative
/// minimum over a 1000px stand-in — the ratio the user asked for survives
/// unless a pane would drop under the shared minimum.
const SYNTHETIC_EXTENT = { width: 1000, height: 1000 }

/// Where `tabs.send` lands (#127): the tab's FOCUSED panel when its
/// workspace is active (the keystrokes path), else the tab's first panel —
/// the shell the text executes in. Null for an unknown tab. Pure so the
/// op-mapping suite pins the resolution without a live terminal.
export function resolveSendTarget(
  state: WorkspaceState,
  tabId: string,
): { workspaceId: string; tabId: string; panelId: string } | null {
  if (typeof tabId !== 'string' || tabId === '') return null
  for (const ws of state.workspaces) {
    const tab = ws.tabs?.find((t) => t.id === tabId)
    if (tab == null) continue
    const isActive = state.activeTabId[ws.id] === tab.id
    const panelId = isActive
      ? (activePanelOf(state, ws.id) ?? leafIds(tab.layout)[0])
      : leafIds(tab.layout)[0]
    if (panelId == null) return null
    return { workspaceId: ws.id, tabId: tab.id, panelId }
  }
  return null
}

/// The layout op catalog (#125) — every op [`applyGatewayOp`] serves, named
/// once for the parity fixture suite: the fixture holds this list against
/// the plan's UI inventory, and the suite probes each entry through the
/// real switch (an entry that answers unknownOp fails the suite, so the
/// list cannot drift from the dispatch).
export const LAYOUT_OPS: ReadonlyArray<string> = [
  'workspaces.create',
  'workspaces.rename',
  'workspaces.delete',
  'workspaces.close',
  'workspaces.open',
  'workspaces.pin',
  'workspaces.unpin',
  'workspaces.color',
  'workspaces.move',
  'workspaces.reorder',
  'groups.create',
  'groups.rename',
  'groups.delete',
  'groups.unpack',
  'groups.pin',
  'groups.unpin',
  'groups.color',
  'groups.move',
  'groups.collapse',
  'groups.expand',
  'tabs.create',
  'tabs.open',
  'tabs.close',
  'tabs.rename',
  'tabs.pin',
  'tabs.unpin',
  'tabs.color',
  'tabs.reorder',
  'tabs.move',
  'tabs.separate',
  'panes.split',
  'panes.close',
  'panes.focus',
  'panes.zoom',
  'panes.resize',
]

/// The op catalog (#125). `params` is the request's params object verbatim;
/// the answer is either the new state + result doc (echoing ids) or the
/// enumerated error. genId is injected so tests are deterministic.
export function applyGatewayOp(
  state: WorkspaceState,
  op: string,
  rawParams: unknown,
  genId: () => string,
): OpOutcome {
  const params = (rawParams ?? {}) as Record<string, unknown>

  const needWorkspace = (ref: unknown): { ok: true; ws: Workspace } | { ok: false; error: GatewayError } => {
    const ws = resolveWorkspace(state, ref)
    if (ws == null) {
      return {
        ok: false,
        error: notFound('workspace', String(ref ?? ''), 'list what exists: umux workspaces list --json'),
      }
    }
    return { ok: true, ws }
  }

  switch (op) {
    // --- workspaces ---------------------------------------------------------
    case 'workspaces.create': {
      const name = str(params, 'name')
      if (name == null) return { ok: false, error: err('badParams', 'params need a non-empty "name"', []) }
      const next = createWorkspace(state, name, genId)
      const created = next.workspaces[next.workspaces.length - 1]
      const tab = created.tabs?.[0]
      return {
        ok: true,
        state: next,
        result: {
          created: {
            id: created.id,
            name: created.name,
            tabId: tab?.id ?? null,
            panelId: tab != null ? leafIds(tab.layout)[0] : null,
          },
        },
      }
    }
    case 'workspaces.rename': {
      const ref = params['workspace']
      const found = needWorkspace(ref)
      if (!found.ok) return found
      const to = str(params, 'to')
      if (to == null || to.trim() === '') {
        return { ok: false, error: err('badParams', 'params need a non-empty "to" (the new name)', []) }
      }
      return {
        ok: true,
        state: renameWorkspace(state, found.ws.id, to),
        result: { renamed: { id: found.ws.id, from: found.ws.name, to } },
      }
    }
    case 'workspaces.delete': {
      const ref = params['workspace']
      const found = needWorkspace(ref)
      if (!found.ok) return found
      return {
        ok: true,
        state: deleteWorkspace(state, found.ws.id),
        result: { deleted: { id: found.ws.id, name: found.ws.name } },
      }
    }
    case 'workspaces.close': {
      const refs = listParam(params, 'workspaces')
      if (refs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "workspaces" (one id/name or a list)', []) }
      }
      const closed: string[] = []
      let next = state
      for (const ref of refs) {
        const ws = resolveWorkspace(next, ref)
        if (ws == null) {
          return { ok: false, error: notFound('workspace', ref, 'list what exists: umux workspaces list --json') }
        }
        next = closeWorkspace(next, ws.id)
        closed.push(ws.id)
      }
      return { ok: true, state: next, result: { closed } }
    }
    case 'workspaces.open': {
      const ref = params['workspace']
      const found = needWorkspace(ref)
      if (!found.ok) return found
      return {
        ok: true,
        state: openWorkspace(state, found.ws.id),
        result: { opened: { id: found.ws.id, name: found.ws.name } },
      }
    }
    case 'workspaces.pin':
    case 'workspaces.unpin': {
      const pinned = op === 'workspaces.pin'
      const refs = listParam(params, 'workspaces')
      if (refs.length === 0) {
        return { ok: false, error: err('badParams', `params need "workspaces" (one id/name or a list)`, []) }
      }
      const ids: string[] = []
      for (const ref of refs) {
        const found = needWorkspace(ref)
        if (!found.ok) return found
        ids.push(found.ws.id)
      }
      return {
        ok: true,
        state: setNodesPinned(state, ids, pinned),
        result: pinned ? { pinned: ids } : { unpinned: ids },
      }
    }
    case 'workspaces.color': {
      const refs = listParam(params, 'workspaces')
      if (refs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "workspaces" (one id/name or a list)', []) }
      }
      const color = parseColor(params['color'])
      if (!color.ok) return color
      const ids: string[] = []
      for (const ref of refs) {
        const found = needWorkspace(ref)
        if (!found.ok) return found
        ids.push(found.ws.id)
      }
      let next = state
      for (const id of ids) next = setWorkspaceColor(next, id, color.color)
      return { ok: true, state: next, result: { colored: ids, color: color.color } }
    }
    case 'workspaces.move': {
      const refs = listParam(params, 'workspaces')
      if (refs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "workspaces" (one id/name or a list)', []) }
      }
      const newGroupName = str(params, 'newGroup')
      const groupRef = str(params, 'group')
      if (newGroupName == null && groupRef == null) {
        return {
          ok: false,
          error: err('badParams', 'params need "group" (an existing group) or "newGroup" (a fresh name)', []),
        }
      }
      const ids: string[] = []
      for (const ref of refs) {
        const found = needWorkspace(ref)
        if (!found.ok) return found
        ids.push(found.ws.id)
      }
      if (newGroupName != null) {
        const first = ids[0]
        let next = moveToNewGroup(state, first, newGroupName, genId)
        const group = next.groups[next.groups.length - 1]
        for (const id of ids.slice(1)) next = moveNode(next, id, { parentId: group.id })
        return { ok: true, state: next, result: { moved: ids, newGroup: { id: group.id, name: group.name } } }
      }
      const group = resolveGroup(state, groupRef)
      if (group == null) {
        return { ok: false, error: notFound('group', String(groupRef), 'groups are listed in umux workspaces list --json') }
      }
      const next = moveNodes(state, ids, { parentId: group.id })
      return { ok: true, state: next, result: { moved: ids, group: group.id } }
    }
    case 'workspaces.reorder': {
      const ref = params['workspace']
      const found = needWorkspace(ref)
      if (!found.ok) return found
      const beforeRef = str(params, 'before')
      let beforeId: string | null = null
      if (beforeRef != null) {
        const target = resolveWorkspace(state, beforeRef) ?? resolveGroup(state, beforeRef)
        if (target == null) {
          return { ok: false, error: notFound('node', beforeRef, '"before" names the sibling to insert in front of') }
        }
        beforeId = target.id
      }
      const parentId = found.ws.groupId ?? null
      const next = moveNode(state, found.ws.id, beforeId != null ? { parentId, beforeId } : { parentId })
      const index = next.order.indexOf(found.ws.id)
      return { ok: true, state: next, result: { reordered: { id: found.ws.id, parentId, index } } }
    }
    // --- groups -------------------------------------------------------------
    case 'groups.create': {
      const name = str(params, 'name')
      if (name == null || name.trim() === '') {
        return { ok: false, error: err('badParams', 'params need a non-empty "name"', []) }
      }
      const parentRef = str(params, 'parent')
      if (parentRef == null) {
        const next = createGroup(state, name.trim(), genId)
        const created = next.groups[next.groups.length - 1]
        return { ok: true, state: next, result: { created: { id: created.id, name: created.name } } }
      }
      const parent = resolveGroup(state, parentRef)
      if (parent == null) {
        return { ok: false, error: notFound('group', parentRef, 'groups are listed in umux workspaces list --json') }
      }
      const created = createGroup(state, name.trim(), genId)
      const group = created.groups[created.groups.length - 1]
      const next = moveNode(created, group.id, { parentId: parent.id })
      return { ok: true, state: next, result: { created: { id: group.id, name: group.name, parentId: parent.id } } }
    }
    case 'groups.rename': {
      const ref = params['group']
      const group = resolveGroup(state, ref)
      if (group == null) {
        return { ok: false, error: notFound('group', String(ref ?? ''), 'groups are listed in umux workspaces list --json') }
      }
      const to = str(params, 'to')
      if (to == null || to.trim() === '') {
        return { ok: false, error: err('badParams', 'params need a non-empty "to" (the new name)', []) }
      }
      return {
        ok: true,
        state: renameGroup(state, group.id, to),
        result: { renamed: { id: group.id, from: group.name, to } },
      }
    }
    case 'groups.delete': {
      const ref = params['group']
      const group = resolveGroup(state, ref)
      if (group == null) {
        return { ok: false, error: notFound('group', String(ref ?? ''), 'groups are listed in umux workspaces list --json') }
      }
      const subtree = params['subtree'] === true
      if (!isGroupEmpty(state, group.id) && !subtree) {
        return {
          ok: false,
          error: err(
            'groupNotEmpty',
            `group "${group.name}" is not empty — delete it with everything inside using --subtree`,
            ['review what dies with it: umux workspaces list --json'],
          ),
        }
      }
      if (!subtree) {
        return {
          ok: true,
          state: deleteGroup(state, group.id),
          result: { deleted: { groups: [group.id], workspaces: [] } },
        }
      }
      const doomed = groupSubtreeIds(state, group.id)
      const doomedWorkspaces = state.workspaces
        .filter((w) => w.groupId != null && doomed.includes(w.groupId))
        .map((w) => w.id)
      return {
        ok: true,
        state: deleteGroupSubtree(state, group.id),
        result: { deleted: { groups: doomed, workspaces: doomedWorkspaces } },
      }
    }
    case 'groups.unpack': {
      const ref = params['group']
      const group = resolveGroup(state, ref)
      if (group == null) {
        return { ok: false, error: notFound('group', String(ref ?? ''), 'groups are listed in umux workspaces list --json') }
      }
      return { ok: true, state: unpackGroup(state, group.id), result: { unpacked: { id: group.id } } }
    }
    case 'groups.pin':
    case 'groups.unpin': {
      const pinned = op === 'groups.pin'
      const refs = listParam(params, 'groups')
      if (refs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "groups" (one id/name or a list)', []) }
      }
      const ids: string[] = []
      for (const ref of refs) {
        const group = resolveGroup(state, ref)
        if (group == null) {
          return { ok: false, error: notFound('group', ref, 'groups are listed in umux workspaces list --json') }
        }
        ids.push(group.id)
      }
      return {
        ok: true,
        state: setNodesPinned(state, ids, pinned),
        result: pinned ? { pinned: ids } : { unpinned: ids },
      }
    }
    case 'groups.color': {
      const refs = listParam(params, 'groups')
      if (refs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "groups" (one id/name or a list)', []) }
      }
      const color = parseColor(params['color'])
      if (!color.ok) return color
      const ids: string[] = []
      for (const ref of refs) {
        const group = resolveGroup(state, ref)
        if (group == null) {
          return { ok: false, error: notFound('group', ref, 'groups are listed in umux workspaces list --json') }
        }
        ids.push(group.id)
      }
      let next = state
      for (const id of ids) next = setGroupColor(next, id, color.color)
      return { ok: true, state: next, result: { colored: ids, color: color.color } }
    }
    case 'groups.move': {
      const ref = params['group']
      const group = resolveGroup(state, ref)
      if (group == null) {
        return { ok: false, error: notFound('group', String(ref ?? ''), 'groups are listed in umux workspaces list --json') }
      }
      const parentRef = str(params, 'parent')
      const beforeRef = str(params, 'before')
      let parentId: string | null = group.parentId ?? null
      if (parentRef != null) {
        const parent = resolveGroup(state, parentRef)
        if (parent == null) {
          return { ok: false, error: notFound('group', parentRef, '"parent" names the group to nest into') }
        }
        parentId = parent.id
      }
      let beforeId: string | null = null
      if (beforeRef != null) {
        const target = resolveGroup(state, beforeRef) ?? resolveWorkspace(state, beforeRef)
        if (target == null) {
          return { ok: false, error: notFound('node', beforeRef, '"before" names the sibling to insert in front of') }
        }
        beforeId = target.id
      }
      const next = moveNode(state, group.id, beforeId != null ? { parentId, beforeId } : { parentId })
      const index = next.order.indexOf(group.id)
      return { ok: true, state: next, result: { moved: { id: group.id, parentId, index } } }
    }
    case 'groups.collapse':
    case 'groups.expand': {
      const ref = params['group']
      const group = resolveGroup(state, ref)
      if (group == null) {
        return { ok: false, error: notFound('group', String(ref ?? ''), 'groups are listed in umux workspaces list --json') }
      }
      // toggleCollapse flips IN PLACE; expand/collapse say the direction, so
      // expand on a collapsed group and collapse on an open one toggle, and
      // the matching direction on an already-there state is a no-op — the
      // result states what the group IS now.
      const collapsed = group.collapsed === true
      if ((op === 'groups.collapse') === collapsed) {
        return { ok: true, state, result: { [collapsed ? 'collapsed' : 'expanded']: group.id } }
      }
      const next = toggleCollapse(state, group.id)
      return {
        ok: true,
        state: next,
        result: { [op === 'groups.collapse' ? 'collapsed' : 'expanded']: group.id },
      }
    }
    // --- tabs ---------------------------------------------------------------
    case 'tabs.create': {
      const ref = params['workspace']
      const found = needWorkspace(ref)
      if (!found.ok) return found
      const shell = str(params, 'shell') ?? undefined
      const name = str(params, 'name') ?? undefined
      const opened = addTab(state, found.ws.id, genId, shell)
      const next = name != null ? renameTab(opened, found.ws.id, opened.activeTabId[found.ws.id], name) : opened
      const tabId = next.activeTabId[found.ws.id]
      const ws = next.workspaces.find((w) => w.id === found.ws.id) as Workspace
      const tab = ws.tabs?.find((t) => t.id === tabId) as Tab
      return {
        ok: true,
        state: next,
        result: {
          created: {
            id: tab.id,
            name: tab.name ?? null,
            workspaceId: found.ws.id,
            panelId: leafIds(tab.layout)[0],
          },
        },
      }
    }
    case 'tabs.open': {
      const tabId = str(params, 'tab')
      const found = findTab(state, tabId)
      if (found == null) {
        return { ok: false, error: notFound('tab', String(tabId ?? ''), 'tab ids come from umux workspaces list --json or a create/reopen echo') }
      }
      const opened = openWorkspace(state, found.ws.id)
      const next = switchWorkspace(opened, found.ws.id)
      const done = switchTab(next, found.ws.id, found.tab.id)
      return {
        ok: true,
        state: done,
        result: { opened: { tab: found.tab.id, workspace: found.ws.id } },
      }
    }
    case 'tabs.close': {
      const tabRefs = listParam(params, 'tabs')
      if (tabRefs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "tabs" (one id or a list)', []) }
      }
      // Group by owning workspace first (closeTabs is workspace-scoped).
      const byWs = new Map<string, string[]>()
      for (const id of tabRefs) {
        const found = findTab(state, id)
        if (found == null) {
          return { ok: false, error: notFound('tab', id, 'tab ids come from umux workspaces list --json or a create echo') }
        }
        const list = byWs.get(found.ws.id) ?? []
        list.push(found.tab.id)
        byWs.set(found.ws.id, list)
      }
      let next = state
      const closed: string[] = []
      for (const [wsId, tabs] of byWs) {
        next = closeTabs(next, wsId, tabs)
        closed.push(...tabs)
      }
      return { ok: true, state: next, result: { closed } }
    }
    case 'tabs.rename': {
      const tabId = str(params, 'tab')
      const found = findTab(state, tabId)
      if (found == null) {
        return { ok: false, error: notFound('tab', String(tabId ?? ''), 'tab ids come from umux workspaces list --json or a create echo') }
      }
      const to = str(params, 'to')
      if (to == null) {
        return { ok: false, error: err('badParams', 'params need "to" (the new name; empty clears it)', []) }
      }
      return {
        ok: true,
        state: renameTab(state, found.ws.id, found.tab.id, to),
        result: { renamed: { id: found.tab.id, to: to.trim() } },
      }
    }
    case 'tabs.pin':
    case 'tabs.unpin': {
      const pinned = op === 'tabs.pin'
      const tabRefs = listParam(params, 'tabs')
      if (tabRefs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "tabs" (one id or a list)', []) }
      }
      let next = state
      const ids: string[] = []
      for (const id of tabRefs) {
        const found = findTab(next, id)
        if (found == null) {
          return { ok: false, error: notFound('tab', id, 'tab ids come from umux workspaces list --json or a create echo') }
        }
        next = setTabPinned(next, found.ws.id, found.tab.id, pinned)
        ids.push(found.tab.id)
      }
      return { ok: true, state: next, result: pinned ? { pinned: ids } : { unpinned: ids } }
    }
    case 'tabs.color': {
      const tabRefs = listParam(params, 'tabs')
      if (tabRefs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "tabs" (one id or a list)', []) }
      }
      const color = parseColor(params['color'])
      if (!color.ok) return color
      let next = state
      const ids: string[] = []
      for (const id of tabRefs) {
        const found = findTab(next, id)
        if (found == null) {
          return { ok: false, error: notFound('tab', id, 'tab ids come from umux workspaces list --json or a create echo') }
        }
        next = setTabColor(next, found.ws.id, found.tab.id, color.color)
        ids.push(found.tab.id)
      }
      return { ok: true, state: next, result: { colored: ids, color: color.color } }
    }
    case 'tabs.reorder': {
      const tabId = str(params, 'tab')
      const found = findTab(state, tabId)
      if (found == null) {
        return { ok: false, error: notFound('tab', String(tabId ?? ''), 'tab ids come from umux workspaces list --json or a create echo') }
      }
      const index = params['index']
      if (typeof index !== 'number' || !Number.isInteger(index) || index < 0) {
        return { ok: false, error: err('badParams', 'params need integer "index" (the 0-based position in the tab bar)', []) }
      }
      const next = moveTab(state, found.ws.id, found.tab.id, index)
      const ws = next.workspaces.find((w) => w.id === found.ws.id) as Workspace
      const newIndex = (ws.tabs ?? []).findIndex((t) => t.id === found.tab.id)
      return { ok: true, state: next, result: { reordered: { id: found.tab.id, index: newIndex } } }
    }
    case 'tabs.move': {
      const tabRefs = listParam(params, 'tabs')
      if (tabRefs.length === 0) {
        return { ok: false, error: err('badParams', 'params need "tabs" (one id or a list)', []) }
      }
      const targetRef = params['workspace']
      const target = resolveWorkspace(state, targetRef)
      if (target == null) {
        return { ok: false, error: notFound('workspace', String(targetRef ?? ''), 'list what exists: umux workspaces list --json') }
      }
      // All tabs must share one source workspace (the UI move is one drag in
      // one bar; a cross-workspace batch has no UI counterpart to match).
      let fromId: string | null = null
      const ids: string[] = []
      for (const id of tabRefs) {
        const found = findTab(state, id)
        if (found == null) {
          return { ok: false, error: notFound('tab', id, 'tab ids come from umux workspaces list --json or a create echo') }
        }
        if (fromId == null) fromId = found.ws.id
        if (found.ws.id !== fromId) {
          return {
            ok: false,
            error: err('badParams', 'all tabs in one move must come from the same workspace', ['move one workspace at a time']),
          }
        }
        ids.push(found.tab.id)
      }
      if (fromId === target.id) {
        return { ok: false, error: err('badParams', 'the tabs already live in that workspace', []) }
      }
      return {
        ok: true,
        state: moveTabsToWorkspace(state, fromId as string, ids, target.id),
        result: { moved: ids, workspace: target.id },
      }
    }
    case 'tabs.separate': {
      const tabId = str(params, 'tab')
      const found = findTab(state, tabId)
      if (found == null) {
        return { ok: false, error: notFound('tab', String(tabId ?? ''), 'tab ids come from umux workspaces list --json or a create echo') }
      }
      const before = (found.ws.tabs ?? []).length
      const next = separateTab(state, found.ws.id, found.tab.id, genId)
      const ws = next.workspaces.find((w) => w.id === found.ws.id) as Workspace
      const spawned = (ws.tabs ?? []).slice(before).map((t) => t.id)
      return { ok: true, state: next, result: { separated: { tab: found.tab.id, created: spawned } } }
    }
    // --- panes --------------------------------------------------------------
    case 'panes.split': {
      const ref = str(params, 'workspace')
      let ws: Workspace | null
      if (ref != null) {
        const found = needWorkspace(ref)
        if (!found.ok) return found
        ws = found.ws
      } else {
        ws = state.workspaces.find((w) => w.id === state.activeId) ?? state.workspaces[0] ?? null
      }
      if (ws == null) {
        return { ok: false, error: err('workspaceNotFound', 'no workspace to split — create one first', ['umux workspaces create <name>']) }
      }
      const orientation = params['orientation'] === 'vertical' ? 'vertical' : params['orientation'] === 'horizontal' ? 'horizontal' : null
      if (orientation == null) {
        return { ok: false, error: err('badParams', 'params need "orientation": "horizontal" or "vertical"', []) }
      }
      const next = splitPanel(state, ws.id, orientation as Orientation, genId)
      const tab = activeTabOf(next, ws.id)
      const focused = tab != null ? next.activePanelId[ws.id] : null
      return {
        ok: true,
        state: next,
        result: { split: { workspace: ws.id, tab: tab?.id ?? null, orientation, createdPanelId: focused } },
      }
    }
    case 'panes.close': {
      const paneId = str(params, 'pane')
      const location = panelLocationOf(state, paneId ?? '')
      if (paneId == null || location == null) {
        return { ok: false, error: notFound('pane', String(paneId ?? ''), 'pane ids are the leaf ids in umux workspaces list --json') }
      }
      const next = closePanel(state, location.workspaceId, paneId)
      return { ok: true, state: next, result: { closed: { pane: paneId } } }
    }
    case 'panes.focus': {
      const paneId = str(params, 'pane')
      const location = panelLocationOf(state, paneId ?? '')
      if (paneId == null || location == null) {
        return { ok: false, error: notFound('pane', String(paneId ?? ''), 'pane ids are the leaf ids in umux workspaces list --json') }
      }
      const next = focusPanel(state, location.workspaceId, paneId)
      return { ok: true, state: next, result: { focused: { pane: paneId, workspace: location.workspaceId } } }
    }
    case 'panes.zoom': {
      const paneId = str(params, 'pane')
      const location = paneId != null ? panelLocationOf(state, paneId) : null
      // An explicit pane must exist; without one the op mirrors the zoom
      // BUTTON (the focused panel of the active workspace).
      if (paneId != null && location == null) {
        return { ok: false, error: notFound('pane', paneId, 'pane ids are the leaf ids in umux workspaces list --json') }
      }
      const wsId = location?.workspaceId ?? state.activeId
      if (wsId == null || !state.workspaces.some((w) => w.id === wsId)) {
        return { ok: false, error: err('workspaceNotFound', 'no workspace to zoom in', []) }
      }
      const next = toggleZoom(state, wsId, paneId ?? undefined)
      const tab = activeTabOf(next, wsId)
      const zoomed = tab != null ? next.zoomedPanelId[tab.id] ?? null : null
      return { ok: true, state: next, result: { zoomed: zoomed } }
    }
    case 'panes.resize': {
      const paneId = str(params, 'pane')
      const location = panelLocationOf(state, paneId ?? '')
      if (paneId == null || location == null) {
        return { ok: false, error: notFound('pane', String(paneId ?? ''), 'pane ids are the leaf ids in umux workspaces list --json') }
      }
      const ratio = params['ratio']
      if (typeof ratio !== 'number' || !Number.isFinite(ratio) || ratio <= 0 || ratio >= 1) {
        return { ok: false, error: err('badParams', 'params need "ratio": a number between 0 and 1 (the first side\'s share)', []) }
      }
      const splitId = splitIdOfLeaf(state, location.workspaceId, paneId)
      if (splitId == null) {
        return {
          ok: false,
          error: err('badParams', `pane "${paneId}" is not in a split — nothing to resize`, ['split it first: umux panes split']),
        }
      }
      const next = resizePanel(state, location.workspaceId, splitId, ratio, SYNTHETIC_EXTENT)
      return { ok: true, state: next, result: { resized: { pane: paneId, split: splitId, ratio } } }
    }
    default:
      return {
        ok: false,
        error: err('unknownOp', `unknown op "${op}"`, ['see: umux agent-context']),
      }
  }
}
