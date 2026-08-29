// workspaces — pure state module for the workspace collection (Phase 6 / #7).
//
// Deep module: a tiny interface (create / rename / switch / list) over the
// in-memory workspace model. No I/O — persistence lives in the Rust
// WorkspaceStore; this module is fed by it on startup and triggers a save on
// every mutation. Trivially unit-testable.
//
// v0.2 Phase 1 / #25: the panel layout is a split TREE persisted INSIDE each
// Workspace. Leaf ids are panel ids — the runtime layouts/panelIds records of
// v0.1 are gone; panel identity derives from the tree.
//
// #37 rework (Adam's correction): a workspace holds TABS — separate terminal
// windows — and each tab holds its own split tree. The tree that used to hang
// directly off the workspace now hangs off each tab; bootState migrates old
// configs, and the active-tab id is runtime-only (like activePanelId).

import {
  createTree,
  splitLeaf,
  closeLeaf,
  setRatio,
  leafIds,
  type LayoutNode,
  type Orientation,
  type Container,
} from './PaneLayout'
import type { AgentStatus } from './agentStatus'

// Forward-compat slot from Phase 8: per-panel config (cwd / SSH target)
// associated with the layout tree's leaf ids. Absent = local panel in the
// default cwd. Kept byte-identical to the Rust `Panel` in workspace_store.rs.
export type Panel = {
  id: string
  workingDirectory?: string
  sshTarget?: string
}

// A tab is ONE terminal window inside a workspace (#37 rework, Adam's
// correction): workspaces are projects, tabs are the separate terminals the
// project is open in, and a tab's area can be split into panes (the layout
// tree that used to hang directly off the workspace). Leaf ids are panel
// ids, exactly as before — only the nesting moved. `name` and `pinned`
// (Adam's follow-up) are persisted definition fields: a named tab shows its
// name instead of the positional "Tab N", and pinned tabs sit at the top of
// the bar as a group. Both optional — absent = unnamed/unpinned, and the
// persisted payload must not gain the keys just by passing through here.
export type Tab = {
  id: string
  layout: LayoutNode
  name?: string
  pinned?: boolean
}

// A group node of the sidebar tree (#48): a named container workspaces can be
// filed into (#49). `collapsed` (#50)/`pinned` (#52) live on the model,
// optional, and the persisted payload must not gain the keys just by passing
// through here — mirrored as Option fields in the Rust store. `parentId`
// (#51) nests groups inside groups without a depth limit; absent = the group
// sits at top level, with the same key hygiene as the workspace's `groupId`.
export type Group = {
  id: string
  name: string
  collapsed?: boolean
  pinned?: boolean
  parentId?: string
}

export type Workspace = {
  id: string
  name: string
  // Parent group (#49). Absent = the workspace sits at top level. Optional so
  // pre-groups configs type-check on load and ungrouped workspaces persist
  // WITHOUT the key — mirrored as an Option field in the Rust store.
  groupId?: string
  // Pinned workspaces sit at the TOP of the list as a group (#37). Optional
  // like `panels`: absent = unpinned, and the persisted payload must not gain
  // the key just by passing through here — mirrored as an Option field in the
  // Rust WorkspaceStore so saves round-trip it only when set.
  pinned?: boolean
  // Optional at runtime: hand-edited and pre-v0.2 configs carry none, and
  // the persisted payload must not gain the key just by passing through
  // here — guards at the use sites keep old saves byte-identical.
  panels?: Panel[]
  // The workspace's terminal tabs (#37 rework). Optional because pre-rework
  // configs (v0.2 and the first #37 cut) carry none — bootState migrates a
  // legacy top-level `layout` into a single tab and strips the old key, so
  // new saves write `tabs` only.
  tabs?: Tab[]
  // LEGACY input field (never written): the pre-rework split tree. Kept on
  // the type so old configs type-check on load; bootState consumes it and
  // removes it. Rust keeps the field for the same round-trip reason.
  layout?: LayoutNode
}

export type WorkspaceState = {
  workspaces: Workspace[]
  // The sidebar tree (#48). `groups` holds the group nodes; `order` is ONE
  // interleaved display order of EVERY node id (groups + workspaces) — a
  // node's siblings are the entries sharing its parent, ranked by their
  // relative position here. Both are persisted via the Rust store (which
  // serializes them since #48); bootState normalizes them on load.
  groups: Group[]
  order: string[]
  activeId: string | null
  // Runtime-only (NOT persisted, like activeId): which workspaces currently
  // have a live, mounted panel — i.e. an open shell. A workspace not in this
  // list is "closed": its definition stays but its panels (and shells) are gone.
  openIds: string[]
  // Runtime-only (NOT persisted): the active tab id per workspace (#37
  // rework). Absent entry means "the first tab is active".
  activeTabId: Record<string, string>
  // Runtime-only (NOT persisted): the focused panel id per workspace (Phase 11
  // / #12, story 34). Absent entry means "no explicit focus" — the first
  // panel is treated as active (see activePanelOf). The renderer draws a ring
  // on the active panel so it is always obvious where keystrokes will go.
  activePanelId: Record<string, string>
  // Runtime-only (NOT persisted): the zoomed panel id per TAB id (#40 / story
  // 48). Absent entry = not zoomed. Zoom is a pure VIEW state — the split
  // tree and the persisted config are never touched, and a restarted app
  // boots with this record empty (every tab renders its normal layout).
  zoomedPanelId: Record<string, string>
}

export const emptyState: WorkspaceState = {
  workspaces: [],
  groups: [],
  order: [],
  activeId: null,
  openIds: [],
  activeTabId: {},
  activePanelId: {},
  zoomedPanelId: {},
}

// Exported for call sites that must pass later optional parameters past it
// (bootState's tree arguments, #48) — the id generator itself stays injectable.
export const defaultGenId = (): string => crypto.randomUUID()

/// Return a shallow copy of `map` without `key`. Pure helper for runtime-only
/// records keyed by workspace id.
function removeKey<V>(map: Record<string, V>, key: string): Record<string, V> {
  const { [key]: _removed, ...rest } = map
  return rest
}

