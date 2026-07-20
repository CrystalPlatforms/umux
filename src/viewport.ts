// ensureFullscreenViewport — makes the app own the whole window.
//
// Phase 3 / Issue #4 follow-up: see viewport.test.ts for the bug this guards
// (browser default body margin → white border + page scrollbar).

export function ensureFullscreenViewport(doc: Document = document): void {
  const fill = (el: HTMLElement) => {
    el.style.margin = '0px'
    el.style.height = '100%'
    el.style.overflow = 'hidden'
  }
  fill(doc.documentElement)
  fill(doc.body)
}
