// importWizard — unit suite (#59, Phase 2).
//
// The wizard's pure layer: scopeImportPlan filters a parsed plan to the
// checked categories, buildImportPreview dry-runs it against the live tree
// through the EXISTING applyImportPlan, and isWindowsPlatform gates the whole
// section off on Windows. These tests run against the SAME sanitized fixtures
// as cmuxImport.test.ts (src/fixtures/ — placeholders only, never the real
// cmux files).
//
// Assumptions encoded (stated before the first test):
//  - Input shapes: a CmuxImportPlan from parseCmuxSources (workspaces carry
//    title/cwd/tabs; groups carry memberIds) and a live WorkspaceState built
//    from the existing workspaces ops (emptyState + createWorkspace).
//  - scopeImportPlan NEVER mutates its input plan (the dialog re-scopes on
//    every checkbox flip).
//  - tabsLayout off → planned tabs are [] — which applyImportPlan renders as
//    each workspace's ONE default unnamed seed tab; directories off → the
//    workspace cwd AND every tab directory are stripped (names survive);
//    grouping off → no groups, flat import; workspaces off → no rows.
//  - buildImportPreview persists NOTHING: it returns the would-be state
//    (planned) plus the rows it would add; for an empty plan planned IS the
//    live state object (applyImportPlan's identity contract).
//  - Collision suffixes in the preview come from the same uniqueName pass
//    Apply uses, because preview and apply share ONE computation.
//  - NOT tested here: persistence and the Tauri read (component tests in
//    SettingsDialog.test.tsx + WorkspaceShell glue, verified manually).

import { describe, it, expect } from 'vitest'
import { parseCmuxSources, type CmuxImportPlan } from './cmuxImport'
import {
  scopeImportPlan,
  buildImportPreview,
  buildImportPreviewTree,
  fullImportScope,
  isWindowsPlatform,
} from './importWizard'
import { emptyState, createWorkspace, type WorkspaceState } from './workspaces'
import sessionFixture from './fixtures/cmux-session.json'
import configFixture from './fixtures/cmux-config.json'

const sessionText = JSON.stringify(sessionFixture)
const configText = JSON.stringify(configFixture)

/// The fixture plan: 3 groups ("Group One/Two/Three"); session workspaces
/// Project A…M (the three hidden group-anchor rows are skipped by parse) plus
/// "Project N" arriving only through cmux.json's actions.
function fixturePlan(): CmuxImportPlan {
  return parseCmuxSources(configText, sessionText)
}

/// A live tree holding one workspace with the given name.
function liveWith(name: string): WorkspaceState {
  return createWorkspace(emptyState, name)
}

describe('scopeImportPlan', () => {
  // T1 (full scope = the whole plan): every category checked changes nothing
  //   — the wizard's default view is exactly what the one-shot import does.
  it('carries the full plan through when every category is checked', () => {
    const plan = fixturePlan()
    const before = JSON.stringify(plan)

    const scoped = scopeImportPlan(plan, fullImportScope)

    expect(scoped.workspaces).toHaveLength(11) // 10 session + "Project N" (actions)
    expect(scoped.groups).toHaveLength(3)
    expect(scoped.workspaces.every((w) => w.cwd === plan.workspaces.find((p) => p.id === w.id)?.cwd)).toBe(true)
    expect(scoped.workspaces.every((w) => w.tabs.length > 0)).toBe(true)
    // The source plan is untouched — the dialog re-scopes on every flip.
    expect(JSON.stringify(plan)).toBe(before)
  })

  // T2 (AC "unchecking tabs + layout yields a plan without tabs"): tabs []
  //   on every workspace, everything else (names, cwd) preserved.
  it('drops all tabs when tabs + layout is unchecked', () => {
    const plan = fixturePlan()

    const scoped = scopeImportPlan(plan, { ...fullImportScope, tabsLayout: false })

    expect(scoped.workspaces).toHaveLength(11)
    expect(scoped.workspaces.every((w) => w.tabs.length === 0)).toBe(true)
    // Directories are not implied by the tabs switch: cwd survives so a
    // workspace definition still knows where it lives.
    expect(scoped.workspaces.every((w) => w.cwd != null)).toBe(true)
    expect(scoped.groups).toHaveLength(3)
  })

  // T3 (directories unchecked — nothing lands in a working directory):
  //   the workspace cwd AND every tab's directory are stripped; names stay.
  it('strips every directory when working directories is unchecked', () => {
    const plan = fixturePlan()

    const scoped = scopeImportPlan(plan, { ...fullImportScope, directories: false })

    expect(scoped.workspaces.every((w) => w.cwd === null)).toBe(true)
    expect(
      scoped.workspaces.every((w) => w.tabs.every((t) => t.directory === null)),
    ).toBe(true)
    expect(scoped.workspaces.every((w) => w.title !== '')).toBe(true)
  })

  // T4 (grouping unchecked — flat import): no groups, no membership; the
  //   workspaces themselves stay.
  it('drops groups and membership when grouping is unchecked', () => {
    const plan = fixturePlan()

    const scoped = scopeImportPlan(plan, { ...fullImportScope, grouping: false })

    expect(scoped.groups).toHaveLength(0)
    expect(scoped.workspaces).toHaveLength(11)
  })

  // T5 (workspaces unchecked — only the category rows remain): no workspace
  //   imports; the group list itself is still representable (empty
  //   containers). applyImportPlan decides what actually lands.
  it('drops all workspaces when workspaces + order is unchecked', () => {
    const plan = fixturePlan()

    const scoped = scopeImportPlan(plan, { ...fullImportScope, workspaces: false })

    expect(scoped.workspaces).toHaveLength(0)
    expect(scoped.groups).toHaveLength(3)
  })
})

