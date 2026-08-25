// workspaces — pure state module for the workspace collection (Phase 6 / #7).
//
// Deep module: a tiny interface (create / rename / switch / list) over the
// in-memory workspace model. No I/O — persistence lives in the Rust
// WorkspaceStore; this module is fed by it on startup and triggers a save on
// every mutation. Trivially unit-testable.
//
// v0.2 Phase 1 / #25: the panel layout is a split TREE persisted INSIDE each
// Workspace (`layout`). Leaf ids are panel ids — the runtime layouts/panelIds
// records of v0.1 are gone; panel identity derives from the tree.

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

export type Workspace = {
  id: string
  name: string
  // Optional at runtime: hand-edited and pre-v0.2 configs carry none, and
  // the persisted payload must not gain the key just by passing through
  // here — guards at the use sites keep old saves byte-identical.
  panels?: Panel[]
  // Split tree (v0.2 / #25); leaf ids ARE panel ids. Optional because
  // pre-v0.2 configs have none — bootState seeds a fresh single leaf there.
  layout?: LayoutNode
}

export type WorkspaceState = {
  workspaces: Workspace[]
  activeId: string | null
  // Runtime-only (NOT persisted, like activeId): which workspaces currently
  // have a live, mounted panel — i.e. an open shell. A workspace not in this
  // list is "closed": its definition stays but its panels (and shells) are gone.
  openIds: string[]
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
  activePanelId: {},
}

const defaultGenId = (): string => crypto.randomUUID()

/// Return a shallow copy of `map` without `key`. Pure helper for runtime-only
/// records keyed by workspace id.
function removeKey<V>(map: Record<string, V>, key: string): Record<string, V> {
  const { [key]: _removed, ...rest } = map
  return rest
}

/// Seed app state from the persisted config (v0.2 / #25, AC4). A workspace
/// that already carries a layout tree keeps it untouched (restart round-trip);
/// a pre-v0.2 workspace without one gets a fresh single leaf so it still opens
/// one shell. Every loaded workspace starts open and the first is active.
export function bootState(
  loaded: Workspace[],
  genId: () => string = defaultGenId,
): WorkspaceState {
  const workspaces = loaded.map((w) =>
    w.layout == null ? { ...w, layout: createTree(genId()) } : w,
  )
  return {
    workspaces,
    activeId: workspaces[0]?.id ?? null,
    openIds: workspaces.map((w) => w.id),
    activePanelId: Object.fromEntries(
      workspaces.map((w) => [w.id, leafIds(w.layout ?? createTree(''))[0]]),
    ),
  }
}

export function createWorkspace(
  state: WorkspaceState,
  name: string,
  genId: () => string = defaultGenId,
): WorkspaceState {
  const workspace: Workspace = { id: genId(), name, panels: [] }
  const layout = createTree(genId())
  workspace.layout = layout
  return {
    workspaces: [...state.workspaces, workspace],
    activeId: workspace.id,
    openIds: [...state.openIds, workspace.id],
    activePanelId: { ...state.activePanelId, [workspace.id]: leafIds(layout)[0] },
  }
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

/// The panel ids of workspace `id`, in tree order — the stable identity the
/// renderer keys terminal surfaces by. Empty for an unknown workspace.
export function panelIdsOf(state: WorkspaceState, id: string): string[] {
  const ws = state.workspaces.find((w) => w.id === id)
  return ws?.layout == null ? [] : leafIds(ws.layout)
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
  const owner = state.workspaces.find(
    (w) => w.layout != null && leafIds(w.layout).includes(panelId),
  )
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

/// Split the ACTIVE panel of workspace `id` in `orientation` (v0.2 / #25,
/// stories 15–17 — unlimited panels, no cap). The existing panel keeps its
/// shell (its leaf becomes the split's first child); `genId` yields the new
/// split-node id, then the new leaf id. The new panel becomes focused so
/// keystrokes follow the split. The new leaf INHERITS the split target's
/// panel config (v0.2 Phase 5 / #29): splitting a panel that sits in ~/proj
/// gives its sibling the same cwd — and an SSH panel splits into a second
/// shell on the same host. No-op for an unknown workspace id.
export function splitPanel(
  state: WorkspaceState,
  id: string,
  orientation: Orientation,
  genId: () => string = defaultGenId,
): WorkspaceState {
  const ws = state.workspaces.find((w) => w.id === id)
  if (ws == null) return state
  const layout = ws.layout ?? createTree(genId())
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
      w.id === id ? { ...w, layout: next, panels } : w,
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
  if (ws == null || ws.layout == null) return state
  const next = setRatio(ws.layout, splitId, ratio, container, minSize)
  if (next === ws.layout) return state
  return {
    ...state,
    workspaces: state.workspaces.map((w) =>
      w.id === id ? { ...w, layout: next } : w,
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
  if (ws == null || ws.layout == null) return state
  if (leafIds(ws.layout).length <= 1) return state
  const next = closeLeaf(ws.layout, panelId)
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
            layout: next,
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
/// the focus ring. Falls back to the first panel in tree order when none has
/// been focused explicitly (so a single-panel workspace is always "focused"),
/// and to null when the workspace is unknown or has no panels.
export function activePanelOf(
  state: WorkspaceState,
  id: string,
): string | null {
  const panels = panelIdsOf(state, id)
  if (panels.length === 0) return null
  return state.activePanelId[id] ?? panels[0]
}
