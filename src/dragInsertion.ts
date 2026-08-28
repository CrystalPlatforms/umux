// dragInsertion — where does a dragged row land? (live drag & drop, round 3)
//
// Pure geometry decisions behind the sidebar's and the tab bar's POINTER
// drag: given the pointer's position and the cached rects of the rows, this
// answers WHERE the row will land and WHERE the live insertion line draws.
// No DOM, no React — trivially unit-testable with synthetic rects (jsdom has
// no layout, so the component feeds it real measurements at runtime).
//
// Sidebar model (tree-aware, same semantics as moveNode):
//   - top half of a WORKSPACE row      -> insert BEFORE it (its parent)
//   - bottom half of a WORKSPACE row   -> insert AFTER it (its parent)
//   - top 25% of a GROUP row           -> insert BEFORE it (its parent)
//   - bottom 25% of a GROUP row        -> insert AFTER it (its parent)
//   - middle of a GROUP row            -> INTO the group, appended at the end
//                                         (highlighted row instead of a line)
//   - below the last row               -> top level, appended
// Nesting (#51): a dragged GROUP may file into another group — except into
// its OWN subtree. The caller passes the dragged subtree's node ids as
// `forbiddenIds`; any landing inside them (or over those rows) is REJECTED —
// `rejected: true`, no line, no highlight — so the cursor can say "not
// allowed" and the release commits nothing.
//
// Tab bar model (one workspace's bar, #45 semantics):
//   - left half of a tab -> insert before it
//   - right half         -> insert after it (before the NEXT tab; the last
//                           tab's right half appends at the end)

/// One draggable region of the sidebar, measured at drag activation. `top`
/// /`bottom` are viewport Y coordinates; `parentId` is the row's parent
/// group (null = top level) — the workspace's `groupId`, or the group's own
/// `parentId` since #51.
export type SidebarRegion = {
  id: string
  kind: 'workspace' | 'group'
  parentId: string | null
  top: number
  bottom: number
}

/// The landing decision for a sidebar drag — feeds moveNode directly:
/// `parentId` + `beforeId` describe the insertion (beforeId null = append at
/// the end of the parent), `intoGroupId` is set when the pointer rests on a
/// group's middle zone (the row highlights instead of drawing a line), and
/// `lineTop` is the viewport Y for the live insertion line (null = no line).
/// `rejected` (#51) marks a landing moveNode would refuse (into the dragged
/// group's own subtree): the UI shows the not-allowed cursor and the release
/// commits nothing.
export type SidebarDrop = {
  parentId: string | null
  beforeId: string | null
  intoGroupId: string | null
  lineTop: number | null
  rejected: boolean
}

/// One tab of a tab bar, measured at drag activation. `left`/`right` are
/// viewport X coordinates.
export type TabRegion = {
  id: string
  left: number
  right: number
}

/// The landing decision for a tab drag: insert before `beforeId` (null =
/// append at the end of the bar), with the line drawn at `lineLeft`.
export type TabDrop = {
  beforeId: string | null
  lineLeft: number
}

/// Decide where a sidebar drag landing at viewport Y `y` would land. The
/// regions must describe the visible rows in any order; they are sorted by
/// position here. `forbiddenIds` (#51) are the node ids a landing must
/// refuse — the dragged GROUP's own subtree (itself, its descendant groups,
/// and every workspace under them); a landing inside them comes back
/// `rejected`. Workspaces pass an empty set (nothing is forbidden).
export function computeSidebarDrop(
  y: number,
  // Kept in the signature for the callers/tests' sake — the forbidden set
  // (which already contains the dragged id) does the deciding.
  _draggedId: string,
  regions: SidebarRegion[],
  forbiddenIds: ReadonlySet<string> = new Set(),
): SidebarDrop {
  const sorted = [...regions].sort((a, b) => a.top - b.top)
  // A landing inside the dragged group's own subtree is forbidden: skip
  // those rows entirely so no line ever promises a moveNode would reject,
  // and surface the INTO-group zone as `rejected` instead (the cursor must
  // be able to say "not allowed" over the group's middle zone).
  const candidates = sorted.filter((r) => !forbiddenIds.has(r.id))

  const before = (r: SidebarRegion): SidebarDrop => ({
    parentId: r.parentId,
    beforeId: r.id,
    intoGroupId: null,
    lineTop: r.top,
    rejected: false,
  })
  const after = (r: SidebarRegion): SidebarDrop => {
    // Insert after `r` = before its next sibling (append at the parent's end
    // when none), with the line at the row's bottom edge.
    let next: SidebarRegion | undefined
    for (const other of candidates) {
      if (other !== r && other.parentId === r.parentId && other.top >= r.bottom) {
        if (next == null || other.top < next.top) next = other
      }
    }
    return {
      parentId: r.parentId,
      beforeId: next?.id ?? null,
      intoGroupId: null,
      lineTop: r.bottom,
      rejected: false,
    }
  }
  const rejectedDrop = (): SidebarDrop => ({
    parentId: null,
    beforeId: null,
    intoGroupId: null,
    lineTop: null,
    rejected: true,
  })

  // The pointer resting ON a forbidden row — the dragged group itself or any
  // row of its own subtree (#51) — is rejected outright: the cursor says
  // "not allowed" and no line pretends a landing the tree would refuse.
  const hovered = sorted.find((r) => y >= r.top && y <= r.bottom)
  if (hovered != null && forbiddenIds.has(hovered.id)) return rejectedDrop()

  for (const r of candidates) {
    if (y < r.top) return before(r)
    if (y <= r.bottom) {
      const mid = (r.top + r.bottom) / 2
      if (r.kind === 'group') {
        const quarter = (r.bottom - r.top) / 4
        if (y < r.top + quarter) return before(r)
        if (y > r.bottom - quarter) return after(r)
        // The middle zone FILES into the group — unless the group is inside
        // the dragged group's own subtree (#51: no dropping a group into
        // itself or its descendants).
        if (forbiddenIds.has(r.id)) return rejectedDrop()
        return {
          parentId: null,
          beforeId: null,
          intoGroupId: r.id,
          lineTop: null,
          rejected: false,
        }
      }
      return y < mid ? before(r) : after(r)
    }
  }
  // Below every row: top level, appended at the end. The line rests at the
  // bottom of the last VISIBLE row (children included — the pointer is past
  // everything, so the honest line is at the very end of the list).
  const last = sorted[sorted.length - 1]
  return {
    parentId: null,
    beforeId: null,
    intoGroupId: null,
    lineTop: last?.bottom ?? null,
    rejected: false,
  }
}

/// Decide where a tab drag landing at viewport X `x` would land.
export function computeTabDrop(x: number, regions: TabRegion[]): TabDrop {
  const sorted = [...regions].sort((a, b) => a.left - b.left)
  for (const r of sorted) {
    if (x <= r.right) {
      const mid = (r.left + r.right) / 2
      if (x < mid) {
        return { beforeId: r.id, lineLeft: r.left }
      }
      // After `r` = before the next tab, appending when none follows.
      let next: TabRegion | undefined
      for (const other of sorted) {
        if (other !== r && other.left >= r.right) {
          if (next == null || other.left < next.left) next = other
        }
      }
      return {
        beforeId: next?.id ?? null,
        lineLeft: next?.left ?? r.right,
      }
    }
  }
  // Past every tab: append at the end, line at the last tab's right edge.
  const last = sorted[sorted.length - 1]
  return {
    beforeId: null,
    lineLeft: last?.right ?? 0,
  }
}