describe('buildImportPreview', () => {
  // T6 (collision suffix visible in the preview, story 64): a live
  //   "Project A" collides with the fixture's "Project A" — the preview row
  //   (and the would-be state) already carries the ` from cmux` suffix.
  it('shows collision-suffixed names before anything is applied', () => {
    const live = liveWith('Project A')

    const preview = buildImportPreview(live, scopeImportPlan(fixturePlan(), fullImportScope))

    const names = preview.workspaces.map((w) => w.name)
    expect(names).toContain('Project A from cmux')
    expect(preview.planned.workspaces.map((w) => w.name)).toContain('Project A from cmux')
    // Exactly one new row per planned workspace; nothing was persisted
    // (planned is a fresh tree, live is untouched).
    expect(preview.workspaces).toHaveLength(11)
    expect(preview.planned).not.toBe(live)
    expect(live.workspaces.map((w) => w.name)).toEqual(['Project A'])
  })

  // T7 (empty plan — identity): nothing to import means the preview keeps
  //   the live state as-is, byte-for-byte the same object, no rows.
  it('returns the live state unchanged for an empty plan', () => {
    const live = liveWith('solo')

    const preview = buildImportPreview(live, { workspaces: [], groups: [] })

    expect(preview.planned).toBe(live)
    expect(preview.workspaces).toHaveLength(0)
    expect(preview.groups).toHaveLength(0)
  })

  // T8 (AC "tabs + layout off … Apply lands accordingly"): with the tabs
  //   category off, every imported workspace lands with exactly ONE default
  //   seed tab — carrying umux's positional default name ("Tab 1"), never a
  //   cmux surface title — and no extra tabs.
  it('lands one default tab per workspace when tabs are scoped out', () => {
    const live = emptyState

    const preview = buildImportPreview(
      live,
      scopeImportPlan(fixturePlan(), { ...fullImportScope, tabsLayout: false }),
    )

    const plannedNew = preview.planned.workspaces.filter(
      (w) => !live.workspaces.some((x) => x.id === w.id),
    )
    expect(plannedNew).toHaveLength(11)
    for (const ws of plannedNew) {
      expect(ws.tabs).toHaveLength(1)
      expect(ws.tabs?.[0].name).toBe('Tab 1')
    }
    expect(preview.workspaces.every((w) => w.tabCount === 1)).toBe(true)
  })

  // T9 (grouping preview): groups appear as their own preview rows with the
  //   member counts of the scoped plan — Group One holds C and D (its anchor
  //   row B was skipped by parse).
  it('previews groups with their member counts', () => {
    const live = emptyState

    const preview = buildImportPreview(live, scopeImportPlan(fixturePlan(), fullImportScope))

    const one = preview.groups.find((g) => g.name === 'Group One')
    expect(one).toEqual({ name: 'Group One', childCount: 2 })
    expect(preview.groups).toHaveLength(3)
    // A grouped workspace's preview row names its final group.
    const grouped = preview.workspaces.find((w) => w.name === 'Project C')
    expect(grouped?.groupName).toBe('Group One')
  })

  // T10 (story 66 — read-only proof): a full preview must not mutate the
  //   live state, the plan, or the source strings it was parsed from.
  it('leaves the live state, plan, and source strings byte-identical', () => {
    const live = liveWith('Project A')
    const plan = fixturePlan()
    const liveBefore = JSON.stringify(live)
    const planBefore = JSON.stringify(plan)
    const sessionBefore = sessionText
    const configBefore = configText

    buildImportPreview(live, scopeImportPlan(plan, fullImportScope))

    expect(JSON.stringify(live)).toBe(liveBefore)
    expect(JSON.stringify(plan)).toBe(planBefore)
    expect(sessionText).toBe(sessionBefore)
    expect(configText).toBe(configBefore)
  })
})

