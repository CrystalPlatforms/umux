// canUseWebglRenderer contract tests (macOS black-screen regression).
//
// The decision must track the webview ENGINE, not the OS: Chromium-based
// webviews (Windows WebView2, and Edge/Chrome generally) present xterm's
// WebGL canvas correctly; Apple WebKit webviews (macOS WKWebView) and
// WebKitGTK (Linux) create the context but paint a black canvas
// (xtermjs/xterm.js#3575). Real UA strings from the wild, kept verbatim.

import { describe, it, expect } from 'vitest'
import { canUseWebglRenderer } from './gpuRenderer'

describe('canUseWebglRenderer', () => {
  it('allows the Windows WebView2 engine (Chromium, includes Edg)', () => {
    const webView2 =
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) ' +
      'Chrome/138.0.0.0 Safari/537.36 Edg/138.0.0.0'
    expect(canUseWebglRenderer(webView2)).toBe(true)
  })

  it('allows plain Chrome', () => {
    const chrome =
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) ' +
      'Chrome/139.0.0.0 Safari/537.36'
    expect(canUseWebglRenderer(chrome)).toBe(true)
  })

  it('blocks the macOS WKWebView engine (black canvas, xtermjs#3575)', () => {
    const wkWebView =
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 ' +
      '(KHTML, like Gecko) Version/18.5 Safari/605.1.15'
    expect(canUseWebglRenderer(wkWebView)).toBe(false)
  })

  it('blocks Linux WebKitGTK (same WebKit engine, same failure mode)', () => {
    const webkitGtk =
      'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) ' +
      'Version/2.48.1 Safari/605.1.15'
    expect(canUseWebglRenderer(webkitGtk)).toBe(false)
  })

  it('blocks unknown engines — safe default is the DOM renderer', () => {
    expect(canUseWebglRenderer('')).toBe(false)
    expect(canUseWebglRenderer('Mozilla/5.0 jsdom/25.0.1')).toBe(false)
  })
})
