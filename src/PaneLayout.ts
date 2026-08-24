// PaneLayout — split-tree state + geometry for unlimited panels
// (v0.2 Phase 1 / #25).
//
// Deep module: a tiny interface over a workspace's panel layout. Pure, no I/O
// — the testable core of splitting. The React layer reads this to render the
// tree; the Rust WorkspaceStore persists the same shape (leaf/split, ids,
// camelCase) so the layout round-trips through restarts.

export type Orientation = 'horizontal' | 'vertical'

// A workspace's layout is a binary tree: leaves are panels (identity lives in
// the tree — no parallel id array), split nodes are dividers.

export type Leaf = { kind: 'leaf'; id: string }

export type Split = {
  kind: 'split'
  id: string
  orientation: Orientation
  ratio: number
  first: LayoutNode
  second: LayoutNode
}

export type LayoutNode = Leaf | Split

/// A fresh tree is a single leaf — one panel carrying `panelId`.
export function createTree(panelId: string): LayoutNode {
  return { kind: 'leaf', id: panelId }
}

/// Split the leaf with id `leafId` in `orientation`, replacing it with a split
/// node whose FIRST child is the original leaf (it keeps its panel id, so its
/// shell survives) and whose SECOND child is a fresh leaf `ids.newLeafId`.
/// Returns the new tree, or `null` when `leafId` is not in the tree. Unlimited
/// depth — there is no panel cap (v0.2 decision).
export function splitLeaf(
  tree: LayoutNode,
  leafId: string,
  orientation: Orientation,
  ids: { splitId: string; newLeafId: string },
): LayoutNode | null {
  if (tree.kind === 'leaf') {
    if (tree.id !== leafId) return null
    return {
      kind: 'split',
      id: ids.splitId,
      orientation,
      ratio: 0.5,
      first: tree,
      second: { kind: 'leaf', id: ids.newLeafId },
    }
  }
  const first = splitLeaf(tree.first, leafId, orientation, ids)
  if (first != null) return { ...tree, first }
  const second = splitLeaf(tree.second, leafId, orientation, ids)
  if (second != null) return { ...tree, second }
  return null
}

/// Close the leaf with id `leafId`: its sibling takes over the freed space and
/// any split left with a single child collapses (the sibling is promoted into
/// the parent's slot). Returns the new tree, or `null` when the tree would
/// become empty (the last panel) or `leafId` is not in the tree — the caller
/// no-ops on both.
export function closeLeaf(tree: LayoutNode, leafId: string): LayoutNode | null {
  if (tree.kind === 'leaf') return null
  if (tree.first.kind === 'leaf' && tree.first.id === leafId) return tree.second
  if (tree.second.kind === 'leaf' && tree.second.id === leafId) return tree.first
  if (tree.first.kind === 'split') {
    const first = closeLeaf(tree.first, leafId)
    if (first != null) return { ...tree, first }
  }
  if (tree.second.kind === 'split') {
    const second = closeLeaf(tree.second, leafId)
    if (second != null) return { ...tree, second }
  }
  return null
}

export type Rect = { x: number; y: number; width: number; height: number }
export type Container = { width: number; height: number }

/// Sensible minimum panel size in pixels along the split axis (story 19) —
/// roughly enough for a usable terminal (~10 columns at an 8px cell). Used as
/// the default clamp when a divider drag would otherwise collapse a panel.
export const MIN_PANEL_PX = 80

/// Move the divider of the split with id `splitId` to `ratio`, clamped so
/// neither side of THAT split shrinks below `minSize` px along its axis (AC2 —
/// works at any nesting depth because `container` is the split node's OWN
/// extent, not the workspace's). A degenerate axis (<= 2*minSize — e.g. a
/// very narrow window) cannot satisfy the minimum on both sides, so the
/// current ratio stands. Unknown splitId: tree unchanged.
export function setRatio(
  tree: LayoutNode,
  splitId: string,
  ratio: number,
  container: Container,
  minSize: number = MIN_PANEL_PX,
): LayoutNode {
  return setRatioIn(tree, splitId, ratio, container, minSize)
}

function setRatioIn(
  node: LayoutNode,
  splitId: string,
  ratio: number,
  container: Container,
  minSize: number,
): LayoutNode {
  if (node.kind === 'leaf') return node
  if (node.id === splitId) {
    const axis = node.orientation === 'horizontal' ? container.width : container.height
    const lo = axis > 0 ? minSize / axis : 0
    const hi = 1 - lo
    // Empty clamp range: no ratio satisfies the minimum on both sides.
    if (lo >= hi) return node
    const clamped = Math.max(lo, Math.min(hi, ratio))
    // Identity is preserved when nothing changes so callers can treat an
    // unchanged result as a no-op (=== compare).
    return clamped === node.ratio ? node : { ...node, ratio: clamped }
  }
  const first = setRatioIn(node.first, splitId, ratio, container, minSize)
  const second = setRatioIn(node.second, splitId, ratio, container, minSize)
  if (first === node.first && second === node.second) return node
  return { ...node, first, second }
}