describe('isWindowsPlatform', () => {
  const setPlatform = (value: string) => {
    Object.defineProperty(window.navigator, 'platform', { value, configurable: true })
  }
  const restorePlatform = () => {
    // The jsdom value is a prototype getter; deleting the own property
    // restores it.
    delete (window.navigator as { platform?: string }).platform
  }

  // T11 (v1.2.0 decision #4 — Windows hides the import): Win32 reads true…
  it('is true for Win32', () => {
    setPlatform('Win32')
    try {
      expect(isWindowsPlatform()).toBe(true)
    } finally {
      restorePlatform()
    }
  })

  // T12 …and every platform that KEEPS the import reads false.
  it('is false on macOS and Linux', () => {
    setPlatform('MacIntel')
    try {
      expect(isWindowsPlatform()).toBe(false)
    } finally {
      restorePlatform()
    }
    setPlatform('Linux x86_64')
    try {
      expect(isWindowsPlatform()).toBe(false)
    } finally {
      restorePlatform()
    }
  })
})

// --- buildImportPreviewTree (HITL rework 2026-08-30) -------------------------
//
// The wizard's preview TREE: group headers carry their member workspaces
// (rendered INDENTED beneath), flat workspaces sit at top level — the same
// nesting picture the sidebar shows after Apply.
describe('buildImportPreviewTree', () => {
  // T-T1: groups first (plan order) with their children nested; flat
  //   workspaces follow in source order.
  it('shapes the preview into groups with nested children plus flat rows', () => {
    const preview = buildImportPreview(emptyState, scopeImportPlan(fixturePlan(), fullImportScope))

    const tree = buildImportPreviewTree(preview)

    const groupNodes = tree.filter((n): n is Extract<typeof n, { kind: 'group' }> => n.kind === 'group')
    const flatNodes = tree.filter((n) => n.kind === 'workspace')
    expect(groupNodes).toHaveLength(3)
    expect(groupNodes[0]).toMatchObject({
      kind: 'group',
      name: 'Group One',
      childCount: 2,
    })
    expect(groupNodes[0].children.map((c) => c.name)).toEqual(['Project C', 'Project D'])
    // Flat: session A, H, I, J (the other six sit in the three groups)
    // + "Project N" arriving through cmux.json's actions.
    expect(flatNodes).toHaveLength(5)
    expect(flatNodes.every((n) => n.kind === 'workspace' && n.tabCount > 0)).toBe(true)
  })

  // T-T2 (grouping off): no groups → every workspace renders at TOP level.
  it('flattens the tree when grouping is unchecked', () => {
    const preview = buildImportPreview(
      emptyState,
      scopeImportPlan(fixturePlan(), { ...fullImportScope, grouping: false }),
    )

    const tree = buildImportPreviewTree(preview)

    expect(tree.every((n) => n.kind === 'workspace')).toBe(true)
    expect(tree).toHaveLength(11)
  })

  // T-T3 (workspaces off): group shells remain, with zero children.
  it('keeps empty group nodes when workspaces are unchecked', () => {
    const preview = buildImportPreview(
      emptyState,
      scopeImportPlan(fixturePlan(), { ...fullImportScope, workspaces: false }),
    )

    const tree = buildImportPreviewTree(preview)

    expect(tree).toHaveLength(3)
    expect(
      tree.every(
        (n) =>
          n.kind === 'group' && n.childCount === 0 && n.children.length === 0,
      ),
    ).toBe(true)
  })
})
