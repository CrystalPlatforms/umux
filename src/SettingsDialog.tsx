// SettingsDialog — the feature-toggle screen (v0.2 Phase 3 / #27).
//
// Presentational modal: it owns no Tauri calls and no app state. The
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
// Import row (#59, HITL rework 2026-08-30): the "from cmux" item now OPENS
// the import wizard (CmuxImportWizard — scan → choose with a live preview →
// apply) instead of applying immediately; this dialog only reports the
// outcome through its status line. Hidden on Windows (v1.2.0 decision #4).
//
// Accessibility: each switch is a real <button role="switch"> with
// aria-checked mirroring the state, so assistive tech announces it as a
// toggle. Escape and the header X close the dialog.

import { useEffect, useRef, useState } from 'react'
import { defaultSettings, type Settings } from './settings'
import { isWindowsPlatform } from './importWizard'
import { downloadProgressText, type UpdateState } from './updater'

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

// The one-line status under the "App updates" row (issue #66). Every settled
// state reads as a human sentence; the idle state renders nothing at all.
function UpdatesStatus({ state }: { state: UpdateState }) {
  switch (state.kind) {
    case 'idle':
      return null
    case 'checking':
      return <p className="settings-status">Checking for updates…</p>
    case 'up-to-date':
      return <p className="settings-status">umux is up to date.</p>
    case 'no-release':
      return (
        <p className="settings-status">
          No update information published yet — new releases appear here once they
          ship an update file.
        </p>
      )
    case 'available':
      return <p className="settings-status">umux {state.version} is available.</p>
    case 'downloading':
      return (
        <p className="settings-status">
          Downloading update — {downloadProgressText(state.received, state.total)}
        </p>
      )
    case 'error':
      return <p className="settings-status settings-status--error">{state.message}</p>
  }
}

export function SettingsDialog({
  settings,
  onChange,
  onClose,
  onOpenSettingsFile,
  onImportWizard,
  updates,
}: {
  settings: Settings
  onChange: (patch: Partial<Settings>) => void
  onClose: () => void
  // The footnote's settings.json mention is a LINK: clicking it asks the
  // parent to open the file with the platform's default editor (the Tauri
  // invoke lives in WorkspaceShell — this component stays invoke-free).
  onOpenSettingsFile?: () => void
  // The "from cmux" menu item (#59): clicking asks the parent to OPEN the
  // import wizard (scan → choose → live preview → apply). The wizard itself
  // renders from WorkspaceShell; Apply closes BOTH dialogs, so this
  // component stays invoke-free with no outcome line of its own.
  onImportWizard?: () => void
  // The update row (issue #66): presentational only — the plugin flows live
  // in updater.ts and the glue in WorkspaceShell. Absent = the row is not
  // rendered at all.
  updates?: {
    state: UpdateState
    onCheck: () => void
    onInstall: () => void
  }
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

  // The Import dropdown (HITL round): open state + close on any press outside
  // it — the same interaction pattern as the header's "+" dropdown.
  const [importOpen, setImportOpen] = useState(false)
  const importRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (!importOpen) return
    const close = (e: PointerEvent) => {
      if (
        importRef.current != null &&
        e.target instanceof Node &&
        importRef.current.contains(e.target)
      ) {
        return
      }
      setImportOpen(false)
    }
    window.addEventListener('pointerdown', close)
    return () => window.removeEventListener('pointerdown', close)
  }, [importOpen])

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

        {/* App updates (issue #66): check on demand + one-click install.
            Both buttons live in the same right-hand slot as the Import
            button; the busy states (checking/downloading) disable them so
            two updates can never race. */}
        {updates != null && (
          <>
            <div className="settings-row">
              <div className="settings-row__text">
                <span className="settings-row__label">App updates</span>
              </div>
              <div className="settings-import">
                {updates.state.kind === 'available' && (
                  <button
                    type="button"
                    className="btn-primary"
                    data-testid="update-install"
                    onClick={updates.onInstall}
                  >
                    Download &amp; restart
                  </button>
                )}
                <button
                  type="button"
                  className="btn-primary"
                  data-testid="update-check"
                  disabled={updates.state.kind === 'checking' || updates.state.kind === 'downloading'}
                  onClick={updates.onCheck}
                >
                  Check for updates
                </button>
              </div>
            </div>
            <UpdatesStatus state={updates.state} />
          </>
        )}

        {/* Import row: same layout as the switches — label on the left, the
            action on the right. "Import" unfolds a dropdown; its "from cmux"
            item opens the import WIZARD (#59 rework). Hidden on Windows
            (v1.2.0 decision #4). */}
        {onImportWizard != null && !isWindowsPlatform() && (
          <>
            <div className="settings-row">
              <div className="settings-row__text">
                <span className="settings-row__label">
                  Import your layouts from another apps
                </span>
              </div>
              <div className="settings-import" ref={importRef}>
                <button
                  type="button"
                  className="btn-primary"
                  data-testid="import-toggle"
                  aria-expanded={importOpen}
                  onClick={() => setImportOpen((o) => !o)}
                >
                  Import
                </button>
                {importOpen && (
                  <div className="create-dropdown" role="menu">
                    <button
                      className="menu-item"
                      role="menuitem"
                      data-testid="import-cmux"
                      onClick={() => {
                        setImportOpen(false)
                        onImportWizard()
                      }}
                    >
                      from cmux
                    </button>
                  </div>
                )}
              </div>
            </div>
          </>
        )}

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
