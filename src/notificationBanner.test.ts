// NotificationBanner — the umux-owned Windows notification banner (#118).
//
// On the Insider machine the native toast's click is dead at the OS level
// (both -ToastActivated and umux-app:// — issue #118), so Windows shows an
// umux-owned banner window instead. This controller is the banner's BRAIN,
// kept DOM-free and Tauri-free (callback hooks injected — same style as
// PendingNotificationTarget was): show / auto-dismiss / click.
//
// Assumptions encoded by these tests:
//  - ONE banner window. A new show REPLACES the current one: the content is
//    re-rendered and the auto-dismiss timer restarts. No stacking — a queue
//    is a follow-up if real use asks for one.
//  - Lifecycle: show → visible (auto-dismiss timer) → fade-out starts
//    (onFadeOut hook) → window hidden after the fade duration (hideWindow
//    hook). The glue maps the hooks onto the DOM/CSS and the Tauri window.
//  - click() means the user clicked the visible banner: the EXACT raw
//    payload travels out through onActivate (the main window's existing
//    `notification_activated` listener navigates with it) and the banner
//    hides IMMEDIATELY — no fade for a click, the user already acted.
//  - click() when nothing is shown is a no-op (nothing on screen to click).
//  - Durations are constructor-injectable (defaults 5 s auto-dismiss,
//    250 ms fade-out) so tests drive the real constants via fake timers.
//  - NOT tested here: the DOM/CSS look, the banner page glue (bannerEntry),
//    Rust window creation/positioning, the platform split itself.

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { NotificationBanner, type BannerHooks } from './notificationBanner'

// Recording hooks — the controller's whole output surface. Each hook counts
// its calls so the tests assert on sequences ("hidden exactly once"), not
// on any DOM detail.
function recordingHooks(): BannerHooks & {
  rendered: { summary: string; body: string; payload: string }[]
  shown: number
  fades: number
  hidden: number
  activated: string[]
} {
  const state = {
    rendered: [] as { summary: string; body: string; payload: string }[],
    shown: 0,
    fades: 0,
    hidden: 0,
    activated: [] as string[],
  }
  return {
    ...state,
    get rendered() {
      return state.rendered
    },
    get shown() {
      return state.shown
    },
    get fades() {
      return state.fades
    },
    get hidden() {
      return state.hidden
    },
    get activated() {
      return state.activated
    },
    render(view) {
      state.rendered.push(view)
    },
    showWindow() {
      state.shown += 1
    },
    onFadeOut() {
      state.fades += 1
    },
    hideWindow() {
      state.hidden += 1
    },
    onActivate(payload) {
      state.activated.push(payload)
    },
  }
}

describe('NotificationBanner', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  // T1 (tracer — AC1: a ping shows the banner):
  //   show() puts the notification's summary/body AND its exact raw payload
  //   on the banner, then makes the window visible.
  it('show renders the notification content and shows the banner window', () => {
    const hooks = recordingHooks()
    const banner = new NotificationBanner(hooks)

    banner.show('Claude Code', 'Agent is waiting for your input', '{"kind":"waiting"}')

    expect(hooks.rendered).toEqual([
      { summary: 'Claude Code', body: 'Agent is waiting for your input', payload: '{"kind":"waiting"}' },
    ])
    expect(hooks.shown).toBe(1)
  })

  // T2 (AC4: the banner auto-dismisses): the banner stays fully visible for
  // the dismiss delay, THEN fades, and only the END of the fade hides the
  // window — a mid-fade banner is still on screen.
  it('auto-dismisses: fade starts after the delay, the window hides after the fade', () => {
    const hooks = recordingHooks()
    const banner = new NotificationBanner(hooks, 5_000, 250)
    banner.show('umux', 'Agent is waiting for your input', '{"kind":"waiting"}')

    vi.advanceTimersByTime(4_999)
    expect(hooks.fades).toBe(0)
    expect(hooks.hidden).toBe(0)

    vi.advanceTimersByTime(1) // t = 5000: the dismiss delay is up — fade begins
    expect(hooks.fades).toBe(1)
    expect(hooks.hidden).toBe(0)

    vi.advanceTimersByTime(249) // mid-fade — still on screen
    expect(hooks.hidden).toBe(0)

    vi.advanceTimersByTime(1) // t = 5250: fade finished — hide
    expect(hooks.hidden).toBe(1)
  })

  // T3 (one banner window, latest wins): a second show while the first banner
  // is still up (or already fading) must KILL its pending dismissal — the old
  // banner's timer never fires, the window hides exactly once, on the second
  // banner's clock.
  it('a re-show cancels the pending dismissal of the previous banner', () => {
    const hooks = recordingHooks()
    const banner = new NotificationBanner(hooks, 5_000, 250)
    banner.show('first', 'body', '{"n":1}')

    vi.advanceTimersByTime(4_000) // first banner nearly dismissed…
    banner.show('second', 'body', '{"n":2}') // …replaced: its timer must die

    vi.advanceTimersByTime(5_000) // t = 9000: ONLY the second dismiss is due
    expect(hooks.rendered).toHaveLength(2)
    expect(hooks.fades).toBe(1)

    vi.advanceTimersByTime(250) // t = 9250: the second banner's fade ends
    expect(hooks.hidden).toBe(1) // exactly one hide — the first never fired
  })

  // T4 (AC2: clicking navigates to the EXACT target): the click sends the
  // banner's raw payload out untouched and hides the window at once — no
  // fade, the user already acted. The dismissal timers die with it.
  it('click sends the exact raw payload and hides the banner immediately', () => {
    const hooks = recordingHooks()
    const banner = new NotificationBanner(hooks, 5_000, 250)
    const payload = '{"kind":"waiting","workspaceId":"w1","tabId":"t1","panelId":"p1"}'
    banner.show('Claude Code', 'Agent is waiting for your input', payload)

    banner.click()

    expect(hooks.activated).toEqual([payload])
    expect(hooks.fades).toBe(0)
    expect(hooks.hidden).toBe(1)

    vi.advanceTimersByTime(5_000 + 250) // the dismissed timers must be dead
    expect(hooks.fades).toBe(0)
    expect(hooks.hidden).toBe(1) // no second hide
  })

  // T5 (nothing on screen → nothing to click): a click with no active banner
  // is a full no-op — no payload out, no window calls at all.
  it('click with nothing shown is a no-op', () => {
    const hooks = recordingHooks()
    const banner = new NotificationBanner(hooks)

    banner.click()

    expect(hooks.activated).toEqual([])
    expect(hooks.hidden).toBe(0)
    expect(hooks.fades).toBe(0)
  })

  // T6 (a fading banner is still on screen and clickable): clicking mid-fade
  // still activates and hides; the fade's own hide timer dies with the click,
  // so the window is hidden exactly once.
  it('click during the fade still activates and hides without a second hide', () => {
    const hooks = recordingHooks()
    const banner = new NotificationBanner(hooks, 5_000, 250)
    banner.show('umux', 'body', '{"n":1}')

    vi.advanceTimersByTime(5_000) // the fade begins
    expect(hooks.fades).toBe(1)

    banner.click()
    expect(hooks.activated).toEqual(['{"n":1}'])
    expect(hooks.hidden).toBe(1)

    vi.advanceTimersByTime(1_000)
    expect(hooks.hidden).toBe(1) // the fade timer died — no double hide
  })
})
