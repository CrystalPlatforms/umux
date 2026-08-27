import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Apple-design discipline for app.css (issue #39), following the
// design-tokens.test.ts precedent: assertions read the CSS source text.
//
// Assumptions encoded by these tests:
//  - Input: the full text of src/app.css.
//  - Rule: the app's only looping animation (the working-agent dot pulse,
//    ~0.63 Hz) is a vestibular trigger — under prefers-reduced-motion it must
//    be disabled. Per Adam's decision the dot STAYS visible in its working
//    color, it just stops pulsing (reduced motion ≠ less information).
//  - Boundary: jsdom does not evaluate media queries, so this is a
//    source-text assertion, not a computed-style one.
//  - Not tested this iteration: computed styles, frame smoothness, how the
//    OS setting reaches the app (Tauri webview handles the media query).

const here = path.dirname(fileURLToPath(import.meta.url));
const css = fs.readFileSync(path.join(here, "app.css"), "utf8");
// The source WITHOUT the reduced-motion block, whose overrides would
// otherwise shadow the real rules during lookups.
const cssNoReduced = stripMedia(css, "prefers-reduced-motion: reduce");

/** Extract the body of a @media (...) { ... } block, if present. Balances
 * braces so nested rules (e.g. a :root override inside the query) survive. */
function mediaBlock(source: string, query: string): string | null {
  const start = source.indexOf(`@media (${query})`);
  if (start === -1) return null;
  const open = source.indexOf("{", start);
  if (open === -1) return null;
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}") {
      depth--;
      if (depth === 0) return source.slice(open + 1, i);
    }
  }
  return null;
}

describe("apple-design: reduced motion (app.css)", () => {
  it("disables the working-agent dot pulse under prefers-reduced-motion, keeping the dot visible", () => {
    const block = mediaBlock(css, "prefers-reduced-motion: reduce");

    // The media query must exist…
    expect(block).not.toBeNull();
    // …and turn off the pulse animation on the working dot.
    expect(block).toMatch(
      /\.agent-status--working\s+\.agent-dot\s*\{[^}]*animation:\s*none/,
    );
  });

  it("neutralizes the press-compression scale under prefers-reduced-motion", () => {
    const block = mediaBlock(css, "prefers-reduced-motion: reduce");

    // scale(1) is the identity transform: press feedback stays, movement
    // goes. Color/background changes are untouched (they aid comprehension
    // and are not motion).
    expect(block).toMatch(/--press-scale:\s*1;/);
  });
});

/** The `cls:active { ... }` rule for a class, or null if it has none. */
function activeRuleFor(source: string, cls: string): string | null {
  return source.match(new RegExp(`${cls}:active\\s*\\{[^}]*\\}`))?.[0] ?? null;
}

