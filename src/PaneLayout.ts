// PaneLayout — split state + geometry for ≤2 panels (Phase 9 / #10).
//
// Deep module: a tiny interface over a workspace's panel layout. Pure, no I/O
// — the testable core of splitting. The React layer reads this to decide how
// many terminals to mount and how to size them.
//
// Scope (Phase 9, stories 15–17): split creation, orientation, two-panel max.
// Resize ratio (18), min-snap (19), close-fills (20) and persistence (37) are
// later phases.

export type Orientation = 'horizontal' | 'vertical'

export type PaneLayout =
  | { kind: 'single' }
  | { kind: 'split'; orientation: Orientation }

export function createLayout(): PaneLayout {
  return { kind: 'single' }
}

/// Split `layout` in the given orientation. Returns the new split layout, or
/// `null` when the layout is already split — two panels is the maximum (story
/// 16). `null` means "rejected", not an error.
export function split(
  layout: PaneLayout,
  orientation: Orientation,
): PaneLayout | null {
  if (layout.kind === 'split') return null
  return { kind: 'split', orientation }
}

export type Rect = { x: number; y: number; width: number; height: number }
export type Container = { width: number; height: number }

/// Compute the panel rects for `layout` inside `container`. Returns 1 rect
/// (single) or 2 (split, 50/50). Orientation convention:
///   horizontal -> side by side (left | right)
///   vertical   -> stacked (top / bottom)
/// Phase 9 is a fixed half/half; resize ratio arrives in Phase 10 (story 18).
export function geometry(layout: PaneLayout, container: Container): Rect[] {
  if (layout.kind === 'single') {
    return [{ x: 0, y: 0, width: container.width, height: container.height }]
  }
  if (layout.orientation === 'horizontal') {
    const half = container.width / 2
    return [
      { x: 0, y: 0, width: half, height: container.height },
      { x: half, y: 0, width: half, height: container.height },
    ]
  }
  const half = container.height / 2
  return [
    { x: 0, y: 0, width: container.width, height: half },
    { x: 0, y: half, width: container.width, height: half },
  ]
}
