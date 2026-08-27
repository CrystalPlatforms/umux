// shortcuts — pure keyboard mapping (Phase 11 / #12, story 33).
//
// Deep, pure module: maps a keyboard event to an app Command (or null). No
// I/O, no DOM listeners — those live in WorkspaceShell. The window-level
// keydown handler calls matchShortcut and, on a match, preventDefault()s and
// dispatches the command; everything else falls through to the terminal
// untouched.
//
// Conflict avoidance (AC3): every letter shortcut requires BOTH Ctrl and
// Shift, so plain Ctrl+letter combos the shell claims (Ctrl+C/D/Z/L/W on
// some shells, ...) never match here and reach the PTY. The one exception is
// Cmd+, (open-settings): the macOS preferences convention, Meta-only. Focus
// in a text field (workspace rename / create) suppresses all shortcuts so
// typing names does not trigger actions.

export type Command =
  | 'new-workspace'
  | 'new-tab'
  | 'next-workspace'
  | 'prev-workspace'
  | 'split-horizontal'
  | 'split-vertical'
  | 'close-panel'
  | 'toggle-zoom'
  | 'open-settings'

/// Shape of the keyboard event matchShortcut consumes. `activeTag` is the
/// uppercased tagName of the focused element (or 'BODY' / null when nothing
/// focusable is active) — used to suppress shortcuts while editing text (see
/// activeTagOf for how that tag is derived). `code` is the PHYSICAL key
/// (e.g. 'KeyZ'): on macOS WebKit reports Ctrl+Shift+letter as text-input
/// characters (⌃⇧F → key 'ƒ'), so the letter is matched by code as a
/// fallback — layout- and modifier-independent.
export type ShortcutEvent = {
  key: string
  code?: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
  activeTag: string | null
}

/// Literal -> Command table for the Ctrl+Shift shortcuts. Keyed by the
/// event's lowercased `key` so caps-lock / Shift state on the letter itself
/// does not matter.
const TABLE: Record<string, Command> = {
  n: 'new-workspace',
  t: 'new-tab',
  h: 'split-horizontal',
  v: 'split-vertical',
  w: 'close-panel',
  z: 'toggle-zoom',
}

// Arrow keys carry a Shift-state-independent name, so they live in their own
// table rather than the lowercased-letter one.
const ARROWS: Record<string, Command> = {
  ArrowRight: 'next-workspace',
  ArrowLeft: 'prev-workspace',
}

/// The letter of a physical key code ('KeyZ' -> 'z'), or null for anything
/// else (arrows keep their named `key`; digits/Intl keys match no shortcut).
function codeLetter(code: string | undefined): string | null {
  const m = code?.match(/^Key([A-Z])$/)
  return m != null ? m[1].toLowerCase() : null
}

/// The `activeTag` matchShortcut should see for a focused element: real text
/// fields (INPUT/TEXTAREA) suppress shortcuts — EXCEPT xterm's hidden helper
/// textarea, which is how the terminal itself holds focus. Treating it like a
/// text field silenced every shortcut the moment a panel was clicked into
/// (#40 HITL: "the keyboard shortcut does not work").
export function activeTagOf(el: Element | null): string | null {
  if (el == null) return null
  if (el.tagName === 'TEXTAREA' && el.classList.contains('xterm-helper-textarea')) {
    return null
  }
  return el.tagName
}

export function matchShortcut(e: ShortcutEvent): Command | null {
  // Typing a workspace name must never trigger a shortcut.
  if (e.activeTag === 'INPUT' || e.activeTag === 'TEXTAREA') return null
  // Cmd+, opens Settings (#39 follow-up) — the macOS preferences convention.
  // Meta ONLY: Ctrl+, and a plain comma stay free for the terminal.
  if (e.key === ',' && e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey)
    return 'open-settings'
  // Ctrl+Shift is the required modifier pair; Alt/Meta combos are left alone.
  if (!(e.ctrlKey && e.shiftKey) || e.altKey || e.metaKey) return null
  if (ARROWS[e.key] != null) return ARROWS[e.key]
  // The reported `key` wins; the physical `code` is the macOS fallback for
  // the exotic text-input characters ⌃⇧ combos produce there.
  const byKey = TABLE[e.key.toLowerCase()]
  if (byKey != null) return byKey
  const letter = codeLetter(e.code)
  return letter != null ? TABLE[letter] ?? null : null
}
