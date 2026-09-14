// notificationRouting — click-to-navigate bookkeeping (#76 follow-up).
//
// Assumptions encoded:
//  - A desktop notification that carries a navigation payload is remembered
//    (with WHEN it was posted). When the app window regains focus shortly
//    after, that read as the user CLICKING the banner: the pending target is
//    consumed and navigation should go to it. macOS/Windows banners activate
//    the app on click, so this covers them; Linux uses explicit actions.
//  - The pending entry is single-shot: one focus consumes it (no repeated
//    yanking on later alt-tabs).
//  - Stale entries (older than the focus window) never navigate — a ping
//    seen an hour ago must not hijack the next alt-tab into the app.
//  - Only OSes whose banner click ACTIVATES the app get the heuristic: the
//    user agent contains the platform; Linux notify-send banners do nothing
//    on click (their "Open" action button is the real path).
//  - NOT tested here: the WorkspaceShell listeners/dispatch (glue, HITL).

import { describe, it, expect } from 'vitest'
import {
  PendingNotificationTarget,
  activatesAppOnNotificationClick,
  NOTIFICATION_FOCUS_WINDOW_MS,
} from './notificationRouting'

const payload = '{"kind":"waiting","workspaceId":"ws-1","tabId":"tab-2","panelId":"p-3"}'

describe('PendingNotificationTarget', () => {
  it('delivers a fresh pending payload on focus, exactly once', () => {
    const pending = new PendingNotificationTarget()
    pending.set(payload, 1000)

    expect(pending.take(2000)).toBe(payload)
    expect(pending.take(2001)).toBeNull()
    expect(pending.take(2002)).toBeNull()
  })

  it('delivers nothing when no notification is pending', () => {
    const pending = new PendingNotificationTarget()

    expect(pending.take(5000)).toBeNull()
  })

  it('drops a payload older than the focus window', () => {
    const pending = new PendingNotificationTarget()
    pending.set(payload, 1000)

    expect(pending.take(1000 + NOTIFICATION_FOCUS_WINDOW_MS + 1)).toBeNull()
    // Even a later, in-window take stays empty — staleness consumed it.
    expect(pending.take(1000 + NOTIFICATION_FOCUS_WINDOW_MS + 2)).toBeNull()
  })

  it('a newer notification replaces the older pending one', () => {
    const pending = new PendingNotificationTarget()
    pending.set('{"older":true}', 1000)
    pending.set(payload, 2000)

    expect(pending.take(2500)).toBe(payload)
  })
})

describe('activatesAppOnNotificationClick', () => {
  it('macOS and Windows banners activate the app; Linux ones do not', () => {
    expect(
      activatesAppOnNotificationClick(
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15',
      ),
    ).toBe(true)
    expect(
      activatesAppOnNotificationClick('Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120'),
    ).toBe(true)
    expect(
      activatesAppOnNotificationClick('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15'),
    ).toBe(false)
  })
})
