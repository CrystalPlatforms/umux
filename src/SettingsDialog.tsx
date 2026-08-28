// SettingsDialog — the feature-toggle screen (v0.2 Phase 3 / #27).
//
// Presentational modal: it owns no state and makes no Tauri calls. The
// settings object and the invoke('save_settings') flow live in WorkspaceShell
// (UI glue, verified manually by Adam); this component is the small,
// testable surface: it renders one switch per toggle (AC1) and reports
// changes upward, where they take effect immediately (AC2/AC3).
//
// NOTE: there is deliberately NO analytics switch — the product decision
// (HITL follow-up) is always-on anonymous analytics with no user control, so
// the toggle Adam's issue described never shipped. The `analyticsEnabled`
// field lives on in settings.json for Phase 6's initialization, defaulting
// to true.
//
// Accessibility: each switch is a real <button role="switch"> with
// aria-checked mirroring the state, so assistive tech announces it as a
// toggle. Escape and the header X close the dialog.

import { useEffect } from 'react'
import { defaultSettings, type Settings } from './settings'

type ToggleProps = {
  label: string
  checked: boolean
  testId: string
  onToggle: (next: boolean) => void
}

function SettingsToggle({ label, checked, testId, onToggle }: ToggleProps) {
  return (
    <div className="settings-row">
      <div className="settings-row__text">
        <span className="settings-row__label">{label}</span>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        data-testid={testId}
        className={`settings-switch${checked ? ' is-on' : ''}`}
        onClick={() => onToggle(!checked)}
      >
        <span className="settings-switch__knob" />
      </button>
    </div>
  )
}

export function SettingsDialog({
  settings,
  onChange,
  onClose,
  onOpenSettingsFile,
}: {
  settings: Settings
  onChange: (patch: Partial<Settings>) => void
  onClose: () => void
  // The footnote's settings.json mention is a LINK: clicking it asks the
  // parent to open the file with the platform's default editor (the Tauri
  // invoke lives in WorkspaceShell — this component stays invoke-free).
  onOpenSettingsFile?: () => void
}) {
  // Escape closes the dialog — the same dismissal key the rename/create
  // inputs use, so the app has one "back out" reflex everywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div
      className="modal-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      data-testid="settings-dialog"
      onClick={onClose}
    >
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        <div className="modal-card__header">
          <span className="modal-card__title">Settings</span>
          <button
            type="button"
            className="icon-btn"
            aria-label="Close settings"
            title="Close settings"
            onClick={onClose}
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        </div>

        <SettingsToggle
          label="Desktop notifications"
          checked={settings.notificationsEnabled}
          testId="toggle-notifications"
          onToggle={(next) => onChange({ notificationsEnabled: next })}
        />
        <SettingsToggle
          label="Agent status indicators"
          checked={settings.agentStatusEnabled}
          testId="toggle-agent-status"
          onToggle={(next) => onChange({ agentStatusEnabled: next })}
        />
        <SettingsToggle
          label="Session restore"
          checked={settings.sessionRestoreEnabled}
          testId="toggle-session-restore"
          onToggle={(next) => onChange({ sessionRestoreEnabled: next })}
        />
        <SettingsToggle
          label="Listening-ports tooltip"
          checked={settings.portsTooltipEnabled}
          testId="toggle-ports-tooltip"
          onToggle={(next) => onChange({ portsTooltipEnabled: next })}
        />

        <div className="settings-footnote">
          Changes apply immediately and are saved to{' '}
          <button
            type="button"
            className="settings-footnote__path"
            title="Open settings.json in your default editor"
            onClick={onOpenSettingsFile}
          >
            settings.json
          </button>
          .
        </div>
      </div>
    </div>
  )
}

// Re-export so WorkspaceShell's boot path can seed from defaults without a
// second import site; keeps the wire shape owned by one module.
export { defaultSettings }