/// Seed app state from the persisted config (v0.2 / #25, AC4; reworked for
/// tabs in #37; tree-shaped since #48). The migration ladder per workspace:
///   - already has `tabs` (a rework-era config): kept untouched;
///   - only the legacy top-level `layout` (v0.2 / first #37 cut): it becomes
///     the workspace's single tab and the legacy key is STRIPPED, so the next
///     save writes the new shape and never both;
///   - neither (pre-v0.2): one fresh tab with a single-leaf tree.
/// The tree side (#48): `loadedGroups`/`loadedOrder` come from the config
/// when present; a PRE-GROUPS (flat) config carries neither and loads with
/// zero groups and every workspace at top level. `order` is normalized —
/// unknown/duplicate ids drop out, and any node the config forgot to rank is
/// appended (workspaces first, then groups) so nothing silently disappears
/// from a hand-edited file. A `groupId` pointing at a group the config does
/// not define is stale and is stripped (the workspace loads at top level).
/// The same guard covers NESTED groups (#51): a `parentId` pointing at a
/// group the config does not define is stripped (the group loads at top
/// level), and a hand-edited CYCLE (g1 in g2, g2 in g1) is broken — every
/// group on or behind a cycle loads at top level, so the renderer can walk
/// parents without a guard. Every loaded workspace starts open with its
/// first tab and first panel focused, and the first workspace is active.
export function bootState(
  loaded: Workspace[],
  genId: () => string = defaultGenId,
  loadedGroups: Group[] = [],
  loadedOrder: string[] = [],
): WorkspaceState {
  const knownGroups = new Set(loadedGroups.map((g) => g.id))
  const migrate = (w: Workspace): Workspace => {
    let base: Workspace
    if (w.tabs != null && w.tabs.length > 0) {
      // Drop a legacy `layout` key if a hand-merge left both behind, and
      // FILL missing tab names with stable positional defaults: an unnamed
      // tab displayed as "Tab N" by POSITION reshuffles its number whenever
      // a pin/reorder moves it (HITL: "I pin tab 2 and tab 1 gets pinned"),
      // so every tab gets a name that sticks at boot.
      const { layout: _legacy, ...rest } = w
      base = {
        ...rest,
        tabs: w.tabs.map((t, i) =>
          t.name == null ? { ...t, name: `Tab ${i + 1}` } : t,
        ),
      }
    } else {
      const tree = w.layout ?? createTree(genId())
      const { layout: _legacy, ...rest } = w
      base = { ...rest, tabs: [{ id: genId(), layout: tree, name: 'Tab 1' }] }
    }
    if (base.groupId != null && !knownGroups.has(base.groupId)) {
      const { groupId: _stale, ...rest } = base
      return rest
    }
    return base
  }
  const workspaces = loaded.map(migrate)
  // Group parent normalization (#51): a stale `parentId` (unknown group)
  // drops to top level, and a parent CYCLE is broken — a group is "rooted"
  // when its parent chain terminates at top level; anything still unrooted
  // after enough passes sits on or behind a cycle and loads at top level.
  const rawParents = new Map<string, string | null>(
    loadedGroups.map((g) => [
      g.id,
      g.parentId != null && knownGroups.has(g.parentId) ? g.parentId : null,
    ]),
  )
  const rooted = new Set<string>()
  for (let pass = 0; pass <= loadedGroups.length; pass++) {
    for (const g of loadedGroups) {
      if (rooted.has(g.id)) continue
      const parent = rawParents.get(g.id) ?? null
      if (parent == null || rooted.has(parent)) rooted.add(g.id)
    }
  }
  const groups = loadedGroups.map((g) => {
    if (!rooted.has(g.id)) return omitGroupParent(g)
    // Rooted on a VALID parent: keep it; rooted at top level (or a stale
    // reference): drop the key so the payload stays clean.
    const parent = rawParents.get(g.id) ?? null
    return parent != null ? { ...g, parentId: parent } : omitGroupParent(g)
  })
  const known = new Set<string>([...workspaces.map((w) => w.id), ...groups.map((g) => g.id)])
  const seen = new Set<string>()
  const order: string[] = []
  for (const id of loadedOrder) {
    if (known.has(id) && !seen.has(id)) {
      order.push(id)
      seen.add(id)
    }
  }
  for (const w of workspaces) {
    if (!seen.has(w.id)) {
      order.push(w.id)
      seen.add(w.id)
    }
  }
  for (const g of groups) {
    if (!seen.has(g.id)) {
      order.push(g.id)
      seen.add(g.id)
    }
  }
  return {
    workspaces,
    groups,
    order,
    activeId: workspaces[0]?.id ?? null,
    openIds: workspaces.map((w) => w.id),
    activeTabId: Object.fromEntries(
      workspaces.map((w) => [w.id, w.tabs?.[0]?.id ?? '']),
    ),
    activePanelId: Object.fromEntries(
      workspaces.map((w) => [w.id, w.tabs ? leafIds(w.tabs[0].layout)[0] : '']),
    ),
    // Zoom never persists (#40): a restarted app always comes up unzoomed.
    zoomedPanelId: {},
  }
}

/// The default display name for a NEW tab in a workspace: "Tab N" with N
/// past any name already taken, so numbers never collide or reshuffle
/// (tabs keep the number they were created with — browser-style).
function nextTabName(tabs: Tab[]): string {
  let n = tabs.length + 1
  const taken = new Set(tabs.map((t) => t.name))
  while (taken.has(`Tab ${n}`)) n += 1
  return `Tab ${n}`
}

export function createWorkspace(
  state: WorkspaceState,
  name: string,
  genId: () => string = defaultGenId,
): WorkspaceState {
  // genId order (stable, tests rely on it): workspace id, first tab id, then
  // the tab's seed-leaf id. The tab is born NAMED ("Tab 1") — see
  // nextTabName for why numbers must stick, not recompute by position.
  const id = genId()
  const tab: Tab = { id: genId(), layout: createTree(genId()), name: 'Tab 1' }
  const workspace: Workspace = { id, name, panels: [], tabs: [tab] }
  return {
    workspaces: [...state.workspaces, workspace],
    groups: state.groups,
    // New workspaces ALWAYS attach at TOP level (#49): last among the
    // top-level siblings in the shared display order, never inside a group.
    order: [...state.order, id],
    activeId: id,
    openIds: [...state.openIds, id],
    activeTabId: { ...state.activeTabId, [id]: tab.id },
    activePanelId: {
      ...state.activePanelId,
      [id]: leafIds(tab.layout)[0],
    },
    // Runtime records of the existing workspaces ride along untouched.
    zoomedPanelId: state.zoomedPanelId,
  }
}

// --- Sidebar tree (#48): groups + the shared interleaved order --------------
//
// All mutations are pure state operations (same discipline as the rest of
// the module): they return a new WorkspaceState and never touch I/O. Groups
// and workspaces share ONE display order (`state.order`); a node's siblings
// are the entries sharing its parent, ranked by relative position there.

/// Append an empty group at the END of the top level (#48, "New group").
/// The genId is consumed for the group's id only.
export function createGroup(
  state: WorkspaceState,
  name: string,
  genId: () => string = defaultGenId,
): WorkspaceState {
  const group: Group = { id: genId(), name }
  return {
    ...state,
    groups: [...state.groups, group],
    order: [...state.order, group.id],
  }
}

/// Rename a group (#48). No-op for an unknown id; an empty name is ignored
/// (the same rule renameWorkspace applies to workspaces).
export function renameGroup(
  state: WorkspaceState,
  id: string,
  name: string,
): WorkspaceState {
  if (!state.groups.some((g) => g.id === id)) return state
  if (name.trim() === '') return state
  return {
    ...state,
    groups: state.groups.map((g) => (g.id === id ? { ...g, name } : g)),
  }
}

/// True when group `id` is bare — nothing inside it but itself, at any depth
/// (#48; subtree-aware since #51): no descendant workspace AND no subgroup.
/// Only a bare group may delete without a confirmation; anything else goes
/// through deleteGroupSubtree behind the shared dialog.
export function isGroupEmpty(state: WorkspaceState, id: string): boolean {
  if (state.groups.some((g) => g.parentId === id)) return false
  return !state.workspaces.some((w) => w.groupId === id)
}

