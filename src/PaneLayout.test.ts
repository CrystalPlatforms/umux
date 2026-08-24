// PaneLayout — pure split TREE state + geometry for unlimited panels
// (v0.2 Phase 1 / #25).
//
// Deep module: tiny interface over the workspace panel layout. No I/O, no
// React — trivially unit-testable. Mirrors the OscParser / workspaces philosophy.
//
// Assumptions encoded here (expensive to reverse — they shape every RED→GREEN):
//   Input:
//     - Orientation is exactly 'horizontal' | 'vertical' (unchanged from v0.1:
//       horizontal -> side by side, vertical divider).
//     - A LayoutNode is either { kind:'leaf'; id } or
//       { kind:'split'; id; orientation; ratio; first; second }, where `ratio`
//       in [0,1] is the fraction of the split axis given to the FIRST child.
//     - Panel identity lives IN the tree (leaf ids) — not in a parallel array.
//       Split nodes carry ids too, so a divider drag can address its node.
//     - splitLeaf takes the ids for the new split node and new leaf explicitly
//       (the caller generates them) — keeps this module pure/deterministic.
//   Output:
//     - createTree(panelId) -> { kind:'leaf'; id: panelId }.
//     - splitLeaf(tree, leafId, orientation, { splitId, newLeafId }) -> new
//       tree where that leaf becomes a split (original leaf first, NEW leaf
//       second, ratio 0.5), or null when leafId is not in the tree. Unlimited
//       depth — no panel cap (v0.2 decision, supersedes the two-panel max).
//     - closeLeaf(tree, leafId) -> tree without that leaf; the sibling fills
//       the freed space and one-child splits collapse, or null when the tree
//       would become empty / leafId is unknown.
//     - setRatio(tree, splitId, ratio, container, minSize?) -> tree with that
//       split's ratio clamped to [minSize/axis, 1 - minSize/axis] where axis
//       is the split node's OWN extent along its orientation (the caller
//       passes the node's measured rect, not the workspace rect — clamping
//       must work at any nesting depth). Degenerate axis (<= 2*minSize):
//       ratio unchanged, never corrupted. Unknown splitId: tree unchanged.
//     - geometry(tree, container) -> { id, rect }[] for every leaf, in tree
//       order (first child first).
//     - leafIds(tree) -> leaf ids in tree order (React keys / focus logic).
//   Boundaries:
//     - Splitting any leaf any number of times always succeeds.
//     - Closing the LAST leaf returns null ("would be empty" — caller no-ops).
//     - Default minimum panel size is MIN_PANEL_PX (px along the split axis).
//   NOT tested this iteration:
//     - persistence (covered by workspace_store / workspaces seeding),
//     - keyboard shortcuts, drag pointer plumbing (WorkspaceShell HITL).

import { describe, it, expect } from 'vitest'
import { geometry, MIN_PANEL_PX, type LayoutNode, createTree, splitLeaf, closeLeaf, setRatio, leafIds, boxes } from './PaneLayout'

