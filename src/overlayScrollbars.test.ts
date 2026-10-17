// Overlay scrollbar contract tests (#105 / story 11).
//
// BEHAVIOR this guards: five scroll regions show an ALWAYS-visible classic
// scrollbar — visual noise over dark chrome that mostly never scrolls. The
// overlay module owns "the bar is part of the content, not the chrome":
// thin + transparent track, hidden until the region scrolls (fading out
// after) or is hovered.
//
// Assumptions encoded (stated before the first RED):
//  - jsdom cannot measure scrollbars, so — same compromise as
//    viewport.test.ts — the style half is asserted as STYLE GUARANTEES: the
//    stylesheet text carries the region selectors, the thin width, the
//    default-hidden thumb, the hover/.is-scrolling reveal, and the fade
//    transition (standard `scrollbar-width`/`scrollbar-color`, honored by
//    WebView2 per the issue's assumption).
//  - NO `::-webkit-scrollbar` pseudo rules, EVER: the first cut shipped
//    them and WebView2's Chromium collapsed the whole shell around them
//    (sidebar list squeezed to the scrollbar's width, tab bar to its
//    height) — live-verified via CDP A/B on 2026-10-17, pseudos removed.
//    The no-pseudo assertion is the regression guard for that crash.
//  - The scroll-flash half is tested BEHAVIORALLY: dispatching `scroll` on a
//    region toggles `.is-scrolling`, the fake clock removes it after the
//    fade delay, the timer re-arms while scrolling continues, several
//    regions track independently, and non-regions (the xterm viewport) are
//    never touched.
//  - NOT tested here: real pixel layout or per-platform scrollbar rendering
//    — Adam verifies in the running app (the #105 HITL script).

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import {
  SCROLL_FADE_MS,
  overlayScrollbarCss,
  attachScrollFlash,
  installOverlayScrollbars,
} from './overlayScrollbars'

describe('overlayScrollbarCss (#105 style guarantees)', () => {
  const css = overlayScrollbarCss()

  it('targets all five regions (sidebar list, tab bar, settings card, cmux preview, dialogs)', () => {
    expect(css).toContain('.workspace-list')
    expect(css).toContain('.tab-bar')
    expect(css).toContain('.modal-card')
    expect(css).toContain('.import-wizard__preview')
  })

  it('styles a thin bar with a transparent track (hidden by default)', () => {
    expect(css).toContain('scrollbar-width: thin')
    expect(css).toContain('scrollbar-color: transparent transparent')
  })

  it('NEVER defines ::-webkit-scrollbar pseudos — they collapse WebView2 layout', () => {
    // The regression guard for the 2026-10-17 crash: author-defined
    // ::-webkit-scrollbar rules switch WebView2's Chromium into a scrollbar
    // mode whose reserved space squeezed the sidebar list to 16px and the
    // tab bar to 8px (live CDP A/B). Standard properties only — forever.
    expect(css).not.toContain('-webkit-scrollbar')
  })

  it('reveals the bar on hover and while scrolling', () => {
    expect(css).toMatch(/:hover[^{}]*\.is-scrolling[^{}]*\{[^}]*scrollbar-color/)
    expect(css).toMatch(/\.is-scrolling[^{}]*\{[^}]*scrollbar-color:[^;]*var\(/)
  })

  it('fades the bar — the thumb color transitions', () => {
    expect(css).toContain('transition: scrollbar-color')
  })

  it('keeps .tab-bar its own padding transition (the fade must not clobber it)', () => {
    expect(css).toMatch(
      /\.tab-bar \{[^}]*transition: padding-left[^}]*scrollbar-color/,
    )
  })
})

describe('installOverlayScrollbars', () => {
  it('injects the stylesheet once, and cleanup removes it', () => {
    expect(document.getElementById('data-umux-overlay-scrollbars')).toBeNull()

    const detachFirst = installOverlayScrollbars()
    const style = document.getElementById('data-umux-overlay-scrollbars')
    expect(style).not.toBeNull()
    expect(style?.textContent).toContain('.workspace-list')

    // Idempotent: a second install replaces the stylesheet — never stacks
    // a copy. Its own cleanup is the live one (the first element it
    // replaced is already gone), so cleanups nest LIFO like listeners do.
    const detachSecond = installOverlayScrollbars()
    expect(
      document.querySelectorAll('#data-umux-overlay-scrollbars'),
    ).toHaveLength(1)

    detachSecond()
    expect(document.getElementById('data-umux-overlay-scrollbars')).toBeNull()
    detachFirst()
  })
})

describe('attachScrollFlash (the flash behavior)', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('marks a scrolling region and fades the mark out after the delay', () => {
    const detach = attachScrollFlash()
    const list = document.createElement('ul')
    list.className = 'workspace-list'
    document.body.appendChild(list)

    list.dispatchEvent(new Event('scroll'))
    expect(list.classList.contains('is-scrolling')).toBe(true)

    vi.advanceTimersByTime(SCROLL_FADE_MS - 1)
    expect(list.classList.contains('is-scrolling')).toBe(true)
    vi.advanceTimersByTime(1)
    expect(list.classList.contains('is-scrolling')).toBe(false)

    detach()
  })

  it('keeps the mark while scrolling continues (the timer re-arms)', () => {
    const detach = attachScrollFlash()
    const bar = document.createElement('div')
    bar.className = 'tab-bar'
    document.body.appendChild(bar)

    bar.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(SCROLL_FADE_MS - 1)
    bar.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(SCROLL_FADE_MS - 1)
    expect(bar.classList.contains('is-scrolling')).toBe(true)
    vi.advanceTimersByTime(1)
    expect(bar.classList.contains('is-scrolling')).toBe(false)

    detach()
  })

  it('ignores elements outside the five regions — the terminal viewport included', () => {
    const detach = attachScrollFlash()
    const viewport = document.createElement('div')
    viewport.className = 'xterm-viewport'
    document.body.appendChild(viewport)

    viewport.dispatchEvent(new Event('scroll'))
    expect(viewport.classList.contains('is-scrolling')).toBe(false)

    detach()
  })

  it('tracks several regions independently', () => {
    const detach = attachScrollFlash()
    const a = document.createElement('div')
    a.className = 'modal-card'
    const b = document.createElement('div')
    b.className = 'import-wizard__preview'
    document.body.append(a, b)

    a.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(SCROLL_FADE_MS - 1)
    b.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(1)
    expect(a.classList.contains('is-scrolling')).toBe(false)
    expect(b.classList.contains('is-scrolling')).toBe(true)

    detach()
  })
})
