// NotificationBanner — the umux-owned Windows notification banner (#118).
//
// The native toast's click is dead at the OS level on the Insider machine
// (both -ToastActivated and umux-app://), so Windows shows an umux-owned
// banner window instead (like Steam/Discord toasts). This module is the
// banner's brain: show / auto-dismiss / click — deliberately DOM-free and
// Tauri-free. The banner page's glue (bannerEntry.ts) injects hooks that map
// these decisions onto the DOM/CSS and the Tauri window, mirroring how
// notificationRouting kept its bookkeeping pure.

/// What the banner is currently displaying. `payload` is the backend's raw
/// notification payload string — it travels back out, untouched, on click so
/// the main window's existing `notification_activated` listener can navigate
/// to the exact workspace/tab/panel.
export type BannerView = {
  summary: string
  body: string
  payload: string
}

/// The controller's whole output surface, injected. The glue implements each
/// hook against the real window; tests record the calls instead.
export type BannerHooks = {
  /// Put this content on the banner (also resets any running fade).
  render(view: BannerView): void
  /// Make the banner window visible.
  showWindow(): void
  /// Start the fade-out (the glue flips a CSS class).
  onFadeOut(): void
  /// The fade finished (or a click cut it short): hide the window.
  hideWindow(): void
  /// The user clicked the banner; carries the view's raw payload.
  onActivate(payload: string): void
}

/// How long a banner stays fully visible before it starts fading.
export const BANNER_AUTO_DISMISS_MS = 5_000
/// How long the fade-out lasts before the window is hidden.
export const BANNER_FADE_OUT_MS = 250

export class NotificationBanner {
  private readonly hooks: BannerHooks
  private readonly autoDismissMs: number
  private readonly fadeOutMs: number
  private autoDismissTimer: ReturnType<typeof setTimeout> | null = null
  private fadeTimer: ReturnType<typeof setTimeout> | null = null
  private active: BannerView | null = null

  constructor(hooks: BannerHooks, autoDismissMs = BANNER_AUTO_DISMISS_MS, fadeOutMs = BANNER_FADE_OUT_MS) {
    this.hooks = hooks
    this.autoDismissMs = autoDismissMs
    this.fadeOutMs = fadeOutMs
  }

  /// Show a notification. Replaces whatever is on screen: content re-renders,
  /// any pending dismissal (auto or fade) is canceled, the timer restarts.
  show(summary: string, body: string, payload: string): void {
    this.clearTimers()
    this.active = { summary, body, payload }
    this.hooks.render(this.active)
    this.hooks.showWindow()
    this.autoDismissTimer = setTimeout(() => this.beginFade(), this.autoDismissMs)
  }

  /// The user clicked the banner: send the exact payload out and hide at
  /// once. A no-op when nothing is shown (nothing on screen to click).
  click(): void {
    const view = this.active
    if (view == null) return
    this.active = null
    this.clearTimers()
    this.hooks.onActivate(view.payload)
    this.hooks.hideWindow()
  }

  private clearTimers(): void {
    if (this.autoDismissTimer != null) {
      clearTimeout(this.autoDismissTimer)
      this.autoDismissTimer = null
    }
    if (this.fadeTimer != null) {
      clearTimeout(this.fadeTimer)
      this.fadeTimer = null
    }
  }

  private beginFade(): void {
    this.hooks.onFadeOut()
    this.fadeTimer = setTimeout(() => {
      this.fadeTimer = null
      this.hooks.hideWindow()
    }, this.fadeOutMs)
  }
}
