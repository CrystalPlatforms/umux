// shortcuts — pure keyboard mapping (Phase 11 / #12).
//
// Tests behavior through the module's only public function `matchShortcut`.
// No mocks: the module is pure. The keyboard event is modeled as a plain
// shape { key, ctrlKey, altKey, shiftKey, metaKey } plus the focused element
// type, so tests don't need a live DOM.
//
// Assumptions encoded (stated before the first RED):
//  - Input: a keyboard event shape + the active element tag name (or null
//    when no element / focus is on <body>).
//  - Output: one of the Command literals, or null when the combo matches no
//    shortcut, lacks the Ctrl+Shift modifier pair, or focus is in a text
//    field (<input>/<textarea> — typing a workspace name must not fire
//    shortcuts).
//  - Boundary: Ctrl+Shift is REQUIRED on every letter shortcut, so plain
//    Ctrl+letter combos (claimed by the shell: Ctrl+C/D/Z/L/...) are left
//    untouched and reach the terminal (AC3 — no conflict).
//  - NOT tested here: the window keydown listener / capture-phase wiring
//    (that lives in WorkspaceShell and is verified manually).

import { describe, it, expect } from 'vitest'
import { matchShortcut, type ShortcutEvent } from './shortcuts'

// Helper: build an event with only the bits that matter, defaulting to a
// non-text focused element (the terminal surface / body).
function ev(
  key: string,
  overrides: Partial<ShortcutEvent> = {},
): ShortcutEvent {
  return {
    key,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    activeTag: 'BODY',
    ...overrides,
  }
}

describe('matchShortcut', () => {
  it('maps Ctrl+Shift+N to new-workspace', () => {
    expect(matchShortcut(ev('n', { ctrlKey: true, shiftKey: true }))).toBe(
      'new-workspace',
    )
  })

  it('maps the split / close shortcuts', () => {
    const cs = { ctrlKey: true, shiftKey: true }
    expect(matchShortcut(ev('h', cs))).toBe('split-horizontal')
    expect(matchShortcut(ev('v', cs))).toBe('split-vertical')
    expect(matchShortcut(ev('w', cs))).toBe('close-panel')
  })

  it('maps Ctrl+Shift+arrows to next/prev workspace', () => {
    const cs = { ctrlKey: true, shiftKey: true }
    expect(matchShortcut(ev('ArrowRight', cs))).toBe('next-workspace')
    expect(matchShortcut(ev('ArrowLeft', cs))).toBe('prev-workspace')
  })

  it('returns null without Ctrl+Shift so shell shortcuts reach the terminal', () => {
    // Plain Ctrl+C / Ctrl+D etc. must fall through (AC3 — no conflict).
    expect(matchShortcut(ev('c', { ctrlKey: true }))).toBeNull()
    expect(matchShortcut(ev('n', { shiftKey: true }))).toBeNull()
    expect(matchShortcut(ev('n', {}))).toBeNull()
  })

  it('returns null when Alt or Meta is held', () => {
    expect(
      matchShortcut(ev('n', { ctrlKey: true, shiftKey: true, altKey: true })),
    ).toBeNull()
    expect(
      matchShortcut(ev('n', { ctrlKey: true, shiftKey: true, metaKey: true })),
    ).toBeNull()
  })

  it('returns null while editing text (input / textarea)', () => {
    const cs = { ctrlKey: true, shiftKey: true, activeTag: 'INPUT' }
    expect(matchShortcut(ev('n', cs))).toBeNull()
    expect(matchShortcut(ev('w', { ...cs, activeTag: 'TEXTAREA' }))).toBeNull()
  })

  it('returns null for an unrecognized Ctrl+Shift key', () => {
    expect(matchShortcut(ev('x', { ctrlKey: true, shiftKey: true }))).toBeNull()
  })
})