/// Delete an EMPTY group (#48): the node leaves `groups` and `order`; its
/// (nonexistent) children are untouched. A group holding anything — or an
/// unknown id — is a no-op; the destructive delete-with-children is
/// deleteGroupSubtree (#51), which the UI puts behind a confirmation.
export function deleteGroup(state: WorkspaceState, id: string): WorkspaceState {
  if (!state.groups.some((g) => g.id === id)) return state
  if (!isGroupEmpty(state, id)) return state
  return {
    ...state,
    groups: state.groups.filter((g) => g.id !== id),
    order: state.order.filter((nodeId) => nodeId !== id),
  }
}

/// Where a moved node should land: the parent group (`null` = top level)
/// and optionally the sibling id to insert BEFORE (reorder). Omitting
/// `beforeId` appends at the end of the target's children (#49: dropping
/// onto a group files the workspace at the end of that group).
export type MoveTarget = { parentId: string | null; beforeId?: string }

/// The parent group id of a node in `order`, or `null` = top level. Groups
/// nest since #51: a group's parent is its own `parentId`, a workspace's is
/// its `groupId`.
function parentOf(state: WorkspaceState, nodeId: string): string | null {
  const group = state.groups.find((g) => g.id === nodeId)
  if (group != null) return group.parentId ?? null
  return state.workspaces.find((w) => w.id === nodeId)?.groupId ?? null
}

/// Move a node (workspace or group) within the tree (#49; #51 generalizes it):
///   - a workspace onto a group -> filed INSIDE it, appended at the end of
///     that group's children;
///   - a workspace to top-level space -> back at top level;
///   - any node with `beforeId` -> inserted before that sibling (reorder);
///   - a GROUP may nest into any group EXCEPT its own subtree — dropping a
///     group into one of its descendants would orphan it in a cycle and is
///     rejected (no-op, the drag cursor says "not allowed").
/// Unknown nodes, unknown target groups, forbidden cycles, and `beforeId`
/// ids that are not siblings of the target parent are no-ops too. Everything
/// else about the state (activation, open set, tabs, panels) rides along
/// untouched.
export function moveNode(
  state: WorkspaceState,
  nodeId: string,
  target: MoveTarget,
): WorkspaceState {
  const isGroupNode = state.groups.some((g) => g.id === nodeId)
  const ws = state.workspaces.find((w) => w.id === nodeId)
  if (!isGroupNode && ws == null) return state
  if (
    target.parentId != null &&
    !state.groups.some((g) => g.id === target.parentId)
  ) {
    return state
  }
  // Nesting safety (#51): the target must live OUTSIDE the moved group's own
  // subtree (the subtree includes the group itself, so dropping a group into
  // itself is rejected by the same guard).
  if (isGroupNode && target.parentId != null) {
    if (groupSubtreeIds(state, nodeId).includes(target.parentId)) return state
  }
  if (target.beforeId === nodeId) return state
  const parentId = target.parentId
  // Remove the node, then re-insert at its new sibling position.
  const without = state.order.filter((id) => id !== nodeId)
  let insertAt: number
  if (
    target.beforeId != null &&
    parentOf(state, target.beforeId) === parentId
  ) {
    const idx = without.indexOf(target.beforeId)
    insertAt = idx === -1 ? without.length : idx
  } else {
    // Append after the target parent's LAST sibling — a new child lands at
    // the end of its group, not at the end of the whole list. With no
    // siblings yet, the FIRST child lands right after its parent group
    // (never in front of it); a top-level append goes to the very end.
    let last = -1
    for (let i = 0; i < without.length; i++) {
      if (parentOf(state, without[i]) === parentId) last = i
    }
    if (last >= 0) {
      insertAt = last + 1
    } else if (parentId != null) {
      const p = without.indexOf(parentId)
      insertAt = p === -1 ? without.length : p + 1
    } else {
      insertAt = without.length
    }
  }
  const order = [...without.slice(0, insertAt), nodeId, ...without.slice(insertAt)]
  // A workspace leaving a group DROPS the `groupId` key — the persisted
  // payload stays byte-identical to one that was never grouped (same hygiene
  // as omitPinned). The group twin (#51): the moved GROUP gains / drops its
  // `parentId` the same way.
  const newParentId = target.parentId
  const workspaces = isGroupNode
    ? state.workspaces
    : newParentId != null
      ? state.workspaces.map((w) =>
          w.id === nodeId ? { ...w, groupId: newParentId } : w,
        )
      : state.workspaces.map((w) => {
          if (w.id !== nodeId) return w
          const { groupId: _dropped, ...rest } = w
          return rest
        })
  const groups = isGroupNode
    ? state.groups.map((g) =>
        g.id === nodeId
          ? newParentId != null
            ? { ...g, parentId: newParentId }
            : omitGroupParent(g)
          : g,
      )
    : state.groups
  return { ...state, workspaces, groups, order }
}

/// "Move to group…" with a FRESH name (#49): creates the group on the fly
/// (appended at the end of the top level, same as "New group") and files the
/// workspace into it as its only child. No-op for an unknown workspace id or
/// a blank name. genId order (stable, tests rely on it): the group's id only.
export function moveToNewGroup(
  state: WorkspaceState,
  workspaceId: string,
  name: string,
  genId: () => string = defaultGenId,
): WorkspaceState {
  if (!state.workspaces.some((w) => w.id === workspaceId)) return state
  if (name.trim() === '') return state
  const created = createGroup(state, name.trim(), genId)
  const gid = created.groups[created.groups.length - 1].id
  return moveNode(created, workspaceId, { parentId: gid })
}

// --- Collapse + subtree + destructive ops (#50/#51) --------------------------

/// Every node id inside group `id`'s SUBTREE, the group itself included:
/// descendant groups at any depth plus every workspace filed under them.
/// The membership test behind nesting (#51) — cycle rejection in moveNode,
/// badge summing (activeAgentCount), destructive delete and the drag's
/// forbidden set all read it. A `seen` guard keeps a hand-built cyclic state
/// from looping forever (bootState breaks cycles on load; the ops here never
/// create one).
export function groupSubtreeIds(state: WorkspaceState, groupId: string): string[] {
  const ids: string[] = []
  const seen = new Set<string>()
  const stack = [groupId]
  while (stack.length > 0) {
    const gid = stack.pop() as string
    if (seen.has(gid)) continue
    seen.add(gid)
    ids.push(gid)
    for (const g of state.groups) if (g.parentId === gid) stack.push(g.id)
    for (const w of state.workspaces) if (w.groupId === gid) ids.push(w.id)
  }
  return ids
}

/// Drop the `parentId` key so a top-level group persists byte-identical to
/// one that was never nested (same hygiene as omitPinned/omitCollapsed).
function omitGroupParent(group: Group): Group {
  const { parentId: _dropped, ...rest } = group
  return rest
}

/// Toggle group `id` collapsed/expanded IN PLACE (#50). The flag lives in the
/// tree (persisted), never in transient UI state; expanding DROPS the
/// `collapsed` key so the payload stays byte-identical to one that was never
/// collapsed. No-op for an unknown id.
export function toggleCollapse(
  state: WorkspaceState,
  id: string,
): WorkspaceState {
  if (!state.groups.some((g) => g.id === id)) return state
  return {
    ...state,
    groups: state.groups.map((g) =>
      g.id === id ? (g.collapsed === true ? omitCollapsed(g) : { ...g, collapsed: true }) : g,
    ),
  }
}

