import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Read the CSS source directly from disk so the assertion is deterministic
// regardless of how the test runner transforms CSS imports.
const here = path.dirname(fileURLToPath(import.meta.url));
const css = fs.readFileSync(path.join(here, "app.css"), "utf8");

// Design-token discipline for app.css.
//
// Assumptions encoded by these tests:
//  - Input: the full text of src/app.css.
//  - Rule: outside the :root token definitions, colors MUST come from CSS
//    variables (var(--...)). Hardcoded hex literals leak inconsistent colors
//    across the UI (Phase 19: design-token consistency).
//  - Boundary: rgba()/rgb() literals are allowed (box-shadows and terminal
//    overlays legitimately use alpha). Only hex literals are forbidden here.
//  - Not tested this iteration: rgba alpha values, contrast ratios (a11y),
//    inline styles in .tsx files.

/** Strip the :root { ... } block so token *definitions* are not flagged. */
function withoutRoot(source: string): string {
  return source.replace(/:root\s*\{[^}]*\}/, "");
}

describe("design tokens: app.css", () => {
  it("defines no hardcoded hex colors outside :root", () => {
    const hexOutsideRoot =
      withoutRoot(css).match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];

    expect(hexOutsideRoot).toEqual([]);
  });

  it("uses typography tokens for every font-size (no raw px/rem)", () => {
    const declarations =
      withoutRoot(css).match(/font-size:\s*[^;]+;/g) ?? [];
    const offending = declarations.filter(
      (d) => !/var\(--text-/.test(d),
    );

    expect(offending).toEqual([]);
  });
});
