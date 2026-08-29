// cmuxImport — unit suite (#54, Phase 8).
//
// The importer is PURE: files arrive as strings, the pipeline is
// parse → plan → apply, and apply composes the existing tree ops. These
// tests run against the SANITIZED fixtures (src/fixtures/) derived from the
// PO's real cmux capture — names are placeholders, paths are
// ~/Documents/project-X, and only the fields the importer reads survive
// sanitization. The real cmux files never enter the repo.
//
// Assumptions encoded:
//  - The session store is the PRIMARY source (workspaces, order, panels,
//    groups, membership); cmux.json `actions` of type "workspace" are
//    secondary — matching titles are skipped, non-matching ones import as
//    extra FLAT workspaces. A missing session store means a flat import.
//  - Per-entry damage is skipped; per-file damage (bad JSON, wrong top-level
//    shape) throws a clear error naming the file — BEFORE apply runs.
//  - Apply is additive: colliding names get ` from cmux` (numbered further
//    when taken), nothing overwrites, imported workspaces stay CLOSED, and
//    the target's runtime-only records (open set, activation) are restored.
//  - The read-only proof: the source STRINGS are identical after a full
//    parse + apply — the module performs no I/O and never mutates its input.

import { describe, it, expect } from 'vitest'
import {
  parseCmuxSources,
  applyImportPlan,
  type CmuxImportPlan,
} from './cmuxImport'
import { emptyState, createWorkspace, flattenSidebar } from './workspaces'
import { leafIds } from './PaneLayout'
import sessionFixture from './fixtures/cmux-session.json'
import configFixture from './fixtures/cmux-config.json'

const sessionText = JSON.stringify(sessionFixture)
const configText = JSON.stringify(configFixture)

const sessionTabManager = (
  sessionFixture as {
    windows: Array<{
      tabManager: {
        workspaces: Array<{ workspaceId: string; customTitle: string }>
        workspaceGroups: Array<{ id: string; anchorWorkspaceId: string }>
      }
    }>
  }
).windows[0].tabManager

/// The titles of the session's HIDDEN group-anchor workspaces (cmux backs
/// every group header with one — the importer must skip them).
const anchoredTitles = new Set(
  sessionTabManager.workspaces
    .filter((w) =>
      sessionTabManager.workspaceGroups.some((g) => g.anchorWorkspaceId === w.workspaceId),
    )
    .map((w) => w.customTitle),
)

/// Deterministic id generator over a fixed sequence (tests stay pure).
function seq(...ids: string[]): () => string {
  let i = 0
  return () => ids[i++]
}

/// A counter generator — the importer consumes MANY ids (one per group, four
/// more per workspace, plus split ids per extra panel), so tests that apply
/// the whole fixture need unique ids forever, not a fixed sequence.
function counterGen(): () => string {
  let n = 0
  return () => `id-${++n}`
}

