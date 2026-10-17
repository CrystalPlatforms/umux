// tabSelection — contract tests (#106, user story 12).
//
// Assumptions encoded (stated before the first RED):
//  - A tab selection is an ordered list of tab ids INSIDE ONE workspace; the
//    module is pure and runtime-only (never persisted).
//  - Toggle (Ctrl+click; Cmd on macOS): adds when absent, removes when
//    present, never reorders the survivors.
//  - Range (Shift+click): anchor..target INCLUSIVE in the bar's CURRENT
//    display order, whichever end sits first; the range REPLACES the
//    selection (browser file-list semantics). Without a usable anchor (no
//    prior selection, or the anchor is gone from the bar) the range is just
//    the clicked tab.
//  - Clear (Esc / plain click): empty list.
//  - NOT tested here: the WorkspaceState mutations over a selection
//    (moveTabGroup / moveTabsToWorkspace / closeTabs — co-located tests in
//    workspaces.test.ts) and the DOM wiring (WorkspaceShell glue, HITL per
//    #106).

import { describe, it, expect } from 'vitest'
import {
  toggleTabInSelection,
  rangeTabSelection,
  clearTabSelection,
} from './tabSelection'

// One workspace's tab bar, in display order.
const BAR = ['t1', 't2', 't3', 't4', 't5']

describe('toggleTabInSelection', () => {
  it('adds a tab to an empty selection', () => {
    expect(toggleTabInSelection([], 't2')).toEqual(['t2'])
  })

  it('toggles a picked tab back off, keeping the others in order', () => {
    expect(toggleTabInSelection(['t1', 't2', 't4'], 't2')).toEqual(['t1', 't4'])
  })

  it('appends newly picked tabs in pick order', () => {
    expect(toggleTabInSelection(['t4', 't1'], 't3')).toEqual(['t4', 't1', 't3'])
  })
})

describe('rangeTabSelection', () => {
  it('selects anchor..target inclusive, forward', () => {
    expect(rangeTabSelection(BAR, 't2', 't4')).toEqual(['t2', 't3', 't4'])
  })

  it('selects the range backward when the target sits before the anchor', () => {
    expect(rangeTabSelection(BAR, 't4', 't2')).toEqual(['t2', 't3', 't4'])
  })

  it('collapses to the clicked tab when the target is the anchor', () => {
    expect(rangeTabSelection(BAR, 't3', 't3')).toEqual(['t3'])
  })

  it('falls back to the clicked tab without an anchor', () => {
    expect(rangeTabSelection(BAR, null, 't4')).toEqual(['t4'])
  })

  it('falls back to the clicked tab when the anchor is no longer in the bar', () => {
    expect(rangeTabSelection(BAR, 'gone', 't4')).toEqual(['t4'])
  })
})

describe('clearTabSelection', () => {
  it('returns an empty list', () => {
    expect(clearTabSelection()).toEqual([])
  })
})
