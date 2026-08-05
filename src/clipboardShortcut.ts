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

export type ClipboardAction = "copy";

/** Subset of KeyboardEvent this function reasons about. */
export interface KeyLike {
  ctrlKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  key: string;
}

/**
 * Recognize the terminal copy shortcut.
 *
 * Ctrl+Shift+C  -> 'copy'   (Linux/GNOME convention; this is the only binding)
 * Ctrl+C        -> null     (MUST reach the shell as SIGINT — never copy)
 * anything else -> null     (pass through to the PTY)
 */
export function clipboardAction(event: KeyLike): ClipboardAction | null {
  // macOS uses Cmd+C and we explicitly do not touch it (Linux/Wayland scope).
  if (event.metaKey) return null;

  // Only the Ctrl+Shift+C chord copies. Ctrl without Shift must pass through
  // so Ctrl+C keeps interrupting the foreground process (SIGINT).
  if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === "c") {
    return "copy";
  }

  return null;
}
