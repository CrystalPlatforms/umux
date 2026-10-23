// notificationRouting — click-to-navigate types (#76 follow-up, #117 rework).
//
// Navigation fires ONLY on a real click signal the backend reports as
// `notification_activated`:
//  - Linux: the banner's "Open" action button (the backend waits on
//    notify-send and reports the exact click).
//  - Windows: the OS relaunches the exe with `-ToastActivated`; the
//    single-instance callback turns that argv into the same event, aimed at
//    the most recently POSTED notification (the argv carries no banner id).
// The old "next window focus within 60 s reads as a click" heuristic is gone
// (#117): it could not tell a banner click from any other refocus, so umux
// switched tabs on plain alt-tabs before the user ever clicked. On macOS a
// banner click activates the window but offers no distinguishable signal —
// click-to-navigate there is a known gap, not a heuristic.
//
// Pure types only — no listeners, no Tauri, no clock reads.

export type NotificationTarget = {
  workspaceId: string
  tabId: string
  panelId: string
}
