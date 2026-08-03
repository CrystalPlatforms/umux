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
  | { kind: 'split'; orientation: Orientation; ratio: number }

export function createLayout(): PaneLayout {
  return { kind: 'single' }
}

/// Split `layout` in the given orientation at a half-and-half ratio. Returns
/// the new split layout, or `null` when the layout is already split — two
/// panels is the maximum (story 16). `null` means "rejected", not an error.
export function split(
  layout: PaneLayout,
  orientation: Orientation,
): PaneLayout | null {
  if (layout.kind === 'split') return null
  return { kind: 'split', orientation, ratio: 0.5 }
}

export type Rect = { x: number; y: number; width: number; height: number }
export type Container = { width: number; height: number }

/// Sensible minimum panel size in pixels along the split axis (story 19) —
/// roughly enough for a usable terminal (~10 columns at an 8px cell). Used as
/// the default clamp when a divider drag would otherwise collapse a panel.
export const MIN_PANEL_PX = 80

/// Compute the panel rects for `layout` inside `container`. Returns 1 rect
/// (single) or 2 (split, sized by `ratio`). Orientation convention:
///   horizontal -> side by side (left | right), vertical divider
///   vertical   -> stacked (top / bottom), horizontal divider
/// `ratio` is the fraction of the split axis given to the FIRST panel
/// (story 18); 0.5 is half-and-half.
export function geometry(layout: PaneLayout, container: Container): Rect[] {
  if (layout.kind === 'single') {
    return [{ x: 0, y: 0, width: container.width, height: container.height }]
  }
  if (layout.orientation === 'horizontal') {
    const first = container.width * layout.ratio
    return [
      { x: 0, y: 0, width: first, height: container.height },
      { x: first, y: 0, width: container.width - first, height: container.height },
    ]
  }
  const first = container.height * layout.ratio
  return [
    { x: 0, y: 0, width: container.width, height: first },
    { x: 0, y: first, width: container.width, height: container.height - first },
  ]
}

/// Length of the split axis for `layout` inside `container` — the dimension
/// the divider travels along. horizontal splits divide the width; vertical
/// splits divide the height.
function splitAxis(layout: PaneLayout, container: Container): number {
  if (layout.kind === 'single') return 0
  return layout.orientation === 'horizontal' ? container.width : container.height
}

/// Set the divider position of a split `layout` to `ratio` (story 18), clamped
/// so neither panel shrinks below `minSize` px along the split axis (story 19).
/// No-op on a single layout: there is no divider to move.
export function resize(
  layout: PaneLayout,
  ratio: number,
  container: Container,
  minSize: number = MIN_PANEL_PX,
): PaneLayout {
  if (layout.kind === 'single') return layout
  const axis = splitAxis(layout, container)
  const lo = axis > 0 ? minSize / axis : 0
  const hi = axis > 0 ? 1 - minSize / axis : 1
  return { ...layout, ratio: Math.max(lo, Math.min(hi, ratio)) }
}

/// Close one panel of a split (story 20): the remaining panel fills the
/// workspace, so the layout becomes single. A single layout has no second
/// panel to close — it is returned unchanged.
export function closePanel(layout: PaneLayout): PaneLayout {
  if (layout.kind === 'single') return layout
  return createLayout()
}
