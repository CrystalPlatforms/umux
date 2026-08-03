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