/// Drop the `collapsed` key on expand — byte-identical to a never-collapsed
/// group (same hygiene as omitGroupParent).
function omitCollapsed(group: Group): Group {
  const { collapsed: _dropped, ...rest } = group
  return rest
}

/// Unpack group `id` (#51): DISSOLVE it — every workspace and subgroup it
/// holds returns to TOP level, nothing closes, and the group node itself
/// disappears. The children ALREADY sit directly below the group in the
/// shared order, so dissolving is just re-parenting them (their parent keys
/// DROP — top level = no key, same hygiene as everywhere) and removing the
/// group node; nobody moves. Deeper descendants follow their own parents
/// unchanged. No-op for an unknown id.
export function unpackGroup(
  state: WorkspaceState,
  id: string,
): WorkspaceState {
  if (!state.groups.some((g) => g.id === id)) return state
  return {
    ...state,
    groups: state.groups
      .filter((g) => g.id !== id)
      .map((g) => (g.parentId === id ? omitGroupParent(g) : g)),
    workspaces: state.workspaces.map((w) =>
      w.groupId === id
        ? (() => {
            const { groupId: _dropped, ...rest } = w
            return rest
          })()
        : w,
    ),
    // The group node leaves the shared order; its (now top-level) children
    // keep their slots.
    order: state.order.filter((nodeId) => nodeId !== id),
  }
}

/// Delete group `id` WITH everything inside it (#51): the group, every
/// descendant group at any depth, and every workspace filed under them —
/// panels, tabs and their shells go away with it. Activation falls back per
/// the existing workspace-delete rules (next/previous open workspace in
/// sidebar order) when the active workspace was inside. No-op for an unknown
/// id.
export function deleteGroupSubtree(
  state: WorkspaceState,
  id: string,
): WorkspaceState {
  if (!state.groups.some((g) => g.id === id)) return state
  const doomedGroups = new Set(groupSubtreeIds(state, id))
  const doomedWs = new Set(
    state.workspaces
      .filter((w) => w.groupId != null && doomedGroups.has(w.groupId))
      .map((w) => w.id),
  )
  const nextActive = doomedWs.has(state.activeId ?? '')
    ? pickReplacementActive(state, state.activeId as string, doomedWs)
    : state.activeId
  // The doomed workspaces' tabs die with them — their zoom records must not
  // outlive them (same hygiene as deleteWorkspace).
  const dyingTabs = new Set(
    state.workspaces
      .filter((w) => doomedWs.has(w.id))
      .flatMap((w) => w.tabs?.map((t) => t.id) ?? []),
  )
  return {
    workspaces: state.workspaces.filter((w) => !doomedWs.has(w.id)),
    groups: state.groups.filter((g) => !doomedGroups.has(g.id)),
    order: state.order.filter(
      (nodeId) => !doomedGroups.has(nodeId) && !doomedWs.has(nodeId),
    ),
    activeId: nextActive,
    openIds: state.openIds.filter((openId) => !doomedWs.has(openId)),
    activeTabId: removeKeys(state.activeTabId, doomedWs),
    activePanelId: removeKeys(state.activePanelId, doomedWs),
    zoomedPanelId: Object.fromEntries(
      Object.entries(state.zoomedPanelId).filter(([tid]) => !dyingTabs.has(tid)),
    ),
  }
}

/// Shallow-copy `map` without EVERY key in `keys` — the multi-id twin of
/// removeKey.
function removeKeys<V>(
  map: Record<string, V>,
  keys: ReadonlySet<string>,
): Record<string, V> {
  return Object.fromEntries(
    Object.entries(map).filter(([k]) => !keys.has(k)),
  )
}

/// How many AGENT STATUSES inside group `id`'s subtree are ACTIVE right now
/// (#50) — the sidebar's `● N` badge on a collapsed group. Adam's call
/// (HITL fix 2026-08-28): the number must GROW with every active agent, so
/// this counts each panel whose status is `working` (the chip's running
/// dot) or `needs-attention` (the agent finished and WAITS for you — an
/// occupied panel, not an empty shell; an idle Claude Code sits in NA most
/// of the time) — across EVERY workspace and tab of the subtree, one point
/// per panel. Plain idle shells never count, and the badge clears when the
/// agents exit or are acknowledged. Since #51 the sum covers the WHOLE
/// subtree, which for flat groups is the same as the direct children
/// (#50's level). Updates live: statuses is the same Record the chips
/// render from, so any panel change re-renders the badge.
export function activeAgentCount(
  state: WorkspaceState,
  groupId: string,
  statuses: Readonly<Record<string, AgentStatus>>,
): number {
  let count = 0
  for (const nodeId of groupSubtreeIds(state, groupId)) {
    if (nodeId === groupId) continue
    const ws = state.workspaces.find((w) => w.id === nodeId)
    if (ws == null) continue
    for (const pid of panelIdsOf(state, ws.id)) {
      const st = statuses[pid]
      if (st === 'working' || st === 'needs-attention') count += 1
    }
  }
  return count
}

/// One flattened, ordered sidebar entry with its depth (#48) — the shape the
/// UI renders so indentation is a pure read. Depth is 0 for top-level nodes
/// and grows with NESTING (#51): a group's children render right below it,
/// one level deeper, in their shared order. A COLLAPSED group (#50) renders
/// alone — its whole subtree stays hidden until it expands again.
export type SidebarEntry =
  | { kind: 'group'; group: Group; depth: number }
  | { kind: 'workspace'; workspace: Workspace; depth: number }

export function flattenSidebar(state: WorkspaceState): SidebarEntry[] {
  const childrenOf = (parentId: string | null): string[] =>
    state.order.filter((nodeId) => parentOf(state, nodeId) === parentId)
  const walk = (parentId: string | null, depth: number): SidebarEntry[] =>
    childrenOf(parentId).flatMap((nodeId): SidebarEntry[] => {
      const group = state.groups.find((g) => g.id === nodeId)
      if (group != null) {
        const entry: SidebarEntry = { kind: 'group', group, depth }
        return group.collapsed === true ? [entry] : [entry, ...walk(group.id, depth + 1)]
      }
      const workspace = state.workspaces.find((w) => w.id === nodeId)
      return workspace == null ? [] : [{ kind: 'workspace', workspace, depth }]
    })
  return walk(null, 0)
}

/// The workspace's ACTIVE tab (runtime record with a first-tab fallback, so
/// a state built without an explicit entry still answers). Null for an
/// unknown workspace id.
export function activeTabOf(state: WorkspaceState, id: string): Tab | null {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws?.tabs == null || ws.tabs.length === 0) return null
  return ws.tabs.find((t) => t.id === state.activeTabId[id]) ?? ws.tabs[0]
}

/// Add a new terminal tab to workspace `id` and activate it (#37 rework).
/// Definitions change (persisted); the new tab is a single fresh shell.
/// No-op for an unknown workspace id.
export function addTab(
  state: WorkspaceState,
  id: string,
  genId: () => string = defaultGenId,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws == null) return state
  // Born named (stable numbering — pins and closes never reshuffle labels).
  const tab: Tab = {
    id: genId(),
    layout: createTree(genId()),
    name: nextTabName(ws.tabs ?? []),
  }
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id ? { ...w, tabs: [...(w.tabs ?? []), tab] } : w,
    ),
    activeTabId: { ...state.activeTabId, [id]: tab.id },
    activePanelId: {
      ...state.activePanelId,
      [id]: leafIds(tab.layout)[0],
    },
  }
}

