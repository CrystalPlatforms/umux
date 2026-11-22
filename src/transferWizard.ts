// transferWizard — the pure layer behind the Desktop↔Terminal import wizard
// (the Terminal counterpart of the cmux wizard's importWizard.ts).
//
// The Terminal wizard is NOT the cmux pipeline: the PO decided (2026-11-22)
// the move keeps FULL fidelity — split trees, per-panel directories, tab
// shells all survive — so the scoped selection is a whole-store doc, and the
// merge itself stays in ONE place: store_core::transfer::merge_states (the
// same implementation the CLI rides). This module only FILTERS the source
// doc down to the checked categories; the wizard then hands the filtered doc
// to the backend preview command, whose answer (the merged planned state +
// display rows) is exactly what Apply lands — preview and apply share one
// computation, the same invariant the cmux wizard holds.
//
// A source state is a raw store doc: serde serializes camelCase, so it is
// the persisted subset of the TS shapes — { workspaces, groups, order },
// without any runtime-only records. This module never touches I/O and never
// mutates its input.

import type { ImportScope } from './importWizard'
import type { Group, Panel, Tab, Workspace } from './workspaces'

export type TransferSourceState = {
  workspaces: Workspace[]
  groups: Group[]
  order: string[]
}

/// Filter a raw store doc down to the checked categories. Pure — the same
/// category semantics as the cmux wizard's scopeImportPlan, applied to FULL
/// definitions instead of the plan shape:
/// - `workspaces` off → no rows (groups checked alone import as containers);
/// - `grouping` off → groups dropped, rows lose their groupId (key hygiene:
///   the key must not reappear on ungrouped rows);
/// - `directories` off → every panel's workingDirectory stripped (sshTarget
///   is NOT a directory — it stays);
/// - `tabsLayout` off → each workspace keeps only its FIRST tab, stripped of
///   its name but WITH its split layout (the fidelity rule), shell and color
///   included; a workspace with no tabs stays with none.
export function scopeWorkspaceState(
  state: TransferSourceState,
  scope: ImportScope,
): TransferSourceState {
  const workspaces = (scope.workspaces ? state.workspaces : []).map((w) => {
    const next: Workspace = { ...w }
    if (!scope.grouping) {
      delete next.groupId
    }
    if (!scope.directories && next.panels != null) {
      next.panels = next.panels.map((p: Panel) =>
        p.workingDirectory != null ? { ...p, workingDirectory: undefined } : p,
      )
    }
    if (!scope.tabsLayout && next.tabs != null && next.tabs.length > 0) {
      const first: Tab = { ...next.tabs[0] }
      delete first.name
      next.tabs = [first]
    }
    return next
  })
  const groups = scope.grouping ? state.groups : []
  const alive = new Set([
    ...workspaces.map((w) => w.id),
    ...groups.map((g) => g.id),
  ])
  return {
    workspaces,
    groups,
    order: state.order.filter((id) => alive.has(id)),
  }
}
