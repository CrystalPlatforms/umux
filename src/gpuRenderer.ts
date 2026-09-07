// GPU-renderer gate for TerminalSurface (perf audit 2026-09-05 follow-up,
// macOS black-screen regression 2026-09-07).
//
// The WebGL addon only RENDERS correctly where the webview's engine is
// Chromium — on Windows that is WebView2, and there it is the difference
// between smooth and janky output. Everywhere else umux's webview runs Apple
// WebKit (macOS WKWebView) or WebKitGTK (Linux), and those create the WebGL
// context "successfully" but present a BLACK canvas: the shell's prompt never
// appears while keystrokes still reach the shell (xtermjs/xterm.js#3575).
// The addon's onContextLoss fallback cannot save us — the context is never
// LOST, it just never paints — so we opt in per engine, not per capability.
//
// Pure and trivially testable: a user-agent string in, a decision out.

// Chromium announces itself as "Chrome" (WebView2 and Edge both embed it;
// Edge's UA additionally carries "Edg"). WebKit UAs — macOS WKWebView and
// Linux WebKitGTK alike — carry "Version/… Safari/…" and never mention Chrome.
export function canUseWebglRenderer(userAgent: string): boolean {
  return userAgent.includes('Chrome')
}