describe('parseCmuxSources (#54)', () => {
  it('parses the sanitized fixture: session workspaces, flat groups, membership, flags', () => {
    // Session store only: the source-order and membership assertions below
    // describe the SESSION's shape; the config's extra action workspace is
    // covered by the merge test beneath.
    const plan = parseCmuxSources(null, sessionText)

    // Every REAL session workspace imports, in SOURCE (array) order — the
    // hidden group-anchor rows ("Grupa N" in the real capture) do not.
    const sourceTitles = sessionTabManager.workspaces.map((w) => w.customTitle)
    expect(anchoredTitles.size).toBe(3) // the fixture mirrors the real capture
    expect(plan.workspaces.map((w) => w.title)).toEqual(
      sourceTitles.filter((t) => !anchoredTitles.has(t)),
    )

    // Groups are FLAT, with their flags carried over.
    expect(plan.groups.map((g) => [g.name, g.collapsed, g.pinned])).toEqual([
      ['Group One', true, false],
      ['Group Two', false, false],
      ['Group Three', true, false],
    ])

    // Membership: every workspace whose groupId names a group counts as its
    // member; the union of members + flat workspaces = all workspaces.
    const memberCount = plan.groups.reduce((n, g) => n + g.memberIds.length, 0)
    expect(memberCount + countFlat(plan)).toBe(plan.workspaces.length)
    // 9 grouped entries in the raw capture, minus the 3 hidden anchors.
    expect(memberCount).toBe(6)
  })

  it('a missing session store means a FLAT plan from cmux.json actions alone', () => {
    const plan = parseCmuxSources(configText, null)

    expect(plan.groups).toEqual([])
    expect(plan.workspaces.length).toBe(
      Object.keys(configFixture.actions).length,
    )
    // Each declared surface becomes its own TAB, carrying the action's cwd.
    const twoSurface = plan.workspaces.find((w) => w.tabs.length === 2)
    expect(twoSurface).toBeDefined()
    expect(twoSurface!.tabs[0].directory).toBe(twoSurface!.cwd)
    expect(twoSurface!.tabs.map((t) => t.name)).toEqual(['Panel 1', 'Panel 2'])
  })

  it('merges BOTH files: session titles win, non-colliding actions import flat', () => {
    const plan = parseCmuxSources(configText, sessionText)

    // No duplicate titles: every action whose name matches a session
    // workspace was skipped; the ONE action title the session does not have
    // ("Project N") is appended as an extra flat workspace.
    const titles = plan.workspaces.map((w) => w.title)
    expect(new Set(titles).size).toBe(titles.length)
    expect(titles).toContain('Project N')
    expect(plan.workspaces).toHaveLength(
      sessionTabManager.workspaces.length - anchoredTitles.size + 1,
    )
  })

  it("a group's ANCHOR workspace (the hidden header row) does not import", () => {
    const session = {
      windows: [
        {
          tabManager: {
            workspaceGroups: [
              {
                id: 'g-1',
                name: 'Group',
                isCollapsed: false,
                isPinned: false,
                anchorWorkspaceId: 'w-anchor',
              },
            ],
            workspaces: [
              // The hidden header row cmux keeps for the group's sidebar…
              { workspaceId: 'w-anchor', customTitle: 'Grupa 1', groupId: 'g-1' },
              // …and a real member.
              { workspaceId: 'w-real', customTitle: 'Real', groupId: 'g-1' },
            ],
          },
        },
      ],
    }
    const plan = parseCmuxSources(null, JSON.stringify(session))

    // Only the real workspace imports, and it stays the group's member.
    expect(plan.workspaces.map((w) => w.title)).toEqual(['Real'])
    expect(plan.groups[0].memberIds).toEqual(['w-real'])
  })

  it('a workspace with a stale groupId (no matching group) imports FLAT', () => {
    const session = {
      windows: [
        {
          tabManager: {
            workspaceGroups: [{ id: 'g-real', name: 'Group', isCollapsed: false, isPinned: false }],
            workspaces: [
              { workspaceId: 'w-1', customTitle: 'A', groupId: 'g-gone' },
              { workspaceId: 'w-2', customTitle: 'B', groupId: 'g-real' },
            ],
          },
        },
      ],
    }
    const plan = parseCmuxSources(null, JSON.stringify(session))

    expect(plan.groups).toHaveLength(1)
    expect(plan.groups[0].memberIds).toEqual(['w-2']) // only the real link
    expect(plan.workspaces).toHaveLength(2) // the stale-linked one still imports
  })

  it('malformed input throws a clear error naming the file', () => {
    expect(() => parseCmuxSources('{not json', null)).toThrow(/cmux\.json.*not valid JSON/)
    expect(() => parseCmuxSources(null, '[1,2]')).toThrow(
      /cmux session store.*expected an object/,
    )
    expect(() => parseCmuxSources('{"actions": 42}', null)).toThrow(
      /cmux\.json.*"actions" must be an object/,
    )
    expect(() => parseCmuxSources(null, '{"windows": 7}')).toThrow(
      /cmux session store.*"windows" must be an array/,
    )
  })

  it('both files absent yields an empty plan', () => {
    expect(parseCmuxSources(null, null)).toEqual({ workspaces: [], groups: [] })
  })
})

function countFlat(plan: CmuxImportPlan): number {
  const members = new Set(plan.groups.flatMap((g) => g.memberIds))
  return plan.workspaces.filter((w) => !members.has(w.id)).length
}

