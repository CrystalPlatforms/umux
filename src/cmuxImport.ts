// cmuxImport — pure, group-aware importer from cmux (#54, Phase 8).
//
// Reads TWO cmux sources (as captured from the PO's machine, 2026-08-28):
//   - the session store (`~/Library/Application Support/cmux/session-*.json`)
//     — the PRIMARY source: it holds the live workspaces (customTitle,
//     currentDirectory, panels, groupId, isPinned), their sidebar order
//     (array order) and the workspace groups (flat: name, isCollapsed,
//     isPinned);
//   - cmux.json (`~/.config/cmux/cmux.json`) — SECONDARY: its `actions` hold
//     workspace launch definitions (name, cwd, panel surfaces). Actions whose
//     name matches a session workspace are skipped (the session store wins);
//     the rest import as extra FLAT workspaces.
//
// The pipeline is parse → plan → apply. This module is PURE: files come in
// as STRINGS (a thin Rust command reads them read-only), nothing here touches
// I/O, and a malformed file throws a clear error BEFORE any state is touched.
// Apply composes the existing tree ops (createGroup / createWorkspace /
// addTab / renameTab / moveNode / upsertPanelCwd) and:
//   - never overwrites: a name that already exists in the target gets a
//     ` from cmux` suffix (numbered further when even that is taken);
//   - imports workspaces CLOSED: definitions land, but the open set, the
//     activation and the per-tab runtime records are restored to the
//     target's originals — shells spawn only when the user opens a row;
//   - maps cmux's flat groups onto the tree: membership by groupId, stale
//     groupIds import at top level (the same hygiene bootState applies).

import { leafIds } from './PaneLayout'
import {
  createGroup,
  createWorkspace,
  addTab,
  renameTab,
  moveNode,
  upsertPanelCwd,
  defaultGenId,
  type WorkspaceState,
} from './workspaces'

/// One cmux surface → ONE umux tab (HITL fix 2026-08-29): cmux's surfaces are
/// the workspace's separate terminals — Adam's "tabs" — so each imports as
/// its own tab (single panel), NAMED from the surface's title, carrying its
/// own directory. A missing title imports the tab unnamed (positional
/// "Tab N", umux's default).
export type CmuxImportTab = { name: string | null; directory: string | null }

export type CmuxImportWorkspace = {
  /// Source id — references group membership; NOT the umux id umux assigns.
  id: string
  title: string
  cwd: string | null
  tabs: CmuxImportTab[]
}

export type CmuxImportGroup = {
  /// Source id — workspaces reference it; NOT the umux id umux assigns.
  id: string
  name: string
  collapsed: boolean
  pinned: boolean
  memberIds: string[]
}

export type CmuxImportPlan = {
  workspaces: CmuxImportWorkspace[]
  groups: CmuxImportGroup[]
}

// --- parse ------------------------------------------------------------------

