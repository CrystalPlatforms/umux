// tabSelection — pure multi-select state for TABS (#106, stories 12–15).
//
// The tab-bar twin of selection.ts (#53 sidebar): a selection is an ordered
// list of tab ids INSIDE ONE workspace (cross-workspace selection is out of
// scope for #106 — the UI starts a fresh selection when a modifier click
// lands in another workspace's bar). RUNTIME-ONLY UI state — never persisted,
// never part of WorkspaceState — so this module stays free of the tree model
// and is trivially unit-testable. The batch ACTIONS over a selection
// (moveTabGroup / moveTabsToWorkspace / closeTabs) live in workspaces.ts,
// composed of the same single-tab ops a solo action uses.

import { toggleInSelection, clearSelection } from './selection'

/// Toggle `tabId` in the selection (#106): adds it when absent, removes it
/// when present. Ctrl+click lands here (Cmd on macOS — the gesture decision
/// is isMultiSelectModifier's, shared with the sidebar).
export function toggleTabInSelection(
  selected: readonly string[],
  tabId: string,
): string[] {
  return toggleInSelection(selected, tabId)
}

/// The Shift+click range (#106): every tab from `anchorId` to `targetId`
/// INCLUSIVE, in the bar's CURRENT display order — whichever of the two sits
/// first. The range REPLACES the selection (browser file-list semantics).
/// Without a usable anchor (no prior selection, or the anchor tab is no
/// longer in the bar) the range is just the clicked tab.
export function rangeTabSelection(
  order: readonly string[],
  anchorId: string | null,
  targetId: string,
): string[] {
  if (anchorId == null) return [targetId]
  const a = order.indexOf(anchorId)
  const b = order.indexOf(targetId)
  if (a === -1 || b === -1) return [targetId]
  const [lo, hi] = a <= b ? [a, b] : [b, a]
  return order.slice(lo, hi + 1)
}

/// Clear the selection (#106): Escape and a plain click both land here.
/// Returns a fresh empty list so callers can use it directly as the next
/// state.
export function clearTabSelection(): string[] {
  return clearSelection()
}
