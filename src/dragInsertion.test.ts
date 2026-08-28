// dragInsertion contract tests (live pointer drag & drop, round 3).
//
// Pure geometry: pointer position + measured row rects -> landing decision
// + line position. No DOM. These encode the interaction Adam asked for: a
// LIVE line that shows where the row will land, for the sidebar tree AND
// the tab bar.
//
// Assumptions encoded:
//  - Sidebar: top half of a workspace row = before it; bottom half = after
//    it (its parent's sibling slot). Group rows split top/bottom 25% with a
//    middle INTO zone (highlight, no line). Below every row = top level,
//    appended. Nesting (#51): a dragged GROUP files into any group EXCEPT
//    its own subtree — the caller passes the dragged subtree's ids as
//    `forbiddenIds`; landings on/inside those rows come back `rejected`
//    (no line, no target) so the cursor can say "not allowed".
//  - "After X" resolves to beforeId = X's next sibling (null when X is the
//    last sibling), and the line draws at X's bottom edge.
//  - Tabs: left half = before, right half = after (next tab; the last
//    tab's right half appends), line at the boundary's X.

import { describe, it, expect } from 'vitest'
import {
  computeSidebarDrop,
  computeTabDrop,
  type SidebarRegion,
  type TabRegion,
} from './dragInsertion'

const ws = (id: string, top: number, bottom: number, parentId: string | null = null): SidebarRegion => ({
  id,
  kind: 'workspace',
  parentId,
  top,
  bottom,
})
const group = (id: string, top: number, bottom: number, parentId: string | null = null): SidebarRegion => ({
  id,
  kind: 'group',
  parentId,
  top,
  bottom,
})
const tab = (id: string, left: number, right: number): TabRegion => ({ id, left, right })

