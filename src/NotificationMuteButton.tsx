// NotificationMuteButton — the notification mute toggle (Phase 14 / #15).
//
// Presentational component: it owns no state and makes no Tauri calls. The mute
// state and the invoke('set_notifications_muted') call live in WorkspaceShell
// (UI glue, verified manually by Adam). This component is the small, testable
// surface: it shows whether notifications are muted (AC3) and asks the parent to
// toggle on click (AC1).
//
// Accessibility: aria-pressed mirrors `muted` so the state is exposed to assistive
// tech; the aria-label says "Mute notifications" (action available) when currently
// audible and "Notifications muted" (current state) when silenced.

type Props = {
  muted: boolean
  onToggle: () => void
}

export function NotificationMuteButton({ muted, onToggle }: Props) {
  const label = muted ? 'Notifications muted' : 'Mute notifications'
  return (
    <button
      type="button"
      className="mute-button"
      aria-pressed={muted}
      aria-label={label}
      title={label}
      onClick={onToggle}
    >
      {/* Bell icon. When muted it is shown struck-through (line through the
          bell) via the .is-muted CSS modifier; the icon itself stays the same so
          the control's identity is stable. */}
      <svg
        className={`mute-button__icon${muted ? ' is-muted' : ''}`}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
        <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
      </svg>
    </button>
  )
}
