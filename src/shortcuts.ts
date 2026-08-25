// shortcuts — pure keyboard mapping (Phase 11 / #12, story 33).
//
// Deep, pure module: maps a keyboard event to an app Command (or null). No
// I/O, no DOM listeners — those live in WorkspaceShell. The window-level
// keydown handler calls matchShortcut and, on a match, preventDefault()s and
// dispatches the command; everything else falls through to the terminal
// untouched.
//
// Conflict avoidance (AC3): every shortcut requires BOTH Ctrl and Shift on a
// letter, so plain Ctrl+letter combos the shell claims (Ctrl+C/D/Z/L/W on
// some shells, ...) never match here and reach the PTY. Focus in a text
// field (workspace rename / create) suppresses all shortcuts so typing names
// does not trigger actions.

export type Command =
  | 'new-workspace'
  | 'new-tab'
  | 'next-workspace'
  | 'prev-workspace'
  | 'split-horizontal'
  | 'split-vertical'
  | 'close-panel'

/// Shape of the keyboard event matchShortcut consumes. `activeTag` is the
/// uppercased tagName of the focused element (or 'BODY' / null when nothing
/// focusable is active) — used to suppress shortcuts while editing text.
export type ShortcutEvent = {
  key: string
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
}

// Arrow keys carry a Shift-state-independent name, so they live in their own
// table rather than the lowercased-letter one.
const ARROWS: Record<string, Command> = {
  ArrowRight: 'next-workspace',
  ArrowLeft: 'prev-workspace',
}

export function matchShortcut(e: ShortcutEvent): Command | null {
  // Typing a workspace name must never trigger a shortcut.
  if (e.activeTag === 'INPUT' || e.activeTag === 'TEXTAREA') return null
  // Ctrl+Shift is the required modifier pair; Alt/Meta combos are left alone.
  if (!(e.ctrlKey && e.shiftKey) || e.altKey || e.metaKey) return null
  return ARROWS[e.key] ?? TABLE[e.key.toLowerCase()] ?? null
}
