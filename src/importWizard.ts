// importWizard — the pure layer behind the cmux import wizard (#59, Phase 2).
//
// The wizard in Settings (#59) is a THREE-step flow over the EXISTING,
// untouched import pipeline (parse → plan → apply in cmuxImport.ts):
//   1. SCOPE  — the user picks categories (workspaces + order, grouping,
//      working directories, tabs + layout);
//   2. PREVIEW — the scoped plan is APPLIED to the live state by the same
//      pure `applyImportPlan` the one-shot import uses, and the newly created
//      rows are shown (collision-suffixed names included). Nothing is
//      persisted: applyImportPlan is pure, so this is a dry run;
//   3. APPLY — the very state computed at preview time is handed back to the
//      shell, which persists it. Because preview and apply share ONE
//      computation, "Apply produces exactly the previewed result" holds by
//      construction — there is no second pass that could drift.
//
// This module never touches I/O and never mutates its inputs.

import {
  applyImportPlan,
  type CmuxImportPlan,
} from './cmuxImport'
import type { WorkspaceState } from './workspaces'

/// The four import categories (story 62): categories only — per-workspace
/// selection is explicitly out of scope. `workspaces` covers the rows and
/// their source order; `grouping` the group nodes and membership;
/// `directories` every imported cwd (workspace + per-tab); `tabsLayout` the
/// named/extra tabs (off = each workspace lands with ONE default unnamed
/// tab — the pre-import default).
export type ImportScope = {
  workspaces: boolean
  grouping: boolean
  directories: boolean
  tabsLayout: boolean
}

/// The wizard opens with every category checked (full import = what the
/// one-shot button does).
export const fullImportScope: ImportScope = {
  workspaces: true,
  grouping: true,
  directories: true,
  tabsLayout: true,
}

/// Filter a parsed plan down to the checked categories (story 62). Pure —
/// the source plan is never mutated, so the wizard can re-scope freely while
/// the user flips checkboxes.
///
/// Category interplay:
/// - `tabsLayout` off → each workspace imports with ZERO planned tabs, which
///   `applyImportPlan` renders as the workspace's single seed tab (positional
///   "Tab N"). This is also why directories die with it: no tabs, no per-tab
///   directories.
/// - `directories` off → the workspace cwd AND every tab's directory are
///   stripped, so nothing lands in a panel's workingDirectory.
/// - `grouping` off → groups and membership disappear; workspaces import
///   flat at the top level.
/// - `workspaces` off → no rows import; group nodes checked alone would
///   import as empty containers.
export function scopeImportPlan(plan: CmuxImportPlan, scope: ImportScope): CmuxImportPlan {
  const workspaces = (scope.workspaces ? plan.workspaces : []).map((w) => ({
    ...w,
    cwd: scope.directories ? w.cwd : null,
    tabs: !scope.tabsLayout
      ? []
      : scope.directories
        ? w.tabs
        : w.tabs.map((t) => ({ name: t.name, directory: null })),
  }))
  const groups = scope.grouping ? plan.groups : []
  return { workspaces, groups }
}

export type ImportPreviewWorkspace = {
  /// The FINAL name the import will create — collision suffix (` from cmux`)
  /// already resolved, so what the user previews is what lands.
  name: string
  /// Final name of the group the workspace files into, or null when flat.
  groupName: string | null
  /// How many tabs the imported workspace will have.
  tabCount: number
}

export type ImportPreviewGroup = {
  /// The final (collision-resolved) group name.
  name: string
  /// How many imported workspaces file into this group.
  childCount: number
}

export type ImportPreview = {
  /// The exact state Apply will persist — computed ONCE, here.
  planned: WorkspaceState
  workspaces: ImportPreviewWorkspace[]
  groups: ImportPreviewGroup[]
}

/// One row of the wizard's preview TREE (HITL rework 2026-08-30): a group
/// header with its member workspaces nested beneath (rendered INDENTED), or
/// a flat workspace at top level. Groups come first in plan order; flat
/// workspaces follow in source order.
export type ImportPreviewNode =
  | {
      kind: 'group'
      name: string
      childCount: number
      children: Array<{ name: string; tabCount: number }>
    }
  | { kind: 'workspace'; name: string; tabCount: number }

/// Shape the flat preview rows into the sidebar-like tree the wizard draws:
/// each group carries its children (matched by final group name), and
/// group-less workspaces sit at the top level — the same picture the
/// workspace tree will show after Apply.
export function buildImportPreviewTree(preview: ImportPreview): ImportPreviewNode[] {
  const nodes: ImportPreviewNode[] = []
  for (const g of preview.groups) {
    const children = preview.workspaces
      .filter((w) => w.groupName === g.name)
      .map((w) => ({ name: w.name, tabCount: w.tabCount }))
    nodes.push({ kind: 'group', name: g.name, childCount: children.length, children })
  }
  for (const w of preview.workspaces) {
    if (w.groupName == null) {
      nodes.push({ kind: 'workspace', name: w.name, tabCount: w.tabCount })
    }
  }
  return nodes
}

/// Dry-run the scoped plan against the LIVE state (#59): the pure
/// `applyImportPlan` produces the would-be state, and the rows it would ADD
/// (ids absent from the live state) are summarized for the wizard's preview
/// list. Nothing is persisted — persistence happens only when the caller
/// hands `preview.planned` back for real (the dialog keeps this object, so
/// Apply persists the previewed state verbatim).
export function buildImportPreview(
  live: WorkspaceState,
  scopedPlan: CmuxImportPlan,
): ImportPreview {
  const planned = applyImportPlan(live, scopedPlan)
  const liveWsIds = new Set(live.workspaces.map((w) => w.id))
  const liveGroupIds = new Set(live.groups.map((g) => g.id))
  const groupNameOf = (groupId: string | undefined) =>
    groupId != null
      ? (planned.groups.find((g) => g.id === groupId)?.name ?? null)
      : null
  const workspaces = planned.workspaces
    .filter((w) => !liveWsIds.has(w.id))
    .map((w) => ({
      name: w.name,
      groupName: groupNameOf(w.groupId),
      tabCount: w.tabs?.length ?? 0,
    }))
  const groups = planned.groups
    .filter((g) => !liveGroupIds.has(g.id))
    .map((g) => ({
      name: g.name,
      childCount: planned.workspaces.filter((w) => w.groupId === g.id).length,
    }))
  return { planned, workspaces, groups }
}

/// True on Windows — the ONE platform the v1.2.0 import is hidden on
/// (discovery decision #4; cmux's own files were never observed there).
/// Same navigator.platform reading the macOS multi-select check uses.
export function isWindowsPlatform(): boolean {
  return (
    typeof navigator !== 'undefined' && /Win/i.test(navigator.platform)
  )
}
