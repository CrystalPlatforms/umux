// notificationRouting — click-to-navigate bookkeeping (#76 follow-up).
//
// A desktop notification now carries a navigation payload (which workspace /
// tab / panel it came from). Turning a CLICK into navigation is split by what
// the OS can tell us:
//  - macOS / Windows: clicking a banner ACTIVATES the app — indistinguishable
//    from an alt-tab at the app level, so the glue navigates to the most
//    recent pending notification on window focus, inside a short window, and
//    only once (this module: PendingNotificationTarget).
//  - Linux: notify-send banners carry a real "Open" action button whose click
//    the backend reports — navigation is exact, no heuristic (the backend
//    emits `notification_activated`).
// Pure bookkeeping only — no listeners, no Tauri, no clock reads.

export type NotificationTarget = {
  workspaceId: string
  tabId: string
  panelId: string
}

/// How long after a post a window focus still counts as "the user clicked the
/// banner". Long enough to cover reading the banner, short enough that an
/// unrelated alt-tab back into umux is not hijacked.
export const NOTIFICATION_FOCUS_WINDOW_MS = 60_000

/// Do THIS platform's notification banners activate the app when clicked?
/// macOS and Windows do (the focus heuristic applies); Linux banners belong
/// to the notification daemon and do nothing on a body click (Linux uses the
/// explicit action button instead). The user agent carries the platform.
export function activatesAppOnNotificationClick(userAgent: string): boolean {
  return /macintosh|windows/i.test(userAgent)
}

/// The one pending notification a focus may navigate to. Single-shot: a take
/// consumes the entry whether or not it was still fresh, so one ping can
/// never yank the user twice. The entry is the notification's RAW payload
/// string — resolving it to a panel is the glue's job (one parse path, at
/// take time, against the CURRENT state).
export class PendingNotificationTarget {
  private pending: { raw: string; at: number } | null = null

  /** A notification with this raw payload was just posted at `at`. */
  set(raw: string, at: number): void {
    this.pending = { raw, at }
  }

  /** The window regained focus at `now`: the pending raw payload if it was
   *  posted within the focus window, else null — and the entry is consumed
   *  either way. */
  take(now: number): string | null {
    const entry = this.pending
    this.pending = null
    if (entry == null) return null
    if (now - entry.at > NOTIFICATION_FOCUS_WINDOW_MS) return null
    return entry.raw
  }
}
