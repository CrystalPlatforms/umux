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
//   - top 25% of a GROUP row           -> insert BEFORE it (top level)
//   - bottom 25% of a GROUP row        -> insert AFTER it (top level)
//   - middle of a GROUP row            -> INTO the group, appended at the end
//                                         (highlighted row instead of a line)
//   - below the last row               -> top level, appended
// A GROUP being dragged can never land inside a group: its middle-zone over
// another group falls back to before/after, and child regions are skipped
// entirely (the line only rests at top-level boundaries).
//
// Tab bar model (one workspace's bar, #45 semantics):
//   - left half of a tab -> insert before it
//   - right half         -> insert after it (before the NEXT tab; the last
//                           tab's right half appends at the end)

/// One draggable region of the sidebar, measured at drag activation. `top`
/// /`bottom` are viewport Y coordinates; `parentId` is the workspace's group
/// (null = top level; groups always sit at top level).
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
export type SidebarDrop = {
  parentId: string | null
  beforeId: string | null
  intoGroupId: string | null
  lineTop: number | null
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
/// position here.
export function computeSidebarDrop(
  y: number,
  draggedId: string,
  regions: SidebarRegion[],
): SidebarDrop {
  const sorted = [...regions].sort((a, b) => a.top - b.top)
  // A dragged GROUP only ever reorders at top level: skip the children of
  // groups so the line never promises a landing moveNode would reject.
  const dragged = sorted.find((r) => r.id === draggedId)
  const candidates =
    dragged?.kind === 'group' ? sorted.filter((r) => r.parentId == null) : sorted

  const before = (r: SidebarRegion): SidebarDrop => ({
    parentId: r.parentId,
    beforeId: r.id,
    intoGroupId: null,
    lineTop: r.top,
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
    }
  }

  for (const r of candidates) {
    if (y < r.top) return before(r)
    if (y <= r.bottom) {
      const mid = (r.top + r.bottom) / 2
      if (r.kind === 'group') {
        // A dragged group cannot FILE into a group: split the middle zone.
        if (dragged?.kind === 'group') {
          return y < mid ? before(r) : after(r)
        }
        const quarter = (r.bottom - r.top) / 4
        if (y < r.top + quarter) return before(r)
        if (y > r.bottom - quarter) return after(r)
        return {
          parentId: null,
          beforeId: null,
          intoGroupId: r.id,
          lineTop: null,
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
