import { describe, expect, it } from "vitest";
import { clipboardAction, type KeyLike } from "./clipboardShortcut";

// Behavioral spec for the terminal copy shortcut (Phase 19).
//
// Assumptions encoded:
//  - Input: a KeyboardEvent-shaped object (ctrlKey/shiftKey/metaKey/key).
//  - Output: 'copy' | null.
//  - The ONLY recognized binding is Ctrl+Shift+C. Ctrl+C alone MUST return null
//    so it reaches the shell as SIGINT (Adam confirmed: Ctrl+C interrupts the
//    process, Ctrl+Shift+C copies).
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

  it("passes Ctrl+C through (null) so the shell sees SIGINT", () => {
    expect(clipboardAction(press({ ctrlKey: true, key: "c" }))).toBe(null);
    expect(clipboardAction(press({ ctrlKey: true, key: "C" }))).toBe(null);
  });

  it("passes non-C keys through even with Ctrl+Shift", () => {
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "v" }))).toBe(null);
    expect(clipboardAction(press({ ctrlKey: true, shiftKey: true, key: "a" }))).toBe(null);
  });

  it("does not handle macOS Cmd+C (Linux scope)", () => {
    expect(clipboardAction(press({ metaKey: true, key: "c" }))).toBe(null);
  });

  it("passes plain keystrokes through", () => {
    expect(clipboardAction(press({ key: "c" }))).toBe(null);
    expect(clipboardAction(press({ shiftKey: true, key: "C" }))).toBe(null);
  });
});
