// Renderer gate for TerminalSurface (perf audit 2026-09-05 follow-up,
// macOS black-screen regression 2026-09-07, TUI-black follow-up 2026-09-07).
//
// The decision tracks the webview ENGINE, not the OS:
//   - Chromium (Windows WebView2, Edge/Chrome): xterm's WebGL addon renders
//     correctly and is the difference between smooth and janky output.
//   - Apple WebKit (macOS WKWebView) and WebKitGTK (Linux): the WebGL context
//     creates "successfully" but presents a BLACK canvas
//     (xtermjs/xterm.js#3575) — so WebGL is out. The DOM renderer there has a
//     worse sibling failure: under a full-screen TUI's rapid frame stream the
//     panel goes black while the rows' text still sits in the DOM (issue #75 —
//     verified by per-panel DOM probes: content present, screen black). The
//     CANVAS renderer (xterm's own 2D-canvas renderer) draws the viewport as
//     one opaque layer per frame — a compositing path WebKit has shipped for
//     decades — so WebKit engines get the canvas renderer, with the DOM
//     renderer as the last fallback.
//
// Pure and trivially testable: a user-agent string in, a decision out.

export type RendererKind = 'webgl' | 'canvas' | 'dom'

// Chromium announces itself as "Chrome" (WebView2 and Edge both embed it;
// Edge's UA additionally carries "Edg"). WebKit UAs — macOS WKWebView and
// Linux WebKitGTK alike — carry "Version/… Safari/…" and never mention Chrome.
export function rendererKind(userAgent: string): RendererKind {
  if (userAgent.includes('Chrome')) return 'webgl'
  if (userAgent.includes('AppleWebKit')) return 'canvas'
  return 'dom'
}

/** Back-compat wrapper: whether the WebGL renderer may be used at all. */
export function canUseWebglRenderer(userAgent: string): boolean {
  return rendererKind(userAgent) === 'webgl'
}