/// Parse the two source FILES (as strings; null = file absent) into an
/// import plan (#54). Throws a clear Error naming the offending file when a
/// PRESENT file is not valid JSON or its top-level shape is wrong — state is
/// never touched, because apply only runs after a successful parse.
/// Per-ENTRY damage is tolerated (a non-workspace action, a workspace without
/// a title, a group without an id — skipped), per-FILE damage is not.
export function parseCmuxSources(
  configJson: string | null,
  sessionJson: string | null,
): CmuxImportPlan {
  const config = parseJsonFile(configJson, 'cmux.json')
  const session = parseJsonFile(sessionJson, 'cmux session store')

  // --- cmux.json: workspace actions (secondary source) ----------------------
  const actionWorkspaces: CmuxImportWorkspace[] = []
  if (config != null) {
    if (typeof config !== 'object' || Array.isArray(config)) {
      throw new Error('cmux.json is malformed: expected an object at the top level')
    }
    const actions = (config as Record<string, unknown>)['actions']
    if (actions != null) {
      if (typeof actions !== 'object' || Array.isArray(actions)) {
        throw new Error('cmux.json is malformed: "actions" must be an object')
      }
      let extra = 0
      for (const value of Object.values(actions as Record<string, unknown>)) {
        if (value == null || typeof value !== 'object') continue
        const entry = value as Record<string, unknown>
        if (entry['type'] !== 'workspace') continue
        const ws = entry['workspace']
        if (ws == null || typeof ws !== 'object') continue
        const wsRec = ws as Record<string, unknown>
        const name = wsRec['name']
        if (typeof name !== 'string' || name === '') continue
        const pane = (wsRec['layout'] as Record<string, unknown> | null | undefined)?.['pane']
        const paneSurfaces = (pane as Record<string, unknown> | null | undefined)?.['surfaces']
        // Each declared surface becomes its own TAB, named from the surface.
        const surfaces: Array<{ name: string | null }> = Array.isArray(paneSurfaces)
          ? paneSurfaces.map((s) => ({
              name:
                s != null &&
                typeof s === 'object' &&
                typeof (s as Record<string, unknown>)['name'] === 'string' &&
                ((s as Record<string, unknown>)['name'] as string).trim() !== ''
                  ? ((s as Record<string, unknown>)['name'] as string).trim()
                  : null,
            }))
          : [{ name: null }]
        const cwd = typeof wsRec['cwd'] === 'string' ? (wsRec['cwd'] as string) : null
        actionWorkspaces.push({
          id: `action-${extra++}`,
          title: name,
          cwd,
          tabs: surfaces.map((s) => ({ name: s.name, directory: cwd })),
        })
      }
    }
  }

  // --- session store: workspaces + flat groups (primary source) -------------
  const workspaces: CmuxImportWorkspace[] = []
  const groups: CmuxImportGroup[] = []
  const groupIds = new Set<string>()
  const sessionTitles = new Set<string>()
  // cmux backs every group's header row with a hidden ANCHOR workspace
  // (HITL 2026-08-29: "Grupa 1…N" appearing inside imported groups — the PO
  // has no such workspaces in cmux). The group's `anchorWorkspaceId` names
  // it, so those entries are collected and SKIPPED: they are the group
  // header itself, which the group node already represents.
  const anchorIds = new Set<string>()
  const staleLinks: Array<{ wsId: string; groupId: string }> = []
  if (session != null) {
    if (typeof session !== 'object' || Array.isArray(session)) {
      throw new Error(
        'cmux session store is malformed: expected an object at the top level',
      )
    }
    const windows = (session as Record<string, unknown>)['windows']
    if (windows != null && !Array.isArray(windows)) {
      throw new Error('cmux session store is malformed: "windows" must be an array')
    }
    const tm = (
      (windows as Array<Record<string, unknown>> | null | undefined)?.[0] ??
      {}
    )['tabManager'] as Record<string, unknown> | null | undefined
    if (tm != null && typeof tm !== 'object') {
      throw new Error('cmux session store is malformed: "tabManager" must be an object')
    }
    const rawWorkspaces = Array.isArray(tm?.['workspaces']) ? tm['workspaces'] : []
    const rawGroups = Array.isArray(tm?.['workspaceGroups']) ? tm['workspaceGroups'] : []

    rawGroups.forEach((rawGroup) => {
      if (rawGroup == null || typeof rawGroup !== 'object') return
      const g = rawGroup as Record<string, unknown>
      const id = g['id']
      const name = g['name']
      if (typeof id !== 'string' || typeof name !== 'string' || name === '') return
      groupIds.add(id)
      if (typeof g['anchorWorkspaceId'] === 'string') {
        anchorIds.add(g['anchorWorkspaceId'] as string)
      }
      groups.push({
        id,
        name,
        collapsed: g['isCollapsed'] === true,
        pinned: g['isPinned'] === true,
        memberIds: [],
      })
    })

    rawWorkspaces.forEach((rawWorkspace, i) => {
      if (rawWorkspace == null || typeof rawWorkspace !== 'object') return
      const w = rawWorkspace as Record<string, unknown>
      const title = w['customTitle']
      if (typeof title !== 'string' || title === '') return
      const wsId = typeof w['workspaceId'] === 'string' ? w['workspaceId'] : `session-${i}`
      // A group's anchor workspace is the hidden group header, not a real
      // workspace — importing it would surface rows the PO never had.
      if (anchorIds.has(wsId)) return
      const cwd = typeof w['currentDirectory'] === 'string' ? w['currentDirectory'] : null
      const rawPanels = Array.isArray(w['panels']) ? w['panels'] : []
      // One TAB per cmux surface (panel), named from its title (HITL fix):
      // an untitled surface imports unnamed — umux falls back to "Tab N".
      const tabs: CmuxImportTab[] =
        rawPanels.length > 0
          ? rawPanels.map((p) => {
              const rec = p != null && typeof p === 'object' ? (p as Record<string, unknown>) : {}
              const rawTitle = rec['title']
              const rawDir = rec['directory']
              return {
                name:
                  typeof rawTitle === 'string' && rawTitle.trim() !== ''
                    ? rawTitle.trim()
                    : null,
                directory: typeof rawDir === 'string' ? rawDir : null,
              }
            })
          : [{ name: null, directory: cwd }]
      workspaces.push({ id: wsId, title, cwd, tabs })
      sessionTitles.add(title)
      const gid = w['groupId']
      if (typeof gid === 'string' && groupIds.has(gid)) {
        groups.find((g) => g.id === gid)?.memberIds.push(wsId)
      } else if (typeof gid === 'string') {
        // A stale groupId (no matching group definition) imports the
        // workspace FLAT — the same treatment bootState gives one on load.
        staleLinks.push({ wsId, groupId: gid })
      }
    })
  }

  // --- merge: session primary, non-colliding actions as extra flat rows -----
  let extraSeq = 0
  for (const action of actionWorkspaces) {
    if (sessionTitles.has(action.title)) continue
    workspaces.push({ ...action, id: `${action.id}-${extraSeq++}` })
  }

  return { workspaces, groups }
}