/// Close one tab of workspace `id` (#37 rework): the tab's panels (and their
/// shells) go away with it, and their `panels` config entries are dropped so
/// the config never accumulates orphans. The neighboring tab (next, else
/// previous) becomes active. A workspace keeps at least ONE tab — closing
/// the last one is a no-op (close the workspace instead). No-op for unknown
/// ids.
export function closeTab(
  state: WorkspaceState,
  id: string,
  tabId: string,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws?.tabs == null) return state
  const idx = ws.tabs.findIndex((t) => t.id === tabId)
  if (idx === -1 || ws.tabs.length <= 1) return state
  const dyingLeaves = new Set(leafIds(ws.tabs[idx].layout))
  const tabs = ws.tabs.filter((t) => t.id !== tabId)
  const nextActiveTab = tabs[idx] ?? tabs[idx - 1]
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id
        ? {
            ...w,
            tabs,
            // `?.` keeps a panel-less workspace's payload unchanged.
            panels: w.panels?.filter((p) => !dyingLeaves.has(p.id)),
          }
        : w,
    ),
    activeTabId:
      state.activeTabId[id] === tabId
        ? { ...state.activeTabId, [id]: nextActiveTab.id }
        : state.activeTabId,
    activePanelId:
      state.activeTabId[id] === tabId
        ? {
            ...state.activePanelId,
            [id]: leafIds(nextActiveTab.layout)[0],
          }
        : state.activePanelId,
    // The tab's runtime records die with it (#40 zoom hygiene, same as the
    // active-tab/panel records above).
    zoomedPanelId: removeKey(state.zoomedPanelId, tabId),
  }
}

/// Activate tab `tabId` of workspace `id` (runtime-only). No-op for unknown
/// ids.
export function switchTab(
  state: WorkspaceState,
  id: string,
  tabId: string,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws?.tabs == null || !ws.tabs.some((t) => t.id === tabId)) return state
  return { ...state, activeTabId: { ...state.activeTabId, [id]: tabId } }
}

/// Rename a tab (Adam's follow-up to the #37 rework). Definitions change —
/// persisted. An empty name clears the field (the tab falls back to the
/// positional "Tab N"). No-op for unknown ids.
export function renameTab(
  state: WorkspaceState,
  id: string,
  tabId: string,
  name: string,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws?.tabs?.some((t) => t.id === tabId) !== true) return state
  const trimmed = name.trim()
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id !== id
        ? w
        : {
            ...w,
            tabs: (w.tabs ?? []).map((t) => {
              if (t.id !== tabId) return t
              const { name: _old, ...rest } = t
              // An empty commit DROPS the key — the tab persists unnamed,
              // byte-identical to one that was never named.
              return trimmed === '' ? rest : { ...rest, name: trimmed }
            }),
          },
    ),
  }
}

/// Pin or unpin a tab — the tab-level twin of setWorkspacePinned. Pinned
/// tabs sit at the TOP of the workspace's tab bar as a group, so the toggle
/// MOVES the tab in the array (the array stays the display order): pinning
/// appends to the END of the pinned block, unpinning inserts at the HEAD of
/// the unpinned block (the list never jumps). No-op for unknown ids or when
/// the flag already matches.
export function setTabPinned(
  state: WorkspaceState,
  id: string,
  tabId: string,
  pinned: boolean,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  const fromIndex = ws?.tabs?.findIndex((t) => t.id === tabId) ?? -1
  if (ws?.tabs == null || fromIndex === -1) return state
  const target = ws.tabs[fromIndex]
  if ((target.pinned ?? false) === pinned) return state
  const rest = ws.tabs.filter((t) => t.id !== tabId)
  const updated = pinned ? { ...target, pinned } : omitTabPinned(target)
  const insertAt = pinned
    ? rest.filter((t) => t.pinned === true).length
    : rest.findIndex((t) => t.pinned !== true)
  const index = insertAt === -1 ? rest.length : insertAt
  const tabs = [...rest.slice(0, index), updated, ...rest.slice(index)]
  return {
    ...state,
    workspaces: state.workspaces.map((w) => (w.id === id ? { ...w, tabs } : w)),
  }
}

/// Drop the `pinned` flag on unpin so an unpinned tab persists WITHOUT the
/// key — byte-identical to one that was never pinned.
function omitTabPinned(tab: Tab): Tab {
  const { pinned: _dropped, ...rest } = tab
  return rest
}

export function listWorkspaces(state: WorkspaceState): Workspace[] {
  return state.workspaces
}

export function renameWorkspace(
  state: WorkspaceState,
  id: string,
  name: string,
): WorkspaceState {
  if (!state.workspaces.some((w) => w.id === id)) return state
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id ? { ...w, name } : w,
    ),
  }
}

export function switchWorkspace(
  state: WorkspaceState,
  id: string,
): WorkspaceState {
  if (!state.workspaces.some((w) => w.id === id)) return state
  return { ...state, activeId: id }
}

/// Pick the next workspace to activate after `removedId` is gone from the open
/// set: the next open one in SIDEBAR order, else the previous open one, else
/// null (no open workspace left -> EmptyState). The tree's shared order is
/// the source of truth for what the user sees as "next" (#48). `excluded`
/// (#53 fix) names workspaces that are dying WITH the removed one — a
/// subtree delete must not hand activation to a sibling that dies in the
/// same operation.
function pickReplacementActive(
  state: WorkspaceState,
  removedId: string,
  excluded: ReadonlySet<string> = new Set(),
): string | null {
  const wsIds = new Set(state.workspaces.map((w) => w.id))
  const ids = state.order.filter((id) => wsIds.has(id))
  const idx = ids.indexOf(removedId)
  const after = ids
    .slice(idx + 1)
    .find((id) => !excluded.has(id) && state.openIds.includes(id))
  if (after != null) return after
  const before = ids
    .slice(0, idx)
    .reverse()
    .find((id) => !excluded.has(id) && state.openIds.includes(id))
  return before ?? null
}

/// Delete a workspace outright: drop its definition, close its panels, and hand
/// activation to a surviving open sibling (or none). No-op for an unknown id.
export function deleteWorkspace(
  state: WorkspaceState,
  id: string,
): WorkspaceState {
  if (!state.workspaces.some((w) => w.id === id)) return state
  const nextActive =
    state.activeId === id ? pickReplacementActive(state, id) : state.activeId
  // The workspace's tabs go with it — their zoom records must not outlive
  // them (#40; same hygiene as the active-tab/panel records).
  const dyingTabs = new Set(
    state.workspaces.find((w) => w.id === id)?.tabs?.map((t) => t.id) ?? [],
  )
  const zoomedPanelId = Object.fromEntries(
    Object.entries(state.zoomedPanelId).filter(([tid]) => !dyingTabs.has(tid)),
  )
  return {
    workspaces: state.workspaces.filter((w) => w.id !== id),
    groups: state.groups,
    // The node also leaves the shared sidebar order (#48) — a stale id there
    // would render a ghost row.
    order: state.order.filter((nodeId) => nodeId !== id),
    activeId: nextActive,
    openIds: state.openIds.filter((openId) => openId !== id),
    activeTabId: removeKey(state.activeTabId, id),
    activePanelId: removeKey(state.activePanelId, id),
    zoomedPanelId,
  }
}

