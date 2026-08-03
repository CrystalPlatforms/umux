// PaneLayout — pure split state + geometry for ≤2 panels (Phase 9 / #10).
//
// Deep module: tiny interface over the workspace panel layout. No I/O, no
// React — trivially unit-testable. Mirrors the OscParser / workspaces philosophy.
//
// Assumptions encoded here (expensive to reverse — they shape every RED→GREEN):
//   Input:
//     - Orientation is exactly 'horizontal' | 'vertical'.
//     - A PaneLayout is either { kind:'single' } or
//       { kind:'split'; orientation }.
//   Output:
//     - createLayout() -> single.
//     - split(layout, orientation) -> a new PaneLayout, OR null when the layout
//       is already split (two-panel cap, story 16). null = "rejected", not a
//       thrown error.
//     - geometry(layout, container) -> Rect[] of length 1 (single) or 2 (split),
//       each panel taking half along the split axis.
//   Boundaries:
//     - Splitting an already-split layout always returns null.
//     - Empty/default layout is single.
//   NOT tested this iteration:
//     - drag-resize + ratio (story 18), min-size snap (19), close-fills (20),
//       layout persistence (story 37), keyboard shortcuts (33). Later phases.

import { describe, it, expect } from 'vitest'
import { createLayout, split, geometry } from './PaneLayout'

describe('PaneLayout', () => {
  // T1 (tracer — fresh layout is a single panel):
  //   Input:  none.
  //   Output: { kind: 'single' }.
  it('creates a fresh layout as a single panel', () => {
    expect(createLayout()).toEqual({ kind: 'single' })
  })

  // T2 (AC story 17 — split horizontally):
  //   Input:  single layout, orientation 'horizontal'.
  //   Output: split layout with orientation 'horizontal'.
  it('splits a single panel horizontally', () => {
    expect(split(createLayout(), 'horizontal')).toEqual({
      kind: 'split',
      orientation: 'horizontal',
    })
  })

  // T3 (AC story 17 — split vertically):
  //   Input:  single layout, orientation 'vertical'.
  //   Output: split layout with orientation 'vertical'.
  it('splits a single panel vertically', () => {
    expect(split(createLayout(), 'vertical')).toEqual({
      kind: 'split',
      orientation: 'vertical',
    })
  })

  // T4 (AC story 16 — two-panel maximum):
  //   Input:  an already-split layout, any orientation.
  //   Output: null (rejected) — never a third panel.
  it('rejects splitting an already-split layout', () => {
    const splitLayout = split(createLayout(), 'horizontal')!

    expect(split(splitLayout, 'horizontal')).toBeNull()
    expect(split(splitLayout, 'vertical')).toBeNull()
  })

  // Orientation -> divider convention (locked here, expensive to reverse):
  //   horizontal -> panels side by side (left | right), vertical divider.
  //   vertical   -> panels stacked (top / bottom), horizontal divider.
  // Mirrors flex-row / flex-column and tmux's `-h`.

  // T5 (geometry — single panel fills the container):
  //   Input:  single layout, a 100x50 container.
  //   Output: one rect equal to the whole container.
  it('fills the whole container for a single panel', () => {
    expect(geometry(createLayout(), { width: 100, height: 50 })).toEqual([
      { x: 0, y: 0, width: 100, height: 50 },
    ])
  })

  // T6 (geometry — horizontal split, side by side):
  //   Input:  horizontal split, 100x50 container.
  //   Output: two rects each 50 wide, full height — left then right.
  it('lays out two side-by-side rects for a horizontal split', () => {
    expect(geometry(split(createLayout(), 'horizontal')!, { width: 100, height: 50 })).toEqual([
      { x: 0, y: 0, width: 50, height: 50 },
      { x: 50, y: 0, width: 50, height: 50 },
    ])
  })

  // T7 (geometry — vertical split, stacked):
  //   Input:  vertical split, 100x50 container.
  //   Output: two rects each 25 tall, full width — top then bottom.
  it('lays out two stacked rects for a vertical split', () => {
    expect(geometry(split(createLayout(), 'vertical')!, { width: 100, height: 50 })).toEqual([
      { x: 0, y: 0, width: 100, height: 25 },
      { x: 0, y: 25, width: 100, height: 25 },
    ])
  })
})
