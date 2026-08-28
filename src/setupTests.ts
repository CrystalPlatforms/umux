import '@testing-library/jest-dom'

// jsdom does not ship ResizeObserver. TerminalSurface uses it to re-fit xterm
// when its container resizes (split / divider drag). Stub it as a no-op so the
// component mounts under test; the real behavior is verified manually on
// Ubuntu/Wayland.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver

// jsdom does not ship PointerEvent either (live pointer drag & drop, round
// 3): stand in with a MouseEvent subclass so fireEvent's init props —
// button, clientX, clientY — actually reach the handlers. Real browsers
// have PointerEvent natively; only the test environment needs this.
if (typeof window !== 'undefined' && typeof window.PointerEvent === 'undefined') {
  class PointerEventStub extends MouseEvent {
    pointerId: number
    pointerType: string
    isPrimary: boolean
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init)
      this.pointerId = init.pointerId ?? 0
      this.pointerType = init.pointerType ?? ''
      this.isPrimary = init.isPrimary ?? false
    }
  }
  window.PointerEvent = PointerEventStub as unknown as typeof PointerEvent
}
