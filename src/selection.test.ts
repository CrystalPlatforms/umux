// selection — unit suite (#53).
//
// A multi-selection is an ordered list of sidebar node ids, RUNTIME-ONLY.
// These tests pin the tiny contract the sidebar builds on: toggle per row
// (add AND remove), mixed workspaces+groups (both are just node ids here —
// the distinction only matters to the batch ops in workspaces.ts), and a
// one-step clear for Escape / a background click.
//
// Assumptions encoded:
//  - Order of the surviving members is preserved on toggle — the selection's
//    order keeps the ghost label and batch resolution deterministic.
//  - The multi-select modifier is Cmd on macOS and Ctrl elsewhere, read off
//    the pointer event's modifier flags; navigator.platform is the boundary
//    (stubbed per test — an environment boundary, not our module).

import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import {
  toggleInSelection,
  clearSelection,
  isMacPlatform,
  isMultiSelectModifier,
} from './selection'

function stubPlatform(value: string): void {
  Object.defineProperty(window.navigator, 'platform', {
    value,
    configurable: true,
  })
}

const realPlatform = window.navigator.platform

describe('selection (#53)', () => {
  describe('toggleInSelection', () => {
    it('adds an absent id, preserving order', () => {
      expect(toggleInSelection(['a'], 'b')).toEqual(['a', 'b'])
    })

    it('removes a present id, keeping the rest in order', () => {
      expect(toggleInSelection(['a', 'b', 'c'], 'b')).toEqual(['a', 'c'])
    })

    it('holds MIXED workspace and group ids — both are just node ids', () => {
      const mixed = toggleInSelection(['ws-1', 'g-1'], 'ws-2')
      expect(mixed).toEqual(['ws-1', 'g-1', 'ws-2'])

      // Toggling the group out leaves the workspaces.
      expect(toggleInSelection(mixed, 'g-1')).toEqual(['ws-1', 'ws-2'])
    })

    it('toggling the same id twice returns to the original set', () => {
      const once = toggleInSelection([], 'ws-1')
      expect(toggleInSelection(once, 'ws-1')).toEqual([])
    })
  })

  describe('clearSelection', () => {
    it('returns a fresh empty list from any selection', () => {
      expect(clearSelection()).toEqual([])
      expect(clearSelection()).not.toBe(clearSelection())
    })
  })

  describe('multi-select modifier (Cmd on macOS, Ctrl elsewhere)', () => {
    beforeEach(() => {
      stubPlatform('')
    })

    afterEach(() => {
      stubPlatform(realPlatform)
    })

    it('macOS: metaKey (Cmd) toggles, plain Ctrl does NOT (it is the menu gesture)', () => {
      stubPlatform('MacIntel')
      expect(isMacPlatform()).toBe(true)
      expect(isMultiSelectModifier({ metaKey: true, ctrlKey: false })).toBe(true)
      expect(isMultiSelectModifier({ metaKey: false, ctrlKey: true })).toBe(false)
      expect(isMultiSelectModifier({ metaKey: false, ctrlKey: false })).toBe(false)
    })

    it('Linux/Windows: ctrlKey toggles, metaKey does not', () => {
      stubPlatform('Linux x86_64')
      expect(isMacPlatform()).toBe(false)
      expect(isMultiSelectModifier({ metaKey: false, ctrlKey: true })).toBe(true)
      expect(isMultiSelectModifier({ metaKey: true, ctrlKey: false })).toBe(false)
    })
  })
})
