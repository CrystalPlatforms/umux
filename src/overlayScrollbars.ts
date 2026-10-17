// overlayScrollbars — the shared overlay-scrollbar module (#105 / story 11).
//
// ONE definition for the five scroll regions (sidebar list, tab bar, Settings
// card, cmux import preview, dialogs): a thin bar with a transparent track
// that shows only while the region is SCROLLING (the scroll-flash class the
// behavior half toggles) or while the region is HOVERED, fading out shortly
// after scrolling stops. The terminal pane's own scrollbar is deliberately
// NOT covered — xterm's viewport matches none of these selectors, and
// restyling it is out of scope (#105).
//
// PLATFORM CONSTRAINT (live-verified in WebView2 on 2026-10-17, after the
// first cut broke the whole shell): the styling uses ONLY the standard
// `scrollbar-width` / `scrollbar-color` properties. Defining
// `::-webkit-scrollbar` pseudo-element rules — even alongside the standard
// ones — switches WebView2's Chromium into its author-defined-scrollbar mode
// and COLLAPSES the regions' layout (the sidebar list squeezed to the
// scrollbar's width, the tab bar to its height; `::-webkit-scrollbar` was
// live-A/B-verified as the sole cause and removed). Standard properties
// never touch layout: engines without support (older WKWebView) simply keep
// their native scrollbars, which is the honest per-platform fallback — the
// issue's assumption names WebView2 as the target that must honor this.
//
// jsdom cannot measure scrollbars, so the tests assert the STYLE GUARANTEES
// (the same compromise viewport.test.ts made for the page scrollbar), and
// pin the no-pseudo constraint so the crash can never come back.

/// The five regions as ONE selector list — the single source both the CSS
/// and the scroll-flash delegate read. `.modal-card` covers two regions at
/// once: the Settings card AND every dialog (close confirmations, the custom
/// shell dialog, ...).
export const OVERLAY_REGION_SELECTOR =
  '.workspace-list, .tab-bar, .modal-card, .import-wizard__preview'

/// How long the thumb stays visible after the LAST scroll event before it
/// fades out. Continuous scrolling re-arms the timer, so the bar never
/// flickers mid-gesture.
export const SCROLL_FADE_MS = 600

/// The shared stylesheet text. Pure — the tests assert the guarantees on
/// this string, and installOverlayScrollbars injects exactly it.
export function overlayScrollbarCss(): string {
  return `
${OVERLAY_REGION_SELECTOR} {
  scrollbar-width: thin;
  /* Hidden until the region scrolls or is hovered — the flash default. */
  scrollbar-color: transparent transparent;
  /* The fade (#105): a real transition on Chromium 121+/WebView2. */
  transition: scrollbar-color 0.35s ease;
}
${OVERLAY_REGION_SELECTOR}:hover,
${OVERLAY_REGION_SELECTOR}.is-scrolling {
  scrollbar-color: var(--text-faint, #5b6373) transparent;
}
/* .tab-bar carries the ONLY pre-existing transition among the regions
   (app.css: the sidebar-collapse padding animation). The bare transition
   declaration above would clobber it on that element — this rule re-states
   both, and the test pins the padding-left half so the two can never
   drift. */
.tab-bar {
  transition: padding-left 0.25s ease, scrollbar-color 0.35s ease;
}
`.trim()
}

/// The scroll-flash behavior: on every scroll event (capture — scroll does
/// not bubble) a region gains `.is-scrolling` and loses it again `fadeMs`
/// after its LAST event, so continuous scrolling keeps the thumb up and the
/// fade only starts once the wheel stops. Elements outside the five regions
/// (the terminal viewport first among them) are ignored. Returns the
/// cleanup.
export function attachScrollFlash(
  doc: Document = document,
  fadeMs: number = SCROLL_FADE_MS,
): () => void {
  const timers = new Map<Element, number>()
  const onScroll = (e: Event) => {
    const el = e.target
    if (!(el instanceof Element)) return
    if (!el.matches(OVERLAY_REGION_SELECTOR)) return
    el.classList.add('is-scrolling')
    const pending = timers.get(el)
    if (pending != null) window.clearTimeout(pending)
    timers.set(
      el,
      window.setTimeout(() => {
        timers.delete(el)
        el.classList.remove('is-scrolling')
      }, fadeMs),
    )
  }
  doc.addEventListener('scroll', onScroll, true)
  return () => {
    doc.removeEventListener('scroll', onScroll, true)
    for (const [el, timer] of timers) {
      window.clearTimeout(timer)
      el.classList.remove('is-scrolling')
    }
    timers.clear()
  }
}

/// Boot both halves at once (main.tsx): inject the stylesheet and wire the
/// flash. Idempotent — a second install removes the first stylesheet before
/// appending, so a double invocation can never stack copies. Returns the
/// combined cleanup.
export function installOverlayScrollbars(doc: Document = document): () => void {
  const id = 'data-umux-overlay-scrollbars'
  doc.getElementById(id)?.remove()
  const style = doc.createElement('style')
  style.id = id
  style.textContent = overlayScrollbarCss()
  doc.head.appendChild(style)
  const detachFlash = attachScrollFlash(doc)
  return () => {
    style.remove()
    detachFlash()
  }
}
