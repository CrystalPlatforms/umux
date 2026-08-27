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
import { matchShortcut, activeTagOf, type ShortcutEvent } from './shortcuts'

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

  // --- Pane zoom (#40 / story 48): Ctrl+Shift+Z expands the focused panel --
  // The letter Z was free in the table; plain Ctrl+Z stays with the shell.

  it('maps Ctrl+Shift+Z to toggle-zoom', () => {
    expect(matchShortcut(ev('z', { ctrlKey: true, shiftKey: true }))).toBe(
      'toggle-zoom',
    )
  })

  // #40 HITL follow-up: WebKit on macOS maps Ctrl+Shift+letter combos to
  // text-input characters (e.g. ⌃⇧F reports key 'ƒ'), so the letter is also
  // matched by the PHYSICAL key (`code`, e.g. 'KeyZ') — layout- and
  // modifier-independent, the way VS Code does it.
  it('matches the physical key when macOS reports an exotic key', () => {
    const cs = { ctrlKey: true, shiftKey: true }
    expect(matchShortcut(ev('ƒ', { ...cs, code: 'KeyZ' }))).toBe('toggle-zoom')
    expect(matchShortcut(ev('≈', { ...cs, code: 'KeyN' }))).toBe('new-workspace')
  })

  it('an exotic key without a code still matches nothing', () => {
    // The code fallback is a second chance, not a bypass: no modifiers, no
    // match — and an exotic key alone never matches either.
    expect(matchShortcut(ev('ƒ', { ctrlKey: true, shiftKey: true }))).toBeNull()
    expect(matchShortcut(ev('z', { code: 'KeyZ' }))).toBeNull()
    expect(
      matchShortcut(ev('z', { ctrlKey: true, shiftKey: true, code: 'KeyX' })),
    ).toBe('toggle-zoom') // the real `key` wins over a mismatched code
  })

  it('leaves plain Ctrl+Z (shell suspend) untouched', () => {
    // Both modifiers are required (AC2): without Shift the combo belongs to
    // the terminal — Ctrl+Z suspends the foreground job.
    expect(matchShortcut(ev('z', { ctrlKey: true }))).toBeNull()
    expect(matchShortcut(ev('Z', { ctrlKey: true }))).toBeNull()
  })

  it('does not claim Ctrl+Shift+Z with Alt or Meta held', () => {
    expect(
      matchShortcut(ev('z', { ctrlKey: true, shiftKey: true, altKey: true })),
    ).toBeNull()
    expect(
      matchShortcut(ev('z', { ctrlKey: true, shiftKey: true, metaKey: true })),
    ).toBeNull()
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

  // --- Settings shortcut (#39 follow-up): Cmd+, opens Settings (macOS) -----

  it('maps Cmd+, (Meta+Comma) to open-settings', () => {
    expect(matchShortcut(ev(',', { metaKey: true }))).toBe('open-settings')
  })

  it('does not claim Ctrl+, / plain comma / other Cmd combos', () => {
    // Ctrl+, stays free for the terminal; a bare comma is typing.
    expect(matchShortcut(ev(',', { ctrlKey: true }))).toBeNull()
    expect(matchShortcut(ev(','))).toBeNull()
    // Meta with anything else keeps falling through.
    expect(matchShortcut(ev('n', { metaKey: true }))).toBeNull()
  })

  it('returns null for Cmd+, while editing text', () => {
    expect(matchShortcut(ev(',', { metaKey: true, activeTag: 'INPUT' }))).toBeNull()
  })
})

// #40 HITL follow-up: after clicking INTO a terminal, focus sits in xterm's
// hidden helper textarea — a TEXTAREA in tag name only. Treating it like a
// text field silenced every shortcut; it IS the terminal, so shortcuts must
// stay live there.
describe('activeTagOf', () => {
  it("treats xterm's helper textarea as the terminal (no suppression)", () => {
    const ta = document.createElement('textarea')
    ta.className = 'xterm-helper-textarea'
    expect(activeTagOf(ta)).toBeNull()
  })

  it('still suppresses real text fields and passes everything else through', () => {
    expect(activeTagOf(document.createElement('input'))).toBe('INPUT')
    expect(activeTagOf(document.createElement('textarea'))).toBe('TEXTAREA')
    expect(activeTagOf(document.createElement('div'))).toBe('DIV')
    expect(activeTagOf(null)).toBeNull()
  })
})