describe("apple-design: press feedback (app.css)", () => {
  // Apple §1 Response: feedback must appear on pointer-DOWN, not on release.
  // Button-like controls compress (scale) on :active; the shared
  // --press-scale token exists so reduced motion can neutralize the movement
  // later without touching each rule.
  const BUTTON_LIKE = [
    ".btn-primary",
    ".btn-secondary",
    ".btn-danger",
    ".sidebar-expand",
    ".icon-btn",
    ".mute-button",
    ".panel-close",
    ".fallback-dismiss",
  ];

  it("defines a --press-scale token in :root", () => {
    expect(css).toMatch(/--press-scale:\s*0\.9[0-9];/);
  });

  it.each(BUTTON_LIKE)("%s compresses on :active via --press-scale", (cls) => {
    const activeRule = activeRuleFor(css, cls);
    expect(activeRule).not.toBeNull();
    expect(activeRule).toMatch(/transform:\s*scale\(var\(--press-scale\)\)/);
  });

  // List-like controls (sidebar rows, tabs, menu items) highlight instantly
  // on :active — background feedback, no compression: that is how Apple's own
  // lists and menus behave.
  const LIST_LIKE = [".workspace-row", ".tab", ".menu-item", ".tab-close"];

  it.each(LIST_LIKE)("%s highlights instantly on :active", (cls) => {
    const activeRule = activeRuleFor(css, cls);
    expect(activeRule).not.toBeNull();
    // Instant means at least one visible change declared in the rule.
    expect(activeRule!.replace(/^[^{]*\{/, "").trim()).not.toBe("");
  });
});

describe("apple-design: typography (app.css)", () => {
  // Apple §15: tracking is size-specific. Display-size text wants NEGATIVE
  // letter-spacing (letters read too far apart as they grow); body/small
  // sizes stay near 0. The empty-state h1 is the one display-size line.
  it("defines a negative --tracking-display token used by the display-size title", () => {
    const token = css.match(/--tracking-display:\s*(-0\.\d+em);/);
    expect(token).not.toBeNull();

    const h1Rule = css.match(/\.empty-state h1\s*\{[^}]*\}/);
    expect(h1Rule).not.toBeNull();
    expect(h1Rule![0]).toMatch(/letter-spacing:\s*var\(--tracking-display\)/);
  });
});

describe("apple-design: uniform control sizes (app.css)", () => {
  // Uniformity by construction (#39 follow-up): controls of the same kind
  // share ONE size token, so sizes can never drift apart again.
  //  - standard icon buttons (chrome controls) -> --control-size
  //  - text buttons                             -> --control-height
  //  - compact close affordances (tab/panel ×)  -> --close-size
  const ICON_BUTTONS = [
    ".icon-btn",
    ".mute-button",
    ".sidebar-expand",
    ".fallback-dismiss",
  ];
  const TEXT_BUTTONS = [".btn-primary", ".btn-secondary", ".btn-danger"];
  const CLOSE_BUTTONS = [".tab-close", ".panel-close"];

  /** Body of the first rule for `selector` (null when absent). */
  function ruleFor(selector: string): string | null {
    return css.match(new RegExp(`${selector}\\s*\\{[^}]*\\}`))?.[0] ?? null;
  }

  it("sizes every standard icon button from --control-size", () => {
    ICON_BUTTONS.forEach((cls) => {
      const rule = ruleFor(cls);
      expect(rule, cls).not.toBeNull();
      expect(rule, cls).toMatch(/width:\s*var\(--control-size\)/);
      expect(rule, cls).toMatch(/height:\s*var\(--control-size\)/);
    });
  });

  it("sizes every text button's height from --control-height", () => {
    TEXT_BUTTONS.forEach((cls) => {
      const rule = ruleFor(cls);
      expect(rule, cls).not.toBeNull();
      expect(rule, cls).toMatch(/height:\s*var\(--control-height\)/);
    });
  });

  it("sizes both compact close affordances from --close-size", () => {
    CLOSE_BUTTONS.forEach((cls) => {
      const rule = ruleFor(cls);
      expect(rule, cls).not.toBeNull();
      expect(rule, cls).toMatch(/width:\s*var\(--close-size\)/);
      expect(rule, cls).toMatch(/height:\s*var\(--close-size\)/);
    });
  });
});

/** Remove a whole @media block (header + balanced body) from the source, so
 * rule lookups find the REAL rule, not the reduced-motion override of it. */
function stripMedia(source: string, query: string): string {
  const start = source.indexOf(`@media (${query})`);
  if (start === -1) return source;
  const open = source.indexOf("{", start);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}") {
      depth--;
      if (depth === 0) return source.slice(0, start) + source.slice(i + 1);
    }
  }
  return source;
}

