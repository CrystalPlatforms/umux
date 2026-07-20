// Viewport reset contract tests (Phase 3 / Issue #4 follow-up).
//
// BUG this guards: without a CSS reset, <body> keeps the browser's default
// 8px margin (white border around the terminal), and a 100vh-tall terminal
// plus that margin overflows the window, producing a page scrollbar the user
// can drag even though the app is a single full-window terminal.
//
// ensureFullscreenViewport() is a small, deep module that owns "the app owns
// the whole window": it forces html/body to zero margin, 100% height, and
// hidden overflow.
//
// Assumptions encoded:
//  - Side effect: mutates document.documentElement + document.body inline styles.
//  - jsdom cannot measure real scrollbars, so we assert the styles that
//    GUARANTEE no scrollbar / white border (margin:0, overflow:hidden,
//    height:100%) rather than the rendered geometry. This is the standard
//    jsdom compromise for layout bugs.
//  - NOT tested here: actual pixel layout / Wayland webview rendering — Adam
//    verifies that manually (npm run tauri dev).

import { describe, it, expect, beforeEach } from 'vitest'
import { ensureFullscreenViewport } from './viewport'

describe('ensureFullscreenViewport', () => {
  beforeEach(() => {
    // Start each test from a "dirty" document (as if the browser default
    // margin were still in place) so we verify the function actually resets it.
    document.documentElement.style.cssText = ''
    document.body.style.cssText = ''
  })

  it('removes the body margin so there is no white border around the terminal', () => {
    ensureFullscreenViewport()

    expect(getComputedStyle(document.body).margin).toBe('0px')
  })

  it('disables page scrolling so the terminal fills the window with no scrollbar', () => {
    ensureFullscreenViewport()

    expect(getComputedStyle(document.body).overflow).toBe('hidden')
    expect(getComputedStyle(document.documentElement).overflow).toBe('hidden')
  })

  it('makes html and body fill the window height', () => {
    ensureFullscreenViewport()

    expect(getComputedStyle(document.documentElement).height).toBe('100%')
    expect(getComputedStyle(document.body).height).toBe('100%')
  })
})
