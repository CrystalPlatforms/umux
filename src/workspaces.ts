// workspaces — pure state module for the workspace collection (Phase 6 / #7).
//
// Deep module: a tiny interface (create / rename / switch / list) over the
// in-memory workspace model. No I/O — persistence lives in the Rust
// WorkspaceStore; this module is fed by it on startup and triggers a save on
// every mutation. Trivially unit-testable.

// Forward-compat slot for Phase 9 (split into panels). `workingDirectory` and
// `sshTarget` are optional: absent = local panel in the default cwd. Kept
// byte-identical to the Rust `Panel` in workspace_store.rs.
export type Panel = {
  id: string
  workingDirectory?: string
  sshTarget?: string
}

export type Workspace = { id: string; name: string; panels: Panel[] }

export type WorkspaceState = {
  workspaces: Workspace[]
  activeId: string | null
  // Runtime-only (NOT persisted, like activeId): which workspaces currently
  // have a live, mounted panel — i.e. an open shell. A workspace not in this
  // list is "closed": its definition stays but its panel (and shell) is gone.
  openIds: string[]
}

export const emptyState: WorkspaceState = {
  workspaces: [],
  activeId: null,
  openIds: [],
}

const defaultGenId = (): string => crypto.randomUUID()

export function createWorkspace(
  state: WorkspaceState,
  name: string,
  genId: () => string = defaultGenId,
): WorkspaceState {
  const workspace: Workspace = { id: genId(), name, panels: [] }
  return {
    workspaces: [...state.workspaces, workspace],
    activeId: workspace.id,
    openIds: [...state.openIds, workspace.id],
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

/// Delete a workspace outright: drop its definition, close its panel, and hand
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
  }
}

/// Close a workspace without deleting its definition: the panel unmounts
/// (shell torn down), but the workspace stays listed so it can be reopened.
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
/// it reopens it (respawning the shell on mount). No-op for an unknown id.
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