describe("apple-design: enter animations (app.css)", () => {
  // Apple §7 (anchor to source) + §12 (materialize, don't just fade): popovers
  // and menus GROW from the point that triggered them, not from their center.
  // Reduced motion replaces the movement with an instant appearance.

  it("context menu enters with fade+scale anchored at the click point", () => {
    const rule = cssNoReduced.match(/\.context-menu\s*\{[^}]*\}/)?.[0];
    expect(rule).toBeDefined();
    // Anchored at the click point (the menu's left/top)…
    expect(rule).toMatch(/transform-origin:\s*top left/);
    // …growing in, not blinking in.
    expect(rule).toMatch(/animation:\s*menu-enter\s/);

    // The keyframes fade AND scale (a material arriving)…
    const frames = css.match(/@keyframes menu-enter\s*\{[\s\S]*?\n\}/)?.[0];
    expect(frames).toBeDefined();
    expect(frames).toMatch(/from\s*\{[^}]*opacity:\s*0/);
    expect(frames).toMatch(/from\s*\{[^}]*transform:\s*scale\(/);

    // …and reduced motion drops the movement entirely.
    const reduced = mediaBlock(css, "prefers-reduced-motion: reduce");
    expect(reduced).toMatch(/\.context-menu\s*\{[^}]*animation:\s*none/);
  });

  it("modal scrim fades in and the card scales in (materializing)", () => {
    const overlay = cssNoReduced.match(/\.modal-overlay\s*\{[^}]*\}/)?.[0];
    expect(overlay).toBeDefined();
    expect(overlay).toMatch(/animation:\s*overlay-enter\s/);

    const card = cssNoReduced.match(/\.modal-card\s*\{[^}]*\}/)?.[0];
    expect(card).toBeDefined();
    expect(card).toMatch(/animation:\s*card-enter\s/);

    // The card grows from slightly smaller — a surface arriving in place…
    const cardFrames = css.match(/@keyframes card-enter\s*\{[\s\S]*?\n\}/)?.[0];
    expect(cardFrames).toBeDefined();
    expect(cardFrames).toMatch(/from\s*\{[^}]*transform:\s*scale\(0\.9[0-9]*\)/);
    expect(cardFrames).toMatch(/from\s*\{[^}]*opacity:\s*0/);

    // …while the scrim is a plain opacity fade (it only ever dims).
    const overlayFrames = css.match(
      /@keyframes overlay-enter\s*\{[\s\S]*?\n\}/,
    )?.[0];
    expect(overlayFrames).toBeDefined();
    expect(overlayFrames).toMatch(/from\s*\{[^}]*opacity:\s*0/);

    // Reduced motion: instant appearance for both layers.
    const reduced = mediaBlock(css, "prefers-reduced-motion: reduce");
    expect(reduced).toMatch(/\.modal-overlay\s*\{[^}]*animation:\s*none/);
    expect(reduced).toMatch(/\.modal-card\s*\{[^}]*animation:\s*none/);
  });

  it("sidebar slides shut (width transition) instead of popping away", () => {
    const base = cssNoReduced.match(/\.sidebar\s*\{[^}]*\}/)?.[0];
    expect(base).toBeDefined();
    // The collapse is a WIDTH animation; contents are clipped, not reflowed…
    expect(base).toMatch(/overflow:\s*hidden/);
    expect(base).toMatch(/transition:[^;]*width/);

    const collapsedRule = css.match(/\.sidebar\.is-collapsed\s*\{[^}]*\}/)?.[0];
    expect(collapsedRule).toBeDefined();
    expect(collapsedRule).toMatch(/width:\s*0/);
    // …and only after the slide does the sidebar leave the a11y tree / tab
    // order (visibility is DELAYED to the end of the collapse).
    expect(collapsedRule).toMatch(/visibility:\s*hidden/);
    expect(collapsedRule).toMatch(/visibility[^;]*0s[^;]*0\.25s|visibility:\s*hidden[\s\S]{0,120}transition:[^;]*visibility[^;]*0\.25s/);

    // The inner column holds a fixed width so text never reflows mid-slide.
    const inner = cssNoReduced.match(/\.sidebar-inner\s*\{[^}]*\}/)?.[0];
    expect(inner).toBeDefined();
    expect(inner).toMatch(/width:\s*240px/);

    // Reduced motion: the sidebar snaps open/shut with no transition.
    const reduced = mediaBlock(css, "prefers-reduced-motion: reduce");
    expect(reduced).toMatch(/\.sidebar[^{]*\{[^}]*transition:\s*none/);
  });
});