describe('PaneLayout', () => {
  // T1 (tracer — fresh tree is a single leaf carrying the panel id):
  //   Input:  a panel id.
  //   Output: { kind: 'leaf', id } — panel identity lives in the tree.
  it('creates a fresh tree as a single leaf with the panel id', () => {
    expect(createTree('p1')).toEqual({ kind: 'leaf', id: 'p1' })
  })

  // T2 (splitLeaf — original leaf first, NEW leaf second, ratio 0.5):
  //   Input:  a single-leaf tree, orientation 'horizontal', explicit ids for
  //           the new split node and the new leaf.
  //   Output: the leaf became a split; the original panel keeps its identity
  //           (and thus its shell) as the FIRST child.
  it('splits a leaf into a split with the new leaf as the second child', () => {
    expect(splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })).toEqual({
      kind: 'split',
      id: 's1',
      orientation: 'horizontal',
      ratio: 0.5,
      first: { kind: 'leaf', id: 'p1' },
      second: { kind: 'leaf', id: 'p2' },
    })
  })

  // T3 (AC1 — split a NESTED leaf, mixed directions, no cap):
  //   Input:  p1 | p2 (horizontal), then split p2 vertically.
  //   Output: 3 leaves; the nested split replaced only the p2 leaf — p1's
  //           subtree is untouched (structural equality, not just leaf count).
  it('splits a nested leaf in a mixed direction without touching the sibling subtree', () => {
    const once = splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!
    const twice = splitLeaf(once, 'p2', 'vertical', { splitId: 's2', newLeafId: 'p3' })

    expect(twice).toEqual({
      kind: 'split',
      id: 's1',
      orientation: 'horizontal',
      ratio: 0.5,
      first: { kind: 'leaf', id: 'p1' },
      second: {
        kind: 'split',
        id: 's2',
        orientation: 'vertical',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'p2' },
        second: { kind: 'leaf', id: 'p3' },
      },
    } satisfies LayoutNode)
  })

  // T4 (boundary — unknown leaf id is rejected, not an error):
  //   Input:  a split request for a leaf id that is not in the tree.
  //   Output: null.
  it('returns null when splitting a leaf that does not exist', () => {
    const tree = splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!

    expect(splitLeaf(tree, 'nope', 'vertical', { splitId: 's2', newLeafId: 'p3' })).toBeNull()
  })

  // T5 (tree geometry — a single leaf fills the whole container, id attached):
  //   Input:  one-leaf tree, 100x50 container.
  //   Output: [{ id, rect }] with the full container rect.
  it('gives a single leaf the whole container', () => {
    expect(geometry(createTree('p1'), { width: 100, height: 50 })).toEqual([
      { id: 'p1', rect: { x: 0, y: 0, width: 100, height: 50 } },
    ])
  })

  // T6 (AC1 — geometry of a 3-panel MIXED tree, the numbers HITL will see):
  //   Input:  p1 | (p2 / p3) — root horizontal 0.5, right child vertical 0.5,
  //           100x50 container.
  //   Output: p1 fills the left half; p2/p3 stack in the right half, in tree
  //           order.
  it('computes rects for a mixed 3-panel tree', () => {
    const tree = splitLeaf(
      splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!,
      'p2',
      'vertical',
      { splitId: 's2', newLeafId: 'p3' },
    )!

    expect(geometry(tree, { width: 100, height: 50 })).toEqual([
      { id: 'p1', rect: { x: 0, y: 0, width: 50, height: 50 } },
      { id: 'p2', rect: { x: 50, y: 0, width: 50, height: 25 } },
      { id: 'p3', rect: { x: 50, y: 25, width: 50, height: 25 } },
    ])
  })

  // T7 (geometry — a nested ratio carves only that node's share):
  //   Input:  root horizontal 0.5; left child horizontal 0.4; 100x50 container.
  //   Output: the nested split divides the LEFT half (50px) into 20/30 — not
  //           the whole width — and the right half is untouched.
  it('carves a nested split within its own share only', () => {
    // Root horizontal 0.5 [p1 | p2]; the LEFT half split again at 0.4.
    const nested: LayoutNode = {
      kind: 'split',
      id: 's1',
      orientation: 'horizontal',
      ratio: 0.5,
      first: {
        kind: 'split',
        id: 's2',
        orientation: 'horizontal',
        ratio: 0.4,
        first: { kind: 'leaf', id: 'p1' },
        second: { kind: 'leaf', id: 'p3' },
      },
      second: { kind: 'leaf', id: 'p2' },
    }

    expect(geometry(nested, { width: 100, height: 50 })).toEqual([
      { id: 'p1', rect: { x: 0, y: 0, width: 20, height: 50 } },
      { id: 'p3', rect: { x: 20, y: 0, width: 30, height: 50 } },
      { id: 'p2', rect: { x: 50, y: 0, width: 50, height: 50 } },
    ])
  })

  // T8 (AC3 — closing a middle panel of 4 leaves a clean layout, no gaps):
  //   Input:  4 panels: A B / C D (root vertical; each half horizontal),
  //           close B — a middle leaf whose parent still has a sibling split.
  //   Output: B's parent collapses to just A; A widens to fill the whole top
  //           half; C/D untouched. geometry() of the result tiles the
  //           container exactly — no gap where B used to be.
  it('closes a middle panel of four and siblings fill the freed space', () => {
    // Build a 2x2 grid: root vertical [ [A | B] / [C | D] ].
    const tree = splitLeaf(
      splitLeaf(
        splitLeaf(createTree('A'), 'A', 'vertical', { splitId: 'root', newLeafId: 'C' })!,
        'A',
        'horizontal',
        { splitId: 'top', newLeafId: 'B' },
      )!,
      'C',
      'horizontal',
      { splitId: 'bottom', newLeafId: 'D' },
    )!

    const after = closeLeaf(tree, 'B')

    expect(after).toEqual({
      kind: 'split',
      id: 'root',
      orientation: 'vertical',
      ratio: 0.5,
      first: { kind: 'leaf', id: 'A' },
      second: {
        kind: 'split',
        id: 'bottom',
        orientation: 'horizontal',
        ratio: 0.5,
        first: { kind: 'leaf', id: 'C' },
        second: { kind: 'leaf', id: 'D' },
      },
    } satisfies LayoutNode)
    // And the survivors tile the 100x50 container exactly: A takes the whole
    // top half (B's gap absorbed), C/D share the bottom half.
    expect(geometry(after!, { width: 100, height: 50 })).toEqual([
      { id: 'A', rect: { x: 0, y: 0, width: 100, height: 25 } },
      { id: 'C', rect: { x: 0, y: 25, width: 50, height: 25 } },
      { id: 'D', rect: { x: 50, y: 25, width: 50, height: 25 } },
    ])
  })

  // T9 (boundary — closing the LAST panel is the caller's no-op):
  //   Input:  a single-leaf tree, closing its only leaf.
  //   Output: null — "would be empty"; the app keeps at least one panel per
  //           open workspace (close the workspace instead).
  it('returns null when closing the last remaining panel', () => {
    expect(closeLeaf(createTree('p1'), 'p1')).toBeNull()
  })

  // T10 (boundary — unknown leaf id is rejected, not an error):
  //   Input:  a tree without the requested leaf id.
  //   Output: null.
  it('returns null when closing a panel that does not exist', () => {
    const tree = splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!

    expect(closeLeaf(tree, 'nope')).toBeNull()
  })

  // --- setRatio (AC2 — min-size clamp per divider, at any nesting depth) ----

  // T11 (AC2 — a NESTED split clamps against its OWN axis, not the window):
  //   Input:  root horizontal (100px wide) with the left 50px half split
  //           horizontally again (s2); drag s2's divider to 0, passing s2's
  //           own 50px rect as container, min 10.
  //   Output: s2 ratio clamps to 10/50 = 0.2 (whole-width clamping would
  //           wrongly give 0.1); the root ratio is untouched.
  it('clamps a nested divider against its own extent, not the container', () => {
    const tree = splitLeaf(
      splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 'root', newLeafId: 'p2' })!,
      'p1',
      'horizontal',
      { splitId: 's2', newLeafId: 'p3' },
    )!

    const after = setRatio(tree, 's2', 0, { width: 50, height: 50 }, 10)

    expect(after).toEqual({
      kind: 'split',
      id: 'root',
      orientation: 'horizontal',
      ratio: 0.5,
      first: {
        kind: 'split',
        id: 's2',
        orientation: 'horizontal',
        ratio: 0.2,
        first: { kind: 'leaf', id: 'p1' },
        second: { kind: 'leaf', id: 'p3' },
      },
      second: { kind: 'leaf', id: 'p2' },
    } satisfies LayoutNode)
  })

  // T12 (boundary — degenerate axis never corrupts the ratio):
  //   Input:  a split whose axis (15px) is below 2*minSize (2*10), ratio 0.5,
  //           drag requested to 0.
  //   Output: ratio stays 0.5 — [lo, hi] is empty, so the drag is refused
  //           rather than clamped to a broken value.
  it('keeps the current ratio when the axis is below twice the minimum', () => {
    const tree = splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!

    expect(setRatio(tree, 's1', 0, { width: 15, height: 50 }, 10)).toEqual({
      kind: 'split',
      id: 's1',
      orientation: 'horizontal',
      ratio: 0.5,
      first: { kind: 'leaf', id: 'p1' },
      second: { kind: 'leaf', id: 'p2' },
    } satisfies LayoutNode)
  })

  // T13 (boundary — unknown split id changes nothing):
  //   Input:  a resize for a split id that is not in the tree.
  //   Output: the tree unchanged (deep-equal).
  it('leaves the tree unchanged when the split id is unknown', () => {
    const tree = splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!

    expect(setRatio(tree, 'nope', 0.3, { width: 100, height: 50 }, 10)).toEqual(tree)
  })

  // T14 (setRatio — default minimum applies when minSize is omitted):
  //   Input:  horizontal split, requested ratio 0, 800x400 container, NO min.
  //   Output: ratio clamped to MIN_PANEL_PX/800 = 0.1.
  it('clamps to the default minimum when minSize is omitted', () => {
    const tree = splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!

    const after = setRatio(tree, 's1', 0, { width: 800, height: 400 })

    expect(after).toEqual({
      kind: 'split',
      id: 's1',
      orientation: 'horizontal',
      ratio: MIN_PANEL_PX / 800,
      first: { kind: 'leaf', id: 'p1' },
      second: { kind: 'leaf', id: 'p2' },
    } satisfies LayoutNode)
  })

  // T15 (leafIds — panel identity in tree order for React keys / focus):
  //   Input:  the mixed 3-panel tree p1 | (p2 / p3).
  //   Output: ['p1', 'p2', 'p3'] — first child first, matching geometry().
  it('lists leaf ids in tree order', () => {
    const tree = splitLeaf(
      splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!,
      'p2',
      'vertical',
      { splitId: 's2', newLeafId: 'p3' },
    )!

    expect(leafIds(tree)).toEqual(['p1', 'p2', 'p3'])
    expect(leafIds(createTree('p1'))).toEqual(['p1'])
  })

  // --- boxes (render layout: absolutely-positioned panes + dividers) ---------
  //
  // The renderer mounts every panel ONCE (keyed by leaf id, in a stable
  // parent) and positions it absolutely — so tree-shape changes never remount
  // a terminal. boxes() computes where each pane and each divider bar sits,
  // reserving `gap` px along each split's axis for the divider.

  // T16 (boxes — a single leaf fills the container, no dividers):
  it('boxes a single leaf as the whole container with no dividers', () => {
    expect(boxes(createTree('p1'), { width: 100, height: 50 }, 6)).toEqual({
      panes: [{ id: 'p1', rect: { x: 0, y: 0, width: 100, height: 50 } }],
      dividers: [],
    })
  })

  // T17 (boxes — horizontal split: gap carved between the halves):
  //   106 wide with a 6px divider -> 100 shared, 50/50 at ratio 0.5.
  it('boxes a horizontal split with a divider between the halves', () => {
    const tree = splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 's1', newLeafId: 'p2' })!

    expect(boxes(tree, { width: 106, height: 50 }, 6)).toEqual({
      panes: [
        { id: 'p1', rect: { x: 0, y: 0, width: 50, height: 50 } },
        { id: 'p2', rect: { x: 56, y: 0, width: 50, height: 50 } },
      ],
      dividers: [
        {
          id: 's1',
          orientation: 'horizontal',
          rect: { x: 50, y: 0, width: 6, height: 50 },
          splitRect: { x: 0, y: 0, width: 106, height: 50 },
        },
      ],
    })
  })

  // T18 (boxes — vertical split stacks with a horizontal divider bar):
  it('boxes a vertical split with a divider between the rows', () => {
    const tree = splitLeaf(createTree('p1'), 'p1', 'vertical', { splitId: 's1', newLeafId: 'p2' })!

    expect(boxes(tree, { width: 100, height: 106 }, 6)).toEqual({
      panes: [
        { id: 'p1', rect: { x: 0, y: 0, width: 100, height: 50 } },
        { id: 'p2', rect: { x: 0, y: 56, width: 100, height: 50 } },
      ],
      dividers: [
        {
          id: 's1',
          orientation: 'vertical',
          rect: { x: 0, y: 50, width: 100, height: 6 },
          splitRect: { x: 0, y: 0, width: 100, height: 106 },
        },
      ],
    })
  })

  // T19 (boxes — a nested mixed tree boxes every pane and both dividers):
  //   p1 | (p2 / p3) in 106x50: right half is 50 wide and splits vertically.
  it('boxes a nested mixed tree with a divider per split node', () => {
    const tree = splitLeaf(
      splitLeaf(createTree('p1'), 'p1', 'horizontal', { splitId: 'root', newLeafId: 'p2' })!,
      'p2',
      'vertical',
      { splitId: 'nested', newLeafId: 'p3' },
    )!

    expect(boxes(tree, { width: 106, height: 50 }, 6)).toEqual({
      panes: [
        { id: 'p1', rect: { x: 0, y: 0, width: 50, height: 50 } },
        { id: 'p2', rect: { x: 56, y: 0, width: 50, height: 22 } },
        { id: 'p3', rect: { x: 56, y: 28, width: 50, height: 22 } },
      ],
      dividers: [
        {
          id: 'root',
          orientation: 'horizontal',
          rect: { x: 50, y: 0, width: 6, height: 50 },
          splitRect: { x: 0, y: 0, width: 106, height: 50 },
        },
        {
          id: 'nested',
          orientation: 'vertical',
          rect: { x: 56, y: 22, width: 50, height: 6 },
          splitRect: { x: 56, y: 0, width: 50, height: 50 },
        },
      ],
    })
  })
})
