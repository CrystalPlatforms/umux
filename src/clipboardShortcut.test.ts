import { describe, expect, it } from "vitest";
import { clipboardAction, type KeyLike } from "./clipboardShortcut";

// Behavioral spec for the terminal clipboard shortcuts (Phase 19; paste
// added HITL 2026-09-10).
//
// Assumptions encoded:
//  - Input: a KeyboardEvent-shaped object (ctrlKey/shiftKey/metaKey/key).
//  - Output: 'copy' | 'paste' | null.
//  - Copy binds Ctrl+Shift+C; paste binds Ctrl+Shift+V (the universal
//    terminal paste chord — it used to be split-vertical in the app's own
//    shortcut table, so pasting split the pane instead). Ctrl+C alone MUST
//    return null so it reaches the shell as SIGINT (Adam confirmed: Ctrl+C
//    interrupts the process, Ctrl+Shift+C copies).
//  - macOS Cmd+C (metaKey) is intentionally not handled (Linux/Wayland scope).

const press = (overrides: Partial<KeyLike>): KeyLike => ({
  ctrlKey: false,
  shiftKey: false,
  metaKey: false,
  key: "",
  ...overrides,
});

describe("clipboardAction", () => {
  it("recognizes Ctrl+Shift+C as copy", () => {
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "c" }))).toBe("copy");
    // Letter case must not matter (Caps Lock, shifted key).
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "C" }))).toBe("copy");
  });

  it("recognizes Ctrl+Shift+V as paste", () => {
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "v" }))).toBe("paste");
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "V" }))).toBe("paste");
  });

  it("passes Ctrl+C and Ctrl+V through (null) so the shell sees them", () => {
    expect(clipboardAction(press({ ctrlKey: true, key: "c" }))).toBe(null);
    expect(clipboardAction(press({ ctrlKey: true, key: "C" }))).toBe(null);
    // Plain Ctrl+V is xterm's native paste path — never intercepted here.
    expect(clipboardAction(press({ ctrlKey: true, key: "v" }))).toBe(null);
  });

  it("passes other keys through even with Ctrl+Shift", () => {
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "a" }))).toBe(null);
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "e" }))).toBe(null);
  });

  it("does not handle macOS Cmd+C (Linux scope)", () => {
    expect(clipboardAction(press({ metaKey: true, key: "c" }))).toBe(null);
  });

  it("passes plain keystrokes through", () => {
    expect(clipboardAction(press({ key: "c" }))).toBe(null);
    expect(clipboardAction(press({ shiftKey: true, key: "C" }))).toBe(null);
  });
});
