// EmptyState — the friendly onboarding screen shown in the main area when the
// user has no workspaces yet (Phase 17 / Issue #18). It welcomes them and
// offers a single, obvious path forward: create the first workspace.
//
// Presentational: the host (WorkspaceShell) decides when to show it and what
// the create action does. `onCreate` is optional only so the heading/guidance
// can be rendered in isolation where needed.

type EmptyStateProps = {
  onCreate?: () => void
}

export function EmptyState({ onCreate }: EmptyStateProps = {}) {
  return (
    <div className="empty-state">
      <h1>Welcome to umux</h1>
      <p>Create a workspace to group terminals for a project.</p>
      {onCreate && (
        <button className="btn-primary" onClick={onCreate}>
          Create workspace
        </button>
      )}
    </div>
  )
}