describe('applyImportPlan (#54)', () => {
  it('creates the imported groups and workspaces in the live tree', () => {
    const plan = parseCmuxSources(configText, sessionText)
    const next = applyImportPlan(emptyState, plan, counterGen())

    const groups = next.groups.map((g) => g.name)
    expect(groups).toContain('Group One')
    expect(groups).toContain('Group Two')
    expect(groups).toContain('Group Three')
    // All 14 workspaces (13 session + 1 extra action) exist…
    expect(next.workspaces).toHaveLength(plan.workspaces.length)
    // …every non-flat one is filed into the group its plan entry names.
    const byPlanTitle = new Map(plan.workspaces.map((w) => [w.title, w]))
    const memberOf = new Map<string, string>()
    for (const g of plan.groups) for (const m of g.memberIds) {
      const t = plan.workspaces.find((w) => w.id === m)!.title
      memberOf.set(t, g.name)
    }
    for (const ws of next.workspaces) {
      const expectedGroup = memberOf.get(ws.name)
      if (expectedGroup == null) continue
      const group = next.groups.find((g) => g.id === ws.groupId)
      expect(group?.name).toBe(expectedGroup)
      // (byPlanTitle asserts the fixture actually covered this workspace.)
      expect(byPlanTitle.has(ws.name)).toBe(true)
    }
    // Collapsed flags rode along.
    expect(next.groups.find((g) => g.name === 'Group One')?.collapsed).toBe(true)
    expect(next.groups.find((g) => g.name === 'Group Two')?.collapsed).toBeUndefined()
  })

  it('imports workspaces CLOSED and keeps the target active — runtime records restored', () => {
    let target = emptyState
    target = createWorkspace(target, 'existing', seq('ws-x', 't-x', 'p-x'))
    const plan = parseCmuxSources(configText, sessionText)

    const next = applyImportPlan(target, plan, counterGen())

    // Only the pre-existing workspace is open/active; the import added
    // definitions only.
    expect(next.openIds).toEqual(['ws-x'])
    expect(next.activeId).toBe('ws-x')
    expect(next.activeTabId).toEqual(target.activeTabId)
  })

  it('colliding names get the ` from cmux` suffix; nothing is overwritten', () => {
    let target = emptyState
    target = createWorkspace(target, 'Project A', seq('ws-a', 't-a', 'p-a'))
    const plan = parseCmuxSources(null, sessionText)

    const next = applyImportPlan(target, plan, counterGen())

    // The original survives untouched…
    const original = next.workspaces.find((w) => w.id === 'ws-a')
    expect(original?.name).toBe('Project A')
    // …and the import landed under the suffixed name.
    expect(next.workspaces.some((w) => w.name === 'Project A from cmux')).toBe(true)
    expect(next.workspaces).toHaveLength(plan.workspaces.length + 1)
  })

  it('a doubly-taken name gets a numbered ` from cmux` suffix', () => {
    let target = emptyState
    target = createWorkspace(target, 'Solo', seq('ws-1', 't-1', 'p-1'))
    target = createWorkspace(target, 'Solo from cmux', seq('ws-2', 't-2', 'p-2'))
    const plan: CmuxImportPlan = {
      workspaces: [{ id: 'w', title: 'Solo', cwd: null, tabs: [{ name: null, directory: null }] }],
      groups: [],
    }

    const next = applyImportPlan(target, plan, seq('ws-3', 't-3', 'p-3'))

    expect(next.workspaces.some((w) => w.name === 'Solo from cmux 2')).toBe(true)
  })

  it('one NAMED umux tab per cmux surface, each carrying its own directory (HITL fix)', () => {
    const plan: CmuxImportPlan = {
      workspaces: [
        {
          id: 'w',
          title: 'Two surfaces',
          cwd: '~/Documents/project-a',
          tabs: [
            { name: '✳ Agent', directory: '~/Documents/project-a' },
            { name: 'Localhost', directory: '~/Documents/project-b' },
            { name: null, directory: '~/Documents/project-c' },
          ],
        },
      ],
      groups: [],
    }

    const next = applyImportPlan(
      emptyState,
      plan,
      seq('ws-n', 't-seed', 'leaf-1', 'tab-2', 'leaf-2', 'tab-3', 'leaf-3'),
    )

    const ws = next.workspaces[0]
    // THREE tabs — not three splits in one tab.
    expect(ws.tabs).toHaveLength(3)
    // Named from cmux; the untitled surface KEEPS the positional name addTab
    // gave it at creation — stable, and exactly umux's default naming.
    expect(ws.tabs!.map((t) => t.name)).toEqual(['✳ Agent', 'Localhost', 'Tab 3'])
    // Every tab is a single panel, and each carries its surface's directory.
    for (const tab of ws.tabs!) expect(leafIds(tab.layout)).toHaveLength(1)
    const dirs = ws.panels!.map((p) => p.workingDirectory)
    expect(dirs).toEqual([
      '~/Documents/project-a',
      '~/Documents/project-b',
      '~/Documents/project-c',
    ])
  })

  it('a workspace with no surfaces imports one unnamed tab with the workspace cwd', () => {
    const plan: CmuxImportPlan = {
      workspaces: [{ id: 'w', title: 'Bare', cwd: '~/Documents/x', tabs: [] }],
      groups: [],
    }

    const next = applyImportPlan(emptyState, plan, seq('ws-n', 't-n', 'leaf-1'))

    const ws = next.workspaces[0]
    expect(ws.tabs).toHaveLength(1)
    expect(ws.tabs![0].name).toBe('Tab 1')
    expect(ws.panels![0].workingDirectory).toBe('~/Documents/x')
  })

  it('the source STRINGS are byte-identical after a full parse + apply (read-only proof)', () => {
    const configBefore = configText
    const sessionBefore = sessionText
    const plan = parseCmuxSources(configText, sessionText)
    applyImportPlan(emptyState, plan, counterGen())

    expect(configText).toBe(configBefore)
    expect(sessionText).toBe(sessionBefore)
    expect(configText.length).toBe(configBefore.length)
    expect(sessionText.length).toBe(sessionBefore.length)
  })

  it('an empty plan returns the SAME state object — nothing to do', () => {
    const state = emptyState
    expect(applyImportPlan(state, { workspaces: [], groups: [] })).toBe(state)
  })

  it('the imported tree renders: groups lead, members indent under them', () => {
    const plan = parseCmuxSources(null, sessionText)
    const next = applyImportPlan(emptyState, plan, counterGen())

    const display = flattenSidebar(next)
    expect(display.length).toBeGreaterThan(0)
    // Every grouped workspace renders exactly one level deeper than its group.
    for (const entry of display) {
      if (entry.kind !== 'workspace' || entry.workspace.groupId == null) continue
      const parentDepth = display.find(
        (e) => e.kind === 'group' && e.group.id === entry.workspace.groupId,
      )
      expect(parentDepth).toBeDefined()
      expect(entry.depth).toBe((parentDepth as { depth: number }).depth + 1)
    }
  })
})
