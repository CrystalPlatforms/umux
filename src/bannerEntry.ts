// bannerEntry — glue for the umux notification banner window (#118).
//
// The Rust side (AppBanner in lib.rs) positions and shows this window per
// notification; the page's whole job is: paint what the controller decides,
// flip the fade class, and on a click send the EXACT raw payload out as
// `notification_activated` — the main window's existing listener navigates
// to that workspace/tab/panel and focuses umux (WorkspaceShell). Delivery
// arrives twice over: the Rust eval (`window.__umuxBannerShow`) for the
// common case, and the `banner_take_pending` boot pull for a notification
// that fired before this page finished loading. Both feed the same
// controller, so a double delivery just restarts the same banner.
//
// NOT tested here (unit suite): this file is the thin DOM/Tauri adapter —
// the decisions live in notificationBanner.ts and are tested there.

import { emit, listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { NotificationBanner } from './notificationBanner'

type BannerEnvelope = {
  summary?: unknown
  body?: unknown
  payload?: unknown
}

const host = document.getElementById('banner') as HTMLElement
const summaryEl = document.getElementById('banner-summary') as HTMLElement
const bodyEl = document.getElementById('banner-body') as HTMLElement
const appWindow = getCurrentWindow()

const controller = new NotificationBanner({
  render(view) {
    summaryEl.textContent = view.summary
    bodyEl.textContent = view.body
    // A re-show reverses a fade that is still running.
    host.classList.remove('is-fading')
  },
  showWindow() {
    // The slide-in restarts only AFTER the window is actually visible —
    // applied any earlier, the animation would run while the window is
    // still hidden and the banner would appear already settled.
    void appWindow.show().then(() => {
      host.classList.remove('is-entering')
      // Force a reflow so removing + re-adding the class in the same tick
      // genuinely restarts the CSS animation.
      void host.offsetWidth
      host.classList.add('is-entering')
    })
  },
  onFadeOut() {
    host.classList.add('is-fading')
  },
  hideWindow() {
    void appWindow.hide()
  },
  onActivate(payload) {
    // The main window's listener navigates to the exact target and focuses
    // umux. An empty payload (the payload-less plain show path) parses as
    // junk there and is ignored — Windows pings always carry one.
    void emit('notification_activated', payload)
  },
})
host.addEventListener('click', () => controller.click())

// Validate + apply one envelope (our own backend's shape; junk never
// crashes the banner).
function applyEnvelope(env: BannerEnvelope): void {
  if (typeof env.summary !== 'string' || typeof env.body !== 'string') return
  const payload = typeof env.payload === 'string' ? env.payload : ''
  controller.show(env.summary, env.body, payload)
}

// The eval entry the Rust shower calls with the envelope OBJECT literal.
(window as unknown as { __umuxBannerShow: (env: BannerEnvelope) => void }).__umuxBannerShow =
  applyEnvelope

// Same envelope arriving as a JSON string (the boot pull path).
function showEnvelope(raw: string): void {
  try {
    applyEnvelope(JSON.parse(raw) as BannerEnvelope)
  } catch {
    return
  }
}

void listen<string>('umux:banner-show', (event) => {
  showEnvelope(event.payload)
})

// The boot pull: a notification that fired before the listener above was
// registered left itself in the backend store (and cleared it).
void invoke<string | null>('banner_take_pending').then((raw) => {
  if (raw != null) showEnvelope(raw)
})