/// Close a workspace without deleting its definition: the panels unmount
/// (shells torn down), but the workspace stays listed so it can be reopened.
/// No-op for an unknown id or one that is already closed.
export function closeWorkspace(
  state: WorkspaceState,
  id: string,
): WorkspaceState {
  if (!state.workspaces.some((w) => w.id === id)) return state
  if (!state.openIds.includes(id)) return state
  const nextActive =
    state.activeId === id ? pickReplacementActive(state, id) : state.activeId
  return {
    ...state,
    activeId: nextActive,
    openIds: state.openIds.filter((openId) => openId !== id),
  }
}

/// Activate a workspace and ensure its panel is open. Clicking a row uses this:
/// for an already-open workspace it just switches activation; for a closed one
/// it reopens it (respawning the shells on mount). No-op for an unknown id.
export function openWorkspace(
  state: WorkspaceState,
  id: string,
): WorkspaceState {
  if (!state.workspaces.some((w) => w.id === id)) return state
  const openIds = state.openIds.includes(id)
    ? state.openIds
    : [...state.openIds, id]
  return { ...state, activeId: id, openIds }
}

/// Pin or unpin a workspace (#37; re-homed onto the tree in #48; per
/// container since #52). A pinned workspace leads ONLY within its own parent
/// group — the flag travels with the workspace when it moves, but the
/// ordering is recomputed among the NEW container's siblings, so pin stays
/// local. The toggle reorders `state.order` among the target's siblings;
/// pinning appends after the pinned block, unpinning inserts at the head of
/// the unpinned block — the list never jumps. Non-sibling nodes keep their
/// absolute slots. No-op for an unknown id or when the flag already matches.
export function setWorkspacePinned(
  state: WorkspaceState,
  id: string,
  pinned: boolean,
): WorkspaceState {
  const target = state.workspaces.find((w) => w.id === id)
  if (target == null) return state
  if ((target.pinned ?? false) === pinned) return state
  const updated = pinned ? { ...target, pinned } : omitPinned(target)
  const withFlag: WorkspaceState = {
    ...state,
    workspaces: state.workspaces.map((w) => (w.id === id ? updated : w)),
  }
  return { ...withFlag, order: reorderPinned(withFlag, id, pinned) }
}

/// Pin or unpin a GROUP (#52): a pinned group leads at its OWN level — top
/// level or inside its parent group — and composes with nesting (inside a
/// pinned group, its pinned children still lead, each by their own flag).
/// Same ordering contract as setWorkspacePinned. The flag drops on unpin so
/// an unpinned group persists byte-identical to one that was never pinned.
/// No-op for an unknown id or when the flag already matches.
export function setGroupPinned(
  state: WorkspaceState,
  id: string,
  pinned: boolean,
): WorkspaceState {
  const target = state.groups.find((g) => g.id === id)
  if (target == null) return state
  if ((target.pinned ?? false) === pinned) return state
  const withFlag: WorkspaceState = {
    ...state,
    groups: state.groups.map((g) =>
      g.id === id ? (pinned ? { ...g, pinned } : omitGroupPinned(g)) : g,
    ),
  }
  return { ...withFlag, order: reorderPinned(withFlag, id, pinned) }
}

/// Drop the `pinned` flag on unpin so an unpinned group persists WITHOUT the
/// key — byte-identical to one that was never pinned (same hygiene as
/// omitPinned/omitCollapsed).
function omitGroupPinned(group: Group): Group {
  const { pinned: _dropped, ...rest } = group
  return rest
}

/// The pinned-first reorder shared by setWorkspacePinned and setGroupPinned
/// (#52): move `id` within its OWN sibling level in `state.order` — to the
/// end of the pinned block when `pinned`, else to the head of the unpinned
/// block. A node's pinned-ness reads off BOTH kinds (`isNodePinned`), so
/// pinned groups and pinned workspaces share the leading block at every
/// level and keep their relative order within it. `state` must already carry
/// the toggled flag. Non-siblings keep their absolute slots.
function reorderPinned(
  state: WorkspaceState,
  id: string,
  pinned: boolean,
): string[] {
  const parentId = parentOf(state, id)
  const isSibling = (nodeId: string): boolean =>
    parentOf(state, nodeId) === parentId
  const siblings = state.order.filter(isSibling)
  const rest = siblings.filter((nodeId) => nodeId !== id)
  const insertAt = pinned
    ? rest.filter((nodeId) => isNodePinned(state, nodeId)).length
    : rest.findIndex((nodeId) => !isNodePinned(state, nodeId))
  const index = insertAt === -1 ? rest.length : insertAt
  const sequence = [...rest.slice(0, index), id, ...rest.slice(index)]
  // Write the reordered sibling sequence back into the SAME order slots, so
  // nodes outside the sibling set never move.
  const slots: number[] = []
  state.order.forEach((nodeId, i) => {
    if (isSibling(nodeId)) slots.push(i)
  })
  const order = [...state.order]
  sequence.forEach((nodeId, k) => {
    order[slots[k]] = nodeId
  })
  return order
}

/// A node's pin flag, whichever kind it is — the per-level pinned block
/// mixes pinned groups and pinned workspaces (#52). Exported for the batch
/// selection menu (#53), whose "Pin all / Unpin all" label reads whether ANY
/// selected node is currently unpinned.
export function isNodePinned(state: WorkspaceState, nodeId: string): boolean {
  const group = state.groups.find((g) => g.id === nodeId)
  if (group != null) return group.pinned === true
  return state.workspaces.find((w) => w.id === nodeId)?.pinned === true
}

/// Drop the `pinned` flag on unpin so an unpinned workspace persists WITHOUT
/// the key — byte-identical to one that was never pinned (same hygiene as
/// removeKey for the runtime-only records).
function omitPinned(workspace: Workspace): Workspace {
  const { pinned: _dropped, ...rest } = workspace
  return rest
}

// --- Batch actions over a multi-selection (#53) ------------------------------
//
// A selection (src/selection.ts) is a list of sidebar node ids — workspaces
// and groups, mixed. Every batch op here is a COMPOSITION of the single-node
// pure ops above, applied sequentially, so each member goes through exactly
// the same guards (cycle rejection, unknown-id no-ops) as a solo action and
// an invalid member is skipped without aborting the rest.

/// Sort selection ids into current SIDEBAR order (state.order), dropping
/// unknown ids. Parent groups rank before their descendants by construction
/// of the shared order, which is what keeps a parent-and-child selection
/// coherent when both move in one gesture.
function orderNodesBySidebar(state: WorkspaceState, nodeIds: readonly string[]): string[] {
  const rank = new Map(state.order.map((id, i) => [id, i]))
  return nodeIds
    .filter((id) => rank.has(id))
    .sort((a, b) => (rank.get(a) as number) - (rank.get(b) as number))
}

