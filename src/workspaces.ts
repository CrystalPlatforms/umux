// workspaces — pure state module for the workspace collection (Phase 6 / #7).
//
// Deep module: a tiny interface (create / rename / switch / list) over the
// in-memory workspace model. No I/O — persistence lives in the Rust
// WorkspaceStore; this module is fed by it on startup and triggers a save on
// every mutation. Trivially unit-testable.

export type Workspace = { id: string; name: string }

export type WorkspaceState = {
  workspaces: Workspace[]
  activeId: string | null
}

export const emptyState: WorkspaceState = {
  workspaces: [],
  activeId: null,
}

const defaultGenId = (): string => crypto.randomUUID()

export function createWorkspace(
  state: WorkspaceState,
  name: string,
  genId: () => string = defaultGenId,
): WorkspaceState {
  const workspace: Workspace = { id: genId(), name }
  return {
    workspaces: [...state.workspaces, workspace],
    activeId: workspace.id,
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
