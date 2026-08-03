// PaneLayout — pure split state + geometry for ≤2 panels (Phase 9 / #10).
//
// Deep module: tiny interface over the workspace panel layout. No I/O, no
// React — trivially unit-testable. Mirrors the OscParser / workspaces philosophy.
//
// Assumptions encoded here (expensive to reverse — they shape every RED→GREEN):
//   Input:
//     - Orientation is exactly 'horizontal' | 'vertical'.
//     - A PaneLayout is either { kind:'single' } or
//       { kind:'split'; orientation; ratio }, where `ratio` in [0,1] is the
//       fraction of the split axis given to the FIRST panel (left/top).
//   Output:
//     - createLayout() -> single.
//     - split(layout, orientation) -> a new PaneLayout at ratio 0.5, OR null
//       when the layout is already split (two-panel cap, story 16). null =
//       "rejected", not a thrown error.
//     - geometry(layout, container) -> Rect[] of length 1 (single) or 2 (split),
//       sized by `ratio` along the split axis.
//     - resize(layout, ratio, container, minSize?) -> a layout whose ratio is
//       clamped to [minSize/axis, 1 - minSize/axis] so neither panel is below
//       the minimum (stories 18/19). No-op on a single layout.
//     - closePanel(layout) -> single (story 20: the remaining panel fills).
//   Boundaries:
//     - Splitting an already-split layout always returns null.
//     - Empty/default layout is single.
//     - Default minimum panel size is MIN_PANEL_PX (px along the split axis).
//   NOT tested this iteration:
//     - layout persistence (story 37), keyboard shortcuts (33). Later phases.

import { describe, it, expect } from 'vitest'
import { createLayout, split, geometry, resize, closePanel, MIN_PANEL_PX, type PaneLayout } from './PaneLayout'

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
  it('splits a single panel horizontally at a half-and-half ratio', () => {
    expect(split(createLayout(), 'horizontal')).toEqual({
      kind: 'split',
      orientation: 'horizontal',
      ratio: 0.5,
    })
  })

  // T3 (AC story 17 — split vertically):
  //   Input:  single layout, orientation 'vertical'.
  //   Output: split layout with orientation 'vertical'.
  it('splits a single panel vertically at a half-and-half ratio', () => {
    expect(split(createLayout(), 'vertical')).toEqual({
      kind: 'split',
      orientation: 'vertical',
      ratio: 0.5,
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

  // T8 (geometry — ratio drives the split, horizontal):
  //   Input:  horizontal split with ratio 0.25, 100x50 container.
  //   Output: first rect 25 wide (left), second 75 wide (right) — full height.
  it('sizes a horizontal split by ratio along the width', () => {
    const layout: PaneLayout = { kind: 'split', orientation: 'horizontal', ratio: 0.25 }
    expect(geometry(layout, { width: 100, height: 50 })).toEqual([
      { x: 0, y: 0, width: 25, height: 50 },
      { x: 25, y: 0, width: 75, height: 50 },
    ])
  })

  // T9 (geometry — ratio drives the split, vertical):
  //   Input:  vertical split with ratio 0.4, 100x50 container.
  //   Output: first rect 20 tall (top), second 30 tall (bottom) — full width.
  it('sizes a vertical split by ratio along the height', () => {
    const layout: PaneLayout = { kind: 'split', orientation: 'vertical', ratio: 0.4 }
    expect(geometry(layout, { width: 100, height: 50 })).toEqual([
      { x: 0, y: 0, width: 100, height: 20 },
      { x: 0, y: 20, width: 100, height: 30 },
    ])
  })

  // --- resize (stories 18/19): drag the divider, clamped to a minimum -----

  // T10 (resize — mid-range ratio passes through unchanged):
  //   Input:  horizontal split, requested ratio 0.5, 100x50 container, min 10.
  //   Output: layout with ratio 0.5 (within [0.1, 0.9], no clamp).
  it('keeps a mid-range ratio unchanged on resize', () => {
    const layout: PaneLayout = { kind: 'split', orientation: 'horizontal', ratio: 0.5 }
    expect(resize(layout, 0.5, { width: 100, height: 50 }, 10)).toEqual({
      kind: 'split',
      orientation: 'horizontal',
      ratio: 0.5,
    })
  })

  // T11 (resize — clamps the low end so the first panel never collapses):
  //   Input:  horizontal split, requested ratio 0, 100x50 container, min 10.
  //   Output: ratio clamped up to 0.1 (10/100) — first panel keeps min width.
  it('clamps the divider so the first panel stays above the minimum', () => {
    const layout: PaneLayout = { kind: 'split', orientation: 'horizontal', ratio: 0.5 }
    expect(resize(layout, 0, { width: 100, height: 50 }, 10)).toEqual({
      kind: 'split',
      orientation: 'horizontal',
      ratio: 0.1,
    })
  })

  // T12 (resize — clamps the high end so the second panel never collapses):
  //   Input:  horizontal split, requested ratio 1, 100x50 container, min 10.
  //   Output: ratio clamped down to 0.9 (1 - 10/100) — second panel keeps min.
  it('clamps the divider so the second panel stays above the minimum', () => {
    const layout: PaneLayout = { kind: 'split', orientation: 'horizontal', ratio: 0.5 }
    expect(resize(layout, 1, { width: 100, height: 50 }, 10)).toEqual({
      kind: 'split',
      orientation: 'horizontal',
      ratio: 0.9,
    })
  })

  // T13 (resize — no-op on a single layout: there is no divider to move):
  //   Input:  single layout, any ratio/container.
  //   Output: the same single layout, unchanged.
  it('leaves a single layout unchanged when resized', () => {
    expect(resize(createLayout(), 0.3, { width: 100, height: 50 })).toEqual({
      kind: 'single',
    })
  })

  // --- closePanel (story 20): close one panel, the other fills ------------

  // T14 (closePanel — split collapses to single so the survivor fills):
  //   Input:  a split layout.
  //   Output: single — geometry() then gives the whole container to the
  //   remaining panel.
  it('collapses a split to a single panel on close', () => {
    const layout: PaneLayout = { kind: 'split', orientation: 'horizontal', ratio: 0.3 }
    expect(closePanel(layout)).toEqual({ kind: 'single' })
  })

  // T15 (closePanel — single stays single):
  //   Input:  a single layout (no second panel to close).
  //   Output: single — no-op.
  it('leaves a single layout as single on close', () => {
    expect(closePanel(createLayout())).toEqual({ kind: 'single' })
  })

  // T16 (resize — default minimum applies when minSize is omitted):
  //   Input:  horizontal split, requested ratio 0, 800x400 container, NO min.
  //   Output: ratio clamped to MIN_PANEL_PX/800 = 0.1.
  it('clamps to the default minimum when minSize is omitted', () => {
    const layout: PaneLayout = { kind: 'split', orientation: 'horizontal', ratio: 0.5 }
    expect(resize(layout, 0, { width: 800, height: 400 })).toEqual({
      kind: 'split',
      orientation: 'horizontal',
      ratio: MIN_PANEL_PX / 800,
    })
  })
})