describe("apple-design: chrome band (app.css)", () => {
  // #39 follow-up: the sidebar header (umux + top buttons) and the terminal
  // tab bar form ONE chrome band — their bottom separators must sit at the
  // same height, reading as a single continuous line. Guaranteed by sharing
  // one row-height token, not by coincidence.

  it("gives the sidebar header and the tab bar the same row height", () => {
    expect(css).toMatch(/--chrome-row:\s*\d+px;/);

    const header = cssNoReduced.match(/\.sidebar-header\s*\{[^}]*\}/)?.[0];
    expect(header).toBeDefined();
    expect(header).toMatch(/height:\s*var\(--chrome-row\)/);
    expect(header).toMatch(/border-bottom/);

    const bar = cssNoReduced.match(/\.tab-bar\s*\{[^}]*\}/)?.[0];
    expect(bar).toBeDefined();
    expect(bar).toMatch(/height:\s*var\(--chrome-row\)/);
    expect(bar).toMatch(/border-bottom/);
  });

  it("seats the expand toggle in the tab row, before the tabs", () => {
    // The toggle is vertically centered on the chrome row (derived from the
    // shared tokens, not a magic number)…
    const btn = cssNoReduced.match(/\.sidebar-expand\s*\{[^}]*\}/)?.[0];
    expect(btn).toBeDefined();
    expect(btn).toMatch(/left:\s*8px/);
    expect(btn).toMatch(
      /top:\s*calc\(\(var\(--chrome-row\) - var\(--control-size\)\) \/ 2\)/,
    );

    // …and the collapsed shell's tab bar reserves room, so the first tab
    // starts AFTER the toggle instead of underneath it.
    const reserve = cssNoReduced.match(
      /\.shell\.is-sidebar-collapsed \.tab-bar\s*\{[^}]*\}/,
    )?.[0];
    expect(reserve).toBeDefined();
    expect(reserve).toMatch(/padding-left:\s*\d+px/);

    // The reserved space slides in with the sidebar (same 0.25s) and, like
    // every other movement, snaps under reduced motion.
    const bar = cssNoReduced.match(/\.tab-bar\s*\{[^}]*\}/)?.[0];
    expect(bar).toMatch(/transition:[^;]*padding-left/);
    const reduced = mediaBlock(css, "prefers-reduced-motion: reduce");
    expect(reduced).toMatch(/\.tab-bar\s*\{[^}]*transition:\s*none/);
  });
});

describe("apple-design: panel chrome (#40)", () => {
  // The pane's top-right corner stacks: × (4px), zoom (next slot), then the
  // agent-status chip — the chip used to sit ON the zoom button (HITL: the
  // arrows were invisible), so its slot is asserted to reserve room for BOTH
  // chrome buttons. The zoom button itself defines ONLY its position here:
  // its looks come from .panel-close, so it can never drift from the ×.
  const statusRule =
    cssNoReduced.match(/\.surface \.agent-status\s*\{[^}]*\}/)?.[0] ?? null;
  const zoomRule =
    cssNoReduced.match(/\.panel-zoom\s*\{[^}]*\}/)?.[0] ?? null;

  it("seats the agent-status chip left of BOTH chrome buttons", () => {
    expect(statusRule).not.toBeNull();
    // 4px margin + two full (button + 4px) slots after it.
    expect(statusRule).toMatch(
      /right:\s*calc\(4px \+ 2 \* \(var\(--close-size\) \+ 4px\)\)/,
    );
  });

  it("styles the zoom button exactly like the close button (position only)", () => {
    expect(zoomRule).not.toBeNull();
    const declarations = zoomRule!
      .replace(/^[^{]*\{/, "")
      .replace(/\}$/, "")
      .split(";")
      .map((d) => d.trim())
      .filter(Boolean);
    expect(declarations).toEqual([
      "right: calc(4px + var(--close-size) + 4px)",
    ]);
  });
});