function parseJsonFile(text: string | null, label: string): unknown {
  if (text == null) return null
  try {
    return JSON.parse(text) as unknown
  } catch {
    throw new Error(`${label} is malformed: not valid JSON`)
  }
}

// --- apply ------------------------------------------------------------------

/// A name that survives collision with EVERYTHING already in `state` plus
/// the names this import has created so far: `X`, then `X from cmux`, then
/// `X from cmux 2`, `X from cmux 3`, … Nothing is ever overwritten (#54).
function uniqueName(existing: ReadonlySet<string>, base: string): string {
  if (!existing.has(base)) return base
  let n = 1
  for (;;) {
    const candidate = n === 1 ? `${base} from cmux` : `${base} from cmux ${n}`
    if (!existing.has(candidate)) return candidate
    n += 1
  }
}

/// Apply an import plan to the LIVE tree (#54): groups land flat at the top
/// level (collapsed/pinned flags carried over), workspaces land in source
/// order with ONE NAMED TAB per cmux surface (HITL fix 2026-08-29 — surfaces
/// are Adam's "tabs", not splits), each tab's panel carrying the surface's
/// directory. Members are filed into their groups, and colliding names get
/// the ` from cmux` suffix. Runtime-only records are RESTORED to the
/// target's originals at the end — imported workspaces are definitions until
/// the user opens them, and the active workspace never yanks. Returns the
/// SAME state object for an empty plan (nothing to do).
/// genId order per workspace (stable): ws id, seed tab id, seed leaf id,
/// then tab id + leaf id per extra tab — the existing ops' contract.
export function applyImportPlan(
  state: WorkspaceState,
  plan: CmuxImportPlan,
  genId: () => string = defaultGenId,
): WorkspaceState {
  if (plan.workspaces.length === 0 && plan.groups.length === 0) return state

  let next = state

  // Groups first, flat at the top level, flags carried straight onto the
  // fresh nodes (a pin reorder among freshly appended nodes is meaningless).
  const groupIdMap = new Map<string, string>()
  for (const g of plan.groups) {
    const taken = new Set(next.groups.map((x) => x.name))
    const created = createGroup(next, uniqueName(taken, g.name), genId)
    const gid = created.groups[created.groups.length - 1].id
    groupIdMap.set(g.id, gid)
    next = {
      ...created,
      groups: created.groups.map((x) =>
        x.id === gid
          ? {
              ...x,
              ...(g.collapsed ? { collapsed: true as const } : {}),
              ...(g.pinned ? { pinned: true as const } : {}),
            }
          : x,
      ),
    }
  }

  // plan ws id -> plan group id (each workspace lives in at most ONE group —
  // cmux groups are flat).
  const ownerOf = new Map<string, string>()
  for (const g of plan.groups) {
    for (const memberId of g.memberIds) ownerOf.set(memberId, g.id)
  }

  for (const w of plan.workspaces) {
    const taken = new Set(next.workspaces.map((x) => x.name))
    const created = createWorkspace(next, uniqueName(taken, w.title), genId)
    const wsId = created.workspaces[created.workspaces.length - 1].id
    next = created

    // One umux TAB per cmux surface (HITL fix 2026-08-29): the seed tab is
    // surface 1; every further surface adds a tab. Each tab is then NAMED
    // from its surface's title (an untitled one keeps umux's positional
    // "Tab N") and its single panel carries the surface's directory — the
    // workspace's currentDirectory as the fallback for surface 1 (also when
    // the source listed no surfaces at all).
    for (let i = 1; i < w.tabs.length; i++) next = addTab(next, wsId, genId)
    const imported = next.workspaces.find((x) => x.id === wsId)
    imported?.tabs?.forEach((tab, i) => {
      const source = w.tabs[i]
      const name = source?.name
      if (name != null) next = renameTab(next, wsId, tab.id, name)
      const directory =
        i === 0 ? (source?.directory ?? w.cwd) : (source?.directory ?? null)
      const leaf = leafIds(tab.layout)[0]
      if (directory != null && directory !== '' && leaf != null) {
        next = upsertPanelCwd(next, leaf, directory)
      }
    })

    const planGroupId = ownerOf.get(w.id)
    if (planGroupId != null) {
      const umuxGroupId = groupIdMap.get(planGroupId)
      if (umuxGroupId != null) next = moveNode(next, wsId, { parentId: umuxGroupId })
    }
  }

  // Restore the runtime-only records: imported workspaces stay CLOSED until
  // the user opens them, and the previously active workspace stays active.
  return {
    ...next,
    activeId: state.activeId,
    openIds: state.openIds,
    activeTabId: state.activeTabId,
    activePanelId: state.activePanelId,
    zoomedPanelId: state.zoomedPanelId,
  }
}