/// Move EVERY selected node per one drop target (#53) — dragging any member
/// of a selection drags all of it: into a group, back to top level, or
/// reordered before a sibling. Members apply in SIDEBAR order, which keeps
/// the selection's own relative order for BOTH target kinds: appends land in
/// selection order, and each insert-before lands directly before the target
/// sibling (below the members applied before it). Invalid members (a group
/// whose target sits inside its own subtree, unknown ids) are skipped by
/// moveNode's no-op without aborting the rest.
export function moveNodes(
  state: WorkspaceState,
  nodeIds: readonly string[],
  target: MoveTarget,
): WorkspaceState {
  let next = state
  for (const id of orderNodesBySidebar(state, nodeIds)) {
    next = moveNode(next, id, target)
  }
  return next
}

/// Close every selected WORKSPACE, keeping their definitions (#53). Groups in
/// the selection are ignored — closing is a workspace concept — and
/// closeWorkspace's own unknown-id no-op skips anything already gone. Applied
/// sequentially so activation falls back per close, per the existing rules.
export function closeWorkspaces(
  state: WorkspaceState,
  nodeIds: readonly string[],
): WorkspaceState {
  let next = state
  for (const id of nodeIds) next = closeWorkspace(next, id)
  return next
}

/// Delete every selected node (#53): a selected group dies with its WHOLE
/// subtree (deleteGroupSubtree), a selected workspace dies alone
/// (deleteWorkspace). A workspace that a selected ancestor group already took
/// with it is skipped by deleteWorkspace's no-op, so overlapping selections
/// (a group AND a workspace inside it) never double-delete. The shared
/// confirmation is the caller's job — batchDeleteWorkspaceCount feeds it.
export function deleteNodes(
  state: WorkspaceState,
  nodeIds: readonly string[],
): WorkspaceState {
  let next = state
  for (const id of nodeIds) {
    next = state.groups.some((g) => g.id === id)
      ? deleteGroupSubtree(next, id)
      : deleteWorkspace(next, id)
  }
  return next
}

/// Pin or unpin EVERY selected node (#53): workspaces via setWorkspacePinned,
/// groups via setGroupPinned. Members already in the target state no-op (the
/// set*Pinned ops refuse to move a node whose flag matches), so the pinned
/// block forms once and the list never jumps per member.
export function setNodesPinned(
  state: WorkspaceState,
  nodeIds: readonly string[],
  pinned: boolean,
): WorkspaceState {
  let next = state
  for (const id of nodeIds) {
    next = next.groups.some((g) => g.id === id)
      ? setGroupPinned(next, id, pinned)
      : setWorkspacePinned(next, id, pinned)
  }
  return next
}

/// How many workspaces a batch delete would remove (#53): the union of the
/// selected workspaces and everything inside selected groups, DEDUPED — a
/// workspace both selected directly and living inside a selected group counts
/// once. This is the number the shared confirmation shows.
export function batchDeleteWorkspaceCount(
  state: WorkspaceState,
  nodeIds: readonly string[],
): number {
  const doomed = new Set<string>()
  for (const id of nodeIds) {
    if (state.groups.some((g) => g.id === id)) {
      for (const nodeId of groupSubtreeIds(state, id)) {
        if (state.workspaces.some((w) => w.id === nodeId)) doomed.add(nodeId)
      }
    } else if (state.workspaces.some((w) => w.id === id)) {
      doomed.add(id)
    }
  }
  return doomed.size
}

/// Reorder workspace `id`'s tabs (#45): drag & drop inside ONE workspace's
/// tab bar (HITL: no cross-workspace drags). Splice the tab out, clamp the
/// target index, splice it back. Everything hangs off
/// the tab's ID (name, panels, pinned flag, the active-tab record), so only
/// ORDER changes. No-op for unknown workspace/tab ids.
export function moveTab(
  state: WorkspaceState,
  id: string,
  tabId: string,
  newIndex: number,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws?.tabs == null) return state
  const fromIndex = ws.tabs.findIndex((t) => t.id === tabId)
  if (fromIndex === -1) return state
  const next = [...ws.tabs]
  const [moved] = next.splice(fromIndex, 1)
  const clamped = Math.max(0, Math.min(newIndex, next.length))
  next.splice(clamped, 0, moved)
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id ? { ...w, tabs: next } : w,
    ),
  }
}

/// The panel ids of workspace `id`, in tab-then-tree order — the stable
/// identity the renderer keys terminal surfaces by. Covers EVERY tab's
/// panes (#37 rework), with the legacy top-level `layout` as a fallback for
/// states that predate the migration. Empty for an unknown workspace.
export function panelIdsOf(state: WorkspaceState, id: string): string[] {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws == null) return []
  if (ws.tabs != null && ws.tabs.length > 0) {
    return ws.tabs.flatMap((t) => leafIds(t.layout))
  }
  return ws.layout == null ? [] : leafIds(ws.layout)
}

/// Record a snapshot cwd for leaf `panelId` (v0.2 Phase 5 / #29): upserts the
/// panel's `workingDirectory` in its workspace's `panels` array, creating the
/// entry when absent. The workspace is located by the LAYOUT TREE (the entry
/// may not exist yet); leaf ids are UUIDs, so the first workspace whose tree
/// contains the leaf is the right one. No-op for an unknown panel id (a panel
/// that was just closed, or a cwd read that raced a teardown) and for an
/// empty cwd.
export function upsertPanelCwd(
  state: WorkspaceState,
  panelId: string,
  cwd: string,
): WorkspaceState {
  if (cwd === '') return state
  // The owning workspace is located by the LAYOUT TREES — across every tab
  // since the #37 rework (leaf ids are UUIDs, so the first workspace whose
  // tabs contain the leaf is the right one).
  const ownsLeaf = (w: Workspace): boolean =>
    w.tabs != null && w.tabs.length > 0
      ? w.tabs.some((t) => leafIds(t.layout).includes(panelId))
      : w.layout != null && leafIds(w.layout).includes(panelId)
  const owner = state.workspaces.find(ownsLeaf)
  if (owner == null) return state
  // Same cwd as stored: return the SAME state object (reference equality) —
  // the periodic snapshot uses it to skip both the re-render and the disk
  // write when nothing moved.
  if (owner.panels?.some((p) => p.id === panelId && p.workingDirectory === cwd)) {
    return state
  }
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id !== owner.id
        ? w
        : {
            ...w,
            panels: (w.panels ?? []).some((p) => p.id === panelId)
              ? (w.panels ?? []).map((p) =>
                  p.id === panelId ? { ...p, workingDirectory: cwd } : p,
                )
              : [...(w.panels ?? []), { id: panelId, workingDirectory: cwd }],
          },
    ),
  }
}

