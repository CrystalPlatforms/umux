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

export type Workspace = {
  id: string
  name: string
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
}

export const emptyState: WorkspaceState = {
  workspaces: [],
  activeId: null,
  openIds: [],
  activeTabId: {},
  activePanelId: {},
}

const defaultGenId = (): string => crypto.randomUUID()

/// Return a shallow copy of `map` without `key`. Pure helper for runtime-only
/// records keyed by workspace id.
function removeKey<V>(map: Record<string, V>, key: string): Record<string, V> {
  const { [key]: _removed, ...rest } = map
  return rest
}

/// Seed app state from the persisted config (v0.2 / #25, AC4; reworked for
/// tabs in #37). The migration ladder per workspace:
///   - already has `tabs` (a rework-era config): kept untouched;
///   - only the legacy top-level `layout` (v0.2 / first #37 cut): it becomes
///     the workspace's single tab and the legacy key is STRIPPED, so the next
///     save writes the new shape and never both;
///   - neither (pre-v0.2): one fresh tab with a single-leaf tree.
/// Every loaded workspace starts open with its first tab and first panel
/// focused, and the first workspace is active.
export function bootState(
  loaded: Workspace[],
  genId: () => string = defaultGenId,
): WorkspaceState {
  const migrate = (w: Workspace): Workspace => {
    if (w.tabs != null && w.tabs.length > 0) {
      // Drop a legacy `layout` key if a hand-merge left both behind, and
      // FILL missing tab names with stable positional defaults: an unnamed
      // tab displayed as "Tab N" by POSITION reshuffles its number whenever
      // a pin/reorder moves it (HITL: "I pin tab 2 and tab 1 gets pinned"),
      // so every tab gets a name that sticks at boot.
      const { layout: _legacy, ...rest } = w
      return {
        ...rest,
        tabs: w.tabs.map((t, i) =>
          t.name == null ? { ...t, name: `Tab ${i + 1}` } : t,
        ),
      }
    }
    const tree = w.layout ?? createTree(genId())
    const { layout: _legacy, ...rest } = w
    return { ...rest, tabs: [{ id: genId(), layout: tree, name: 'Tab 1' }] }
  }
  const workspaces = loaded.map(migrate)
  return {
    workspaces,
    activeId: workspaces[0]?.id ?? null,
    openIds: workspaces.map((w) => w.id),
    activeTabId: Object.fromEntries(
      workspaces.map((w) => [w.id, w.tabs?.[0]?.id ?? '']),
    ),
    activePanelId: Object.fromEntries(
      workspaces.map((w) => [w.id, w.tabs ? leafIds(w.tabs[0].layout)[0] : '']),
    ),
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
    activeId: id,
    openIds: [...state.openIds, id],
    activeTabId: { ...state.activeTabId, [id]: tab.id },
    activePanelId: {
      ...state.activePanelId,
      [id]: leafIds(tab.layout)[0],
    },
  }
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
/// set: the next open sibling in list order, else the previous open one, else
/// null (no open workspace left -> EmptyState). Definitions order is the source
/// of truth for "sibling".
function pickReplacementActive(
  state: WorkspaceState,
  removedId: string,
): string | null {
  const ids = state.workspaces.map((w) => w.id)
  const idx = ids.indexOf(removedId)
  const after = ids.slice(idx + 1).find((id) => state.openIds.includes(id))
  if (after != null) return after
  const before = ids
    .slice(0, idx)
    .reverse()
    .find((id) => state.openIds.includes(id))
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
  return {
    workspaces: state.workspaces.filter((w) => w.id !== id),
    activeId: nextActive,
    openIds: state.openIds.filter((openId) => openId !== id),
    activeTabId: removeKey(state.activeTabId, id),
    activePanelId: removeKey(state.activePanelId, id),
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

/// Pin or unpin a workspace (#37). Pinned workspaces always sit at the TOP of
/// the list as a group, so the toggle MOVES the definition in the array — the
/// array stays the single source of display order (drag-and-drop, the sidebar
/// list, and the tab bar all read it directly). Pinning appends to the END of
/// the pinned block; unpinning inserts at the HEAD of the unpinned block (the
/// workspace lands right behind the pinned group — the list never jumps).
/// No-op for an unknown id or when the flag already matches.
export function setWorkspacePinned(
  state: WorkspaceState,
  id: string,
  pinned: boolean,
): WorkspaceState {
  const fromIndex = state.workspaces.findIndex((w) => w.id === id)
  if (fromIndex === -1) return state
  const target = state.workspaces[fromIndex]
  if ((target.pinned ?? false) === pinned) return state
  const rest = state.workspaces.filter((w) => w.id !== id)
  const updated = pinned ? { ...target, pinned } : omitPinned(target)
  const insertAt = pinned
    ? rest.filter((w) => w.pinned === true).length
    : rest.findIndex((w) => w.pinned !== true)
  const index = insertAt === -1 ? rest.length : insertAt
  return {
    ...state,
    workspaces: [...rest.slice(0, index), updated, ...rest.slice(index)],
  }
}

/// Drop the `pinned` flag on unpin so an unpinned workspace persists WITHOUT
/// the key — byte-identical to one that was never pinned (same hygiene as
/// removeKey for the runtime-only records).
function omitPinned(workspace: Workspace): Workspace {
  const { pinned: _dropped, ...rest } = workspace
  return rest
}

/// Reorder a workspace definition: move the workspace with `id` to `newIndex`
/// in the list (clamped to the valid range). Definitions-only — openIds are a
/// runtime set and are not reordered. No-op for an unknown id.
export function moveWorkspace(
  state: WorkspaceState,
  id: string,
  newIndex: number,
): WorkspaceState {
  const fromIndex = state.workspaces.findIndex((w) => w.id === id)
  if (fromIndex === -1) return state
  const next = [...state.workspaces]
  const [moved] = next.splice(fromIndex, 1)
  const clamped = Math.max(0, Math.min(newIndex, next.length))
  next.splice(clamped, 0, moved)
  return { ...state, workspaces: next }
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