/// Compute the panel rects for `tree` inside `container`: one { id, rect } per
/// leaf, in tree order (first child first). Orientation convention:
///   horizontal -> side by side (left | right), vertical divider
///   vertical   -> stacked (top / bottom), horizontal divider
/// `ratio` is the fraction of the split axis given to the FIRST child.
export function geometry(
  tree: LayoutNode,
  container: Container,
): Array<{ id: string; rect: Rect }> {
  return layoutRects(tree, { x: 0, y: 0, width: container.width, height: container.height })
}

/// Recursive core of geometry(): carve `rect` by the splits, left-to-right /
/// top-to-bottom in tree order.
function layoutRects(
  node: LayoutNode,
  rect: Rect,
): Array<{ id: string; rect: Rect }> {
  if (node.kind === 'leaf') return [{ id: node.id, rect }]
  if (node.orientation === 'horizontal') {
    const first = rect.width * node.ratio
    return [
      ...layoutRects(node.first, { ...rect, width: first }),
      ...layoutRects(node.second, { x: rect.x + first, y: rect.y, width: rect.width - first, height: rect.height }),
    ]
  }
  const first = rect.height * node.ratio
  return [
    ...layoutRects(node.first, { ...rect, height: first }),
    ...layoutRects(node.second, { x: rect.x, y: rect.y + first, width: rect.width, height: rect.height - first }),
  ]
}

/// Leaf ids in tree order (first child first) — the stable panel identity the
/// React layer uses as keys, and the fallback order for focus.
export function leafIds(tree: LayoutNode): string[] {
  if (tree.kind === 'leaf') return [tree.id]
  return [...leafIds(tree.first), ...leafIds(tree.second)]
}

/// Where to draw a divider bar: the `gap`-wide strip between a split's two
/// children, plus the split node's own extent (`splitRect`) — the drag math
/// and the min-size clamp both need the parent's full box, not the bar.
export type DividerBox = {
  id: string
  orientation: Orientation
  rect: Rect
  splitRect: Rect
}

export type PaneBox = { id: string; rect: Rect }

/// Render layout: absolutely-positioned pane rects + divider bars for the
/// whole tree. Each split reserves `gap` px along its axis for the divider, so
/// `ratio` splits the REMAINING space: first = (axis - gap) * ratio. Panes
/// come in tree order; dividers depth-first (a parent's bar after its first
/// subtree's bars, before its second's). The renderer keys panes by leaf id in
/// a stable parent, so tree-shape changes reposition surfaces without ever
/// remounting them (shells survive splits and closes).
export function boxes(
  tree: LayoutNode,
  container: Container,
  gap: number,
): { panes: PaneBox[]; dividers: DividerBox[] } {
  const panes: PaneBox[] = []
  const dividers: DividerBox[] = []
  boxNode(tree, { x: 0, y: 0, width: container.width, height: container.height }, panes, dividers, gap)
  return { panes, dividers }
}

function boxNode(
  node: LayoutNode,
  rect: Rect,
  panes: PaneBox[],
  dividers: DividerBox[],
  gap: number,
): void {
  if (node.kind === 'leaf') {
    panes.push({ id: node.id, rect })
    return
  }
  if (node.orientation === 'horizontal') {
    const first = (rect.width - gap) * node.ratio
    boxNode(node.first, { ...rect, width: first }, panes, dividers, gap)
    dividers.push({
      id: node.id,
      orientation: node.orientation,
      rect: { x: rect.x + first, y: rect.y, width: gap, height: rect.height },
      splitRect: rect,
    })
    boxNode(
      node.second,
      { x: rect.x + first + gap, y: rect.y, width: rect.width - first - gap, height: rect.height },
      panes,
      dividers,
      gap,
    )
    return
  }
  const first = (rect.height - gap) * node.ratio
  boxNode(node.first, { ...rect, height: first }, panes, dividers, gap)
  dividers.push({
    id: node.id,
    orientation: node.orientation,
    rect: { x: rect.x, y: rect.y + first, width: rect.width, height: gap },
    splitRect: rect,
  })
  boxNode(
    node.second,
    { x: rect.x, y: rect.y + first + gap, width: rect.width, height: rect.height - first - gap },
    panes,
    dividers,
    gap,
  )
}
