// selection — pure multi-select state for the sidebar (#53).
//
// A selection is an ordered list of sidebar node ids (workspaces AND groups,
// mixed). It is RUNTIME-ONLY UI state — never persisted, never part of
// WorkspaceState — so this module stays free of the tree model and is
// trivially unit-testable. The batch ACTIONS over a selection live in
// workspaces.ts (moveNodes / closeWorkspaces / deleteNodes / setNodesPinned);
// this module only builds and clears the set.

/// Toggle `id` in the selection (#53): adds it when absent, removes it when
/// present. Order of the surviving members is preserved — the selection's
/// order never drives layout, but stable order keeps the ghost label and the
/// menu's batch resolution deterministic.
export function toggleInSelection(
  selected: readonly string[],
  id: string,
): string[] {
  return selected.includes(id)
    ? selected.filter((nodeId) => nodeId !== id)
    : [...selected, id]
}

/// Clear the selection (#53): Escape and a click on the sidebar background
/// both land here. Returns a fresh empty list so callers can use it directly
/// as the next state.
export function clearSelection(): string[] {
  return []
}

/// True when the platform is macOS — the ONE platform where the multi-select
/// modifier is Cmd (⌘) instead of Ctrl. The sidebar's menu gesture (Ctrl+
/// click on macOS = right-click) shares the Ctrl key, so the two gestures
/// must never collide: on macOS multi-select reads metaKey only.
export function isMacPlatform(): boolean {
  return (
    typeof navigator !== 'undefined' &&
    /Mac|iPhone|iPad/i.test(navigator.platform)
  )
}

/// Whether this pointer press carries the platform's multi-select modifier
/// (#53): Cmd on macOS, Ctrl on Linux/Windows. A plain press (no modifier)
/// never toggles the selection.
export function isMultiSelectModifier(e: {
  metaKey: boolean
  ctrlKey: boolean
}): boolean {
  return isMacPlatform() ? e.metaKey : e.ctrlKey
}