/// Split the ACTIVE panel of workspace `id`'s ACTIVE TAB in `orientation`
/// (v0.2 / #25, stories 15–17 — unlimited panels, no cap; #37 rework moved
/// the tree under the tab). The existing panel keeps its shell (its leaf
/// becomes the split's first child); `genId` yields the new split-node id,
/// then the new leaf id. The new panel becomes focused so keystrokes follow
/// the split. The new leaf INHERITS the split target's panel config (v0.2
/// Phase 5 / #29): splitting a panel that sits in ~/proj gives its sibling
/// the same cwd — and an SSH panel splits into a second shell on the same
/// host. No-op for an unknown workspace id (or a workspace with no tabs).
export function splitPanel(
  state: WorkspaceState,
  id: string,
  orientation: Orientation,
  genId: () => string = defaultGenId,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  const tab = activeTabOf(state, id)
  if (ws == null || tab == null) return state
  const layout = tab.layout
  const target = activePanelOf(state, id) ?? leafIds(layout)[0]
  if (target == null) return state
  const ids = { splitId: genId(), newLeafId: genId() }
  const next = splitLeaf(layout, target, orientation, ids)
  if (next == null) return state
  const inherited = (ws.panels ?? []).find((p) => p.id === target)
  const newEntry = inherited
    ? {
        id: ids.newLeafId,
        ...(inherited.workingDirectory != null
          ? { workingDirectory: inherited.workingDirectory }
          : {}),
        ...(inherited.sshTarget != null ? { sshTarget: inherited.sshTarget } : {}),
      }
    : undefined
  // Only touch `panels` when there is something to record — a workspace
  // without entries (undefined) must persist unchanged.
  const panels =
    newEntry != null
      ? [...(ws.panels ?? []).filter((p) => p.id !== ids.newLeafId), newEntry]
      : ws.panels
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id
        ? {
            ...w,
            tabs: (w.tabs ?? []).map((t) =>
              t.id === tab.id ? { ...t, layout: next } : t,
            ),
            panels,
          }
        : w,
    ),
    activePanelId: { ...state.activePanelId, [id]: ids.newLeafId },
  }
}

/// Move the divider of workspace `id`'s split `splitId` to `ratio` (v0.2 / #25,
/// story 18), clamped so neither side shrinks below `minSize` px along the
/// split's axis (story 19). `container` is that split node's OWN extent (the
/// renderer passes its measured rect), so the clamp is correct at any nesting
/// depth. No-op for an unknown workspace or split id.
export function resizePanel(
  state: WorkspaceState,
  id: string,
  splitId: string,
  ratio: number,
  container: Container,
  minSize?: number,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  const tab = activeTabOf(state, id)
  if (ws == null || tab == null) return state
  const next = setRatio(tab.layout, splitId, ratio, container, minSize)
  if (next === tab.layout) return state
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id
        ? {
            ...w,
            tabs: (w.tabs ?? []).map((t) =>
              t.id === tab.id ? { ...t, layout: next } : t,
            ),
          }
        : w,
    ),
  }
}

/// Close one panel of workspace `id` — the panel whose leaf id is `panelId`
/// (v0.2 / #25, story 20). Siblings fill the freed space (the tree collapses
/// one-child splits); the closed leaf's `panels` entry is dropped with it so
/// the config never accumulates orphans (#29). No-op when the workspace id is
/// unknown, the panel does not exist, or only one panel is mounted (close the
/// workspace instead).
export function closePanel(
  state: WorkspaceState,
  id: string,
  panelId: string,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  const tab = activeTabOf(state, id)
  if (ws == null || tab == null) return state
  if (leafIds(tab.layout).length <= 1) return state
  const next = closeLeaf(tab.layout, panelId)
  if (next == null) return state
  // If the closed panel was the focused one, hand focus to the first survivor
  // so keystrokes always land somewhere (Phase 11 / #12, story 34).
  const activePanelId =
    activePanelOf(state, id) === panelId
      ? { ...state.activePanelId, [id]: leafIds(next)[0] }
      : state.activePanelId
  // Closing the ZOOMED panel exits zoom (#40): the zoom record dies with the
  // panel it pointed at, and the surviving layout shows intact. A covered
  // panel closing leaves the zoom untouched.
  const zoomedPanelId =
    state.zoomedPanelId[tab.id] === panelId
      ? removeKey(state.zoomedPanelId, tab.id)
      : state.zoomedPanelId
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id
        ? {
            ...w,
            tabs: (w.tabs ?? []).map((t) =>
              t.id === tab.id ? { ...t, layout: next } : t,
            ),
            // `?.` keeps a panel-less workspace's payload unchanged.
            panels: w.panels?.filter((p) => p.id !== panelId),
          }
        : w,
    ),
    activePanelId,
    zoomedPanelId,
  }
}

/// Mark `panelId` as the focused panel of workspace `id` (Phase 11 / #12,
/// story 34). Runtime-only — not persisted. No-op (returns `state` unchanged)
/// when the workspace id is unknown or `panelId` is not one of its panels.
export function focusPanel(
  state: WorkspaceState,
  id: string,
  panelId: string,
): WorkspaceState {
  if (!state.workspaces.some((w) => w.id === id)) return state
  if (!panelIdsOf(state, id).includes(panelId)) return state
  return { ...state, activePanelId: { ...state.activePanelId, [id]: panelId } }
}

/// The currently focused panel of workspace `id` — the panel that should wear
/// the focus ring. Scoped to the ACTIVE TAB since the #37 rework: the stored
/// focus is only honored while it points INTO that tab (a record left by a
/// tab the user switched away from must not steer splits/keystrokes), else
/// it falls back to the tab's first panel in tree order. Null when the
/// workspace is unknown or has no panels.
export function activePanelOf(
  state: WorkspaceState,
  id: string,
): string | null {
  const tab = activeTabOf(state, id)
  if (tab == null) return null
  const panels = leafIds(tab.layout)
  if (panels.length === 0) return null
  const focused = state.activePanelId[id]
  return focused != null && panels.includes(focused) ? focused : panels[0]
}

/// Toggle zoom for the ACTIVE TAB of workspace `id` (#40 / story 48): the
/// focused panel — or the explicit `panelId`, the UI-button path — expands to
/// fill the tab, and the same toggle restores the exact previous layout. Pure
/// view state: only the runtime record moves; the split tree, the workspace
/// definitions, and the persisted config are untouched, so restore is exact
/// by construction. No-op for an unknown workspace, a panel that is not in
/// the tab's tree, or a tab with a single panel (nothing to cover).
export function toggleZoom(
  state: WorkspaceState,
  id: string,
  panelId?: string,
): WorkspaceState {
  const tab = activeTabOf(state, id)
  if (tab == null) return state
  // Already zoomed: the same action restores the previous layout.
  if (state.zoomedPanelId[tab.id] != null) {
    return { ...state, zoomedPanelId: removeKey(state.zoomedPanelId, tab.id) }
  }
  const target = panelId ?? activePanelOf(state, id)
  const leaves = leafIds(tab.layout)
  if (target == null || !leaves.includes(target) || leaves.length < 2) {
    return state
  }
  return {
    ...state,
    zoomedPanelId: { ...state.zoomedPanelId, [tab.id]: target },
  }
}

/// The zoomed panel of tab `tabId`, validated against the tab's tree — a
/// stale record (its panel is no longer a leaf) reads as NOT zoomed. Null
/// for an unknown tab id or a tab that is not zoomed.
export function zoomedPanelOf(
  state: WorkspaceState,
  tabId: string,
): string | null {
  const tab = state.workspaces
    .flatMap((w) => w.tabs ?? [])
    .find((t) => t.id === tabId)
  if (tab == null) return null
  const zoomed = state.zoomedPanelId[tabId]
  return zoomed != null && leafIds(tab.layout).includes(zoomed) ? zoomed : null
}
