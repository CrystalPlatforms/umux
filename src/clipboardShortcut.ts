// clipboardShortcut — pure keyboard-shortcut recognition for terminal
// clipboard copy (Phase 19, HITL: Adam wanted Ctrl+Shift+C to copy).
//
// xterm.js sends every keystroke to the PTY by default; to make Ctrl+Shift+C
// copy the selection instead of reaching the shell, TerminalSurface installs
// attachCustomKeyEventHandler, which consults this function. Returning 'copy'
// means "handle it in the renderer (copy) and swallow the key"; returning null
// means "not ours — pass it through to the PTY".
//
// Kept pure (no DOM, no clipboard API) so it is trivially unit-testable, mirroring
// the OscParser / PaneLayout philosophy of a small interface over a focused rule.

export type ClipboardAction = "copy" | "paste";

/** Subset of KeyboardEvent this function reasons about. */
export interface KeyLike {
  ctrlKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  key: string;
}

/**
 * Recognize the terminal clipboard shortcuts.
 *
 * Ctrl+Shift+C  -> 'copy'   (Linux/GNOME convention)
 * Ctrl+Shift+V  -> 'paste'  (the universal terminal paste chord; HITL
 *                            2026-09-10 — it used to sit on split-vertical,
 *                            so pasting split the pane instead)
 * Ctrl+C        -> null     (MUST reach the shell as SIGINT — never copy)
 * Ctrl+V        -> null     (xterm's own native paste path handles it)
 * anything else -> null     (pass through to the PTY)
 */
export function clipboardAction(event: KeyLike): ClipboardAction | null {
  // macOS uses Cmd+C and we explicitly do not touch it (Linux/Wayland scope).
  if (event.metaKey) return null;

  // Only the Ctrl+Shift chords act. Ctrl without Shift must pass through so
  // Ctrl+C keeps interrupting the foreground process (SIGINT) and Ctrl+V
  // keeps reaching xterm's paste handler.
  if (event.ctrlKey && event.shiftKey) {
    const key = event.key.toLowerCase();
    if (key === "c") return "copy";
    if (key === "v") return "paste";
  }

  return null;
}