describe('computeSidebarDrop', () => {
  // Layout: [ws-2 (top level)] [group g-1] [ws-1 (inside g-1)]
  const regions: SidebarRegion[] = [
    ws('ws-2', 0, 30),
    group('g-1', 30, 60),
    ws('ws-1', 60, 90, 'g-1'),
  ]

  it('top half of a workspace row inserts BEFORE it', () => {
    const drop = computeSidebarDrop(10, 'ws-1', regions)

    expect(drop).toEqual({
      parentId: null,
      beforeId: 'ws-2',
      intoGroupId: null,
      lineTop: 0,
      rejected: false,
    })
  })

  it('bottom half of a workspace row inserts AFTER it (its next sibling)', () => {
    // ws-2's bottom half -> after ws-2 = before g-1 (its top-level sibling).
    const drop = computeSidebarDrop(20, 'ws-1', regions)

    expect(drop).toEqual({
      parentId: null,
      beforeId: 'g-1',
      intoGroupId: null,
      lineTop: 30,
      rejected: false,
    })
  })

  it('"after the last sibling" resolves to append (beforeId null)', () => {
    // Bottom half of ws-1 (the last row of g-1) -> after ws-1 = appended at
    // the end of g-1.
    const drop = computeSidebarDrop(80, 'ws-2', regions)

    expect(drop.parentId).toBe('g-1')
    expect(drop.beforeId).toBeNull()
    expect(drop.lineTop).toBe(90)
  })

  it('middle of a group row lands INTO the group (highlight, no line)', () => {
    const drop = computeSidebarDrop(45, 'ws-2', regions)

    expect(drop).toEqual({
      parentId: null,
      beforeId: null,
      intoGroupId: 'g-1',
      lineTop: null,
      rejected: false,
    })
  })

  it('group edges keep before/after: top quarter before, bottom quarter after', () => {
    const top = computeSidebarDrop(32, 'ws-2', regions)
    expect(top.beforeId).toBe('g-1')
    expect(top.lineTop).toBe(30)

    const bottom = computeSidebarDrop(58, 'ws-2', regions)
    // After g-1 at top level = append (no top-level sibling follows).
    expect(bottom.parentId).toBeNull()
    expect(bottom.beforeId).toBeNull()
    expect(bottom.lineTop).toBe(60)
  })

  it('below the last row lands at TOP LEVEL, appended', () => {
    const drop = computeSidebarDrop(120, 'g-1', regions)

    expect(drop).toEqual({
      parentId: null,
      beforeId: null,
      intoGroupId: null,
      lineTop: 90,
      rejected: false,
    })
  })

  it('a dragged GROUP files into an unrelated group (#51)', () => {
    const twoGroups: SidebarRegion[] = [
      group('g-1', 0, 30),
      group('g-2', 30, 60),
      ws('ws-9', 60, 90, 'g-2'),
    ]

    // Drag g-1 (forbidden = itself) over g-2's middle zone — it NESTS.
    const drop = computeSidebarDrop(45, 'g-1', twoGroups, new Set(['g-1']))

    expect(drop).toEqual({
      parentId: null,
      beforeId: null,
      intoGroupId: 'g-2',
      lineTop: null,
      rejected: false,
    })
  })

  it('a dragged GROUP may reorder inside another group\'s children (#51)', () => {
    const twoGroups: SidebarRegion[] = [
      group('g-1', 0, 30),
      group('g-2', 30, 60),
      ws('ws-9', 60, 90, 'g-2'),
    ]

    // y=75 sits inside g-2's child region — a legal internal reorder slot
    // for the dragged group (after ws-9 = appended at the end of g-2).
    const drop = computeSidebarDrop(75, 'g-1', twoGroups, new Set(['g-1']))

    expect(drop.parentId).toBe('g-2')
    expect(drop.beforeId).toBeNull()
    expect(drop.rejected).toBe(false)
  })

  it('a landing on/inside the dragged group\'s OWN subtree is REJECTED (#51)', () => {
    // g-2 nested inside g-1 (g-1 → g-2 → ws-9), dragged g-1's forbidden set
    // covers itself, its child group and its child workspace.
    const nested: SidebarRegion[] = [
      group('g-1', 0, 30),
      group('g-2', 30, 60, 'g-1'),
      ws('ws-9', 60, 90, 'g-2'),
      group('g-3', 90, 120),
    ]
    const forbidden = new Set(['g-1', 'g-2', 'ws-9'])

    // Middle zone of the dragged group's own child → rejected.
    const intoChild = computeSidebarDrop(45, 'g-1', nested, forbidden)
    expect(intoChild.rejected).toBe(true)
    expect(intoChild.intoGroupId).toBeNull()
    expect(intoChild.lineTop).toBeNull()

    // Pointer resting ON the dragged row itself → rejected too.
    const ontoSelf = computeSidebarDrop(15, 'g-1', nested, forbidden)
    expect(ontoSelf.rejected).toBe(true)

    // Pointer over the forbidden grandchild row → rejected, no line.
    const ontoDescendant = computeSidebarDrop(75, 'g-1', nested, forbidden)
    expect(ontoDescendant.rejected).toBe(true)

    // The unrelated group g-3 stays fully available, edges included.
    const ontoOther = computeSidebarDrop(105, 'g-1', nested, forbidden)
    expect(ontoOther.rejected).toBe(false)
    expect(ontoOther.intoGroupId).toBe('g-3')
  })

  it('an empty list answers a top-level append with no line', () => {
    const drop = computeSidebarDrop(10, 'ws-1', [])

    expect(drop).toEqual({
      parentId: null,
      beforeId: null,
      intoGroupId: null,
      lineTop: null,
      rejected: false,
    })
  })
})

describe('computeTabDrop', () => {
  const regions: TabRegion[] = [
    tab('tab-a', 0, 100),
    tab('tab-b', 100, 200),
    tab('tab-c', 200, 300),
  ]

  it('left half of a tab inserts before it', () => {
    expect(computeTabDrop(40, regions)).toEqual({ beforeId: 'tab-a', lineLeft: 0 })
  })

  it('right half inserts after it — before the next tab', () => {
    expect(computeTabDrop(150, regions)).toEqual({ beforeId: 'tab-c', lineLeft: 200 })
  })

  it('right half of the LAST tab appends at the end', () => {
    expect(computeTabDrop(280, regions)).toEqual({ beforeId: null, lineLeft: 300 })
  })

  it('past every tab appends at the end', () => {
    expect(computeTabDrop(500, regions)).toEqual({ beforeId: null, lineLeft: 300 })
  })
})
