// SettingsDialog — the feature-toggle screen (v0.2 Phase 3 / #27).
//
// Presentational modal: it owns no Tauri calls and no app state. The
// settings object and the invoke('save_settings') flow live in WorkspaceShell
// (UI glue, verified manually by Adam); this component is the small,
// testable surface: it renders one switch per toggle (AC1) and reports
// changes upward, where they take effect immediately (AC2/AC3).
//
// NOTE: there is deliberately NO analytics switch — analytics is ALWAYS ON
// with no flag anywhere (quickupdate 2026-09-12, Adam): the old
// `analyticsEnabled` kill switch is gone from the schema on both sides, and
// a stale key in an old settings.json is ignored.
//
// Import row (#59, HITL rework 2026-08-30): the "from cmux" item now OPENS
// the import wizard (CmuxImportWizard — scan → choose with a live preview →
// apply) instead of applying immediately; this dialog only reports the
// outcome through its status line. Hidden on Windows (v1.2.0 decision #4).
//
// Accessibility: each switch is a real <button role="switch"> with
// aria-checked mirroring the state, so assistive tech announces it as a
// toggle. Escape and the header X close the dialog.
//
// Storestation sub-view (2026-10-02 rework, Adam): the two Storestation
// switches and the live status moved OUT of the main settings page into a
// dedicated view, reached through one full-width "umux Storestation" entry
// button. The view's own Back button and Escape both return to the main
// page first — one dismissal reflex per level.
//
// Core sub-view (v1.7.5 phase 2, #94): same pattern, one level deeper in
// the story — "Core (Always-On device)". The switch does NOT live in
// settings.json: the daemon owns the Core flag (it must survive restarts
// with every window closed), so this dialog only MIRRORS the daemon's
// reported state and reports toggles upward. The instruction string and
// the toggle error render in the view — nothing is ever silent.

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { defaultSettings, type Settings } from './settings'
import { isWindowsPlatform } from './importWizard'
import { downloadProgressText, type UpdateState } from './updater'
import { pickerOptions, type ShellEntry } from './shellDetector'

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
  onResetAll,
  updates,
  shells = [],
  onStorestationToggle,
  onStorestationAutostartToggle,
  storestationStatus = null,
  onCoreToggle,
  coreStatus = null,
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
  // Factory reset (#74): clicking asks the parent to wipe EVERY umux state
  // file and relaunch first-run clean. Presentational here — the invoke and
  // the relaunch live in WorkspaceShell. The button arms on the FIRST click
  // (its label changes) and only the SECOND click fires, so a stray press
  // can never wipe the store on its own.
  onResetAll?: () => void
  // The update row (issue #66): presentational only — the plugin flows live
  // in updater.ts and the glue in WorkspaceShell. Absent = the row is not
  // rendered at all.
  updates?: {
    state: UpdateState
    onCheck: () => void
    onInstall: () => void
  }
  // The detected-shell list (#77, v1.6.0): raw probes already ranked by the
  // pure ShellDetector (the WorkspaceShell glue runs the `list_shells`
  // invoke). Empty = only "Auto" and the custom entry are offered — no shell
  // is ever assumed to exist.
  shells?: ShellEntry[]
  // The Storestation section (#86, v1.7.0): the daemon toggle + a live
  // status line. The toggle reports UPWARD (this component stays
  // invoke-free) — the parent runs the spawn/stop flow, decides whether a
  // confirmation is needed (live sessions die on stop), and persists the
  // setting after the backend answers. Absent = the section is not rendered.
  onStorestationToggle?: (next: boolean) => void
  // The autostart toggle (#89, v1.7.0 phase 7): the second Storestation
  // control. Same upward-reporting contract as the daemon toggle — the
  // parent runs the OS install/remove (Run key / LaunchAgent / systemd
  // unit) and persists the setting. Absent = the row is not rendered.
  onStorestationAutostartToggle?: (next: boolean) => void
  // The last probed daemon status (the parent refreshes it when the dialog
  // opens and after every toggle). null = no status line yet.
  storestationStatus?: {
    enabled: boolean
    running: boolean
    version?: string
    sessions?: number
    attachedClients?: number
  } | null
  // The Core (Always-On device) section (#94, v1.7.5 phase 2): the toggle
  // reports UPWARD like every other control (this component stays
  // invoke-free) — the parent spawns the daemon if absent, sends core.set,
  // and only adopts the daemon's answer on success. The mirror state is
  // the DAEMON's view (enabled/held/instruction), never local UI state;
  // `error` carries a failed op so the view can surface it. Absent = the
  // section is not rendered.
  onCoreToggle?: (next: boolean) => void
  coreStatus?: {
    enabled: boolean
    held: boolean
    instruction: string | null
    error: string | null
  } | null
}) {
  // The sub-views (2026-10-02 rework, Adam): 'main' is the regular settings
  // page; 'storestation', 'core' and 'reset' are the dedicated screens the
  // entry buttons open — each REPLACES the whole settings page. Local
  // presentational state only — WorkspaceShell renders this dialog
  // conditionally, so every fresh open starts on 'main'.
  const [view, setView] = useState<'main' | 'storestation' | 'core' | 'reset'>('main')

  // Custom shell entry (#77, fix round 2): the "Custom…" menu item opens a
  // small dialog with the command field. The field prefills with the saved
  // command when one is in effect and it is not already a detected entry.
  // "Use" applies it trimmed; empty does nothing — clearing back to Auto is
  // the picker's "Auto" entry, not an empty command.
  const [customOpen, setCustomOpen] = useState(false)
  const [customShell, setCustomShell] = useState(() => {
    const saved = settings.defaultShell
    if (saved == null) return ''
    return shells.some((s) => s.launchCommand === saved) ? '' : saved
  })

  // Escape closes the dialog — the same dismissal key the rename/create
  // inputs use, so the app has one "back out" reflex everywhere. While the
  // custom-shell dialog is open, Escape closes ONLY that dialog; inside a
  // sub-view (Storestation / Core / reset), Escape is the BACK key — one
  // level per press, returning to the main settings page.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (customOpen) {
        setCustomOpen(false)
        return
      }
      if (view !== 'main') {
        setView('main')
        return
      }
      onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, customOpen, view])

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

  // The Default-shell picker (#77) reuses that exact dropdown interaction —
  // open state + close on any press outside it. The menu is FIXED-positioned
  // from the button's box (fix round 2026-09-11): the settings card has
  // overflow-y: auto, which used to clip the absolute dropdown at the card
  // wall when a long entry (a WSL distro command) widened it. maxWidth keeps
  // the escaped menu on screen; rows ellipsis and carry the full label as
  // their title tooltip.
  const [shellPickerOpen, setShellPickerOpen] = useState(false)
  const [shellPickerPos, setShellPickerPos] = useState({ top: 0, left: 0, maxWidth: 480 })
  const shellPickerRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (!shellPickerOpen) return
    const close = (e: PointerEvent) => {
      if (
        shellPickerRef.current != null &&
        e.target instanceof Node &&
        shellPickerRef.current.contains(e.target)
      ) {
        return
      }
      setShellPickerOpen(false)
    }
    window.addEventListener('pointerdown', close)
    return () => window.removeEventListener('pointerdown', close)
  }, [shellPickerOpen])

  // Reset arming (#74): the first click ARMS (label changes to the confirm
  // question), only the second fires the reset. Two deliberate clicks — a
  // stray press can never wipe the store.
  const [resetArmed, setResetArmed] = useState(false)

  // The picker's menu rows and the label the button shows for the value in
  // effect ("Auto" when nothing is set). pickerOptions already appends a
  // saved custom command, so the current value is always on the list.
  const shellOptions = pickerOptions(shells, settings.defaultShell)
  const currentShell =
    shellOptions.find((o) => o.value === (settings.defaultShell ?? null)) ?? shellOptions[0]
  const currentShellLabel = currentShell?.label ?? 'Auto'

  // Flip (fix round 3, 2026-09-11 — replaces the shrink/scroll attempt): the
  // menu opens BELOW the button by default, but never past the umux window's
  // bottom edge — when there is no room below it opens ABOVE the button
  // instead (that side with more room when neither fits). Measured in a
  // layout effect, before paint, so the flip never flickers; the state bail
  // keeps re-renders from looping.
  const pickerMenuRef = useRef<HTMLDivElement | null>(null)
  const pickerAnchor = useRef<DOMRect | null>(null)
  useLayoutEffect(() => {
    if (!shellPickerOpen) return
    const a = pickerAnchor.current
    const menu = pickerMenuRef.current
    if (a == null || menu == null) return
    const h = menu.offsetHeight
    const below = window.innerHeight - a.bottom - 4
    const above = a.top - 4
    let top = a.bottom + 4
    if (h > below && (h <= above || above >= below)) {
      top = Math.max(4, a.top - h - 4)
    }
    setShellPickerPos((p) => (p.top === top ? p : { ...p, top }))
  })

  return (
    <>
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
          <span className="modal-card__title settings-dialog__title">
            {view === 'storestation'
              ? 'umux Storestation'
              : view === 'core'
                ? 'umux Core'
                : view === 'reset'
                  ? 'Reset umux'
                  : 'Settings'}
          </span>
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

        {/* The main settings page (2026-10-02 rework, Adam): EVERYTHING
            below — switches, shell picker, updates, import, reset, footnote
            — hides while the Storestation view is open. That view shows only
            Back and the Storestation controls, nothing else. */}
        {view === 'main' && (
          <>
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
        {/* #80 (v1.6.0): hides the git-branch labels on tab rows. */}
        <SettingsToggle
          label="Git branch on tab rows"
          checked={settings.showTabBranch}
          testId="toggle-show-tab-branch"
          onToggle={(next) => onChange({ showTabBranch: next })}
        />
        {/* #81 (v1.6.0): one line per tab on workspace rows — chip + folder. */}
        <SettingsToggle
          label="Show per-tab folders on workspaces"
          checked={settings.showTabFolders}
          testId="toggle-show-tab-folders"
          onToggle={(next) => onChange({ showTabFolders: next })}
        />

        {/* Default shell (#77, v1.6.0): what every newly opened LOCAL tab
            spawns through. The picker is the SAME dropdown the sidebar "+"
            uses — a press-styled button unfolding a .create-dropdown menu —
            listing "Auto" (null = the backend fallback chain, today's
            behavior) plus the shells the pure ShellDetector ranked from the
            raw probes; the "Custom…" item opens the command dialog. A value
            rides pty_open verbatim. */}
        <div className="settings-row">
          <div className="settings-row__text">
            <span className="settings-row__label">Default shell</span>
            <span className="settings-row__description">
              Shell for newly opened tabs. SSH tabs keep the remote default.
            </span>
          </div>
          <div className="settings-import" ref={shellPickerRef}>
            <button
              type="button"
              className="btn-primary shell-picker-button"
              data-testid="shell-picker"
              aria-label="Default shell"
              aria-haspopup="menu"
              aria-expanded={shellPickerOpen}
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect()
                pickerAnchor.current = r
                setShellPickerPos({
                  top: r.bottom + 4,
                  left: r.left,
                  // Stay on screen however wide the entries are (the menu may
                  // hang past the card — that is the point); vertical fitting
                  // is the flip logic's job, not a width shrink.
                  maxWidth: Math.min(480, window.innerWidth - r.left - 8),
                })
                setShellPickerOpen((o) => !o)
              }}
            >
              {currentShellLabel}
            </button>
            {shellPickerOpen && (
              <div
                ref={pickerMenuRef}
                className="create-dropdown settings-shell-dropdown"
                role="menu"
                data-testid="shell-picker-menu"
                style={{
                  top: shellPickerPos.top,
                  left: shellPickerPos.left,
                  maxWidth: shellPickerPos.maxWidth,
                }}
              >
                {shellOptions.map((o) => (
                  <button
                    key={o.value ?? '__auto'}
                    className="menu-item"
                    role="menuitem"
                    title={o.label}
                    onClick={() => {
                      setShellPickerOpen(false)
                      onChange({ defaultShell: o.value })
                    }}
                  >
                    {o.label}
                  </button>
                ))}
                {/* Custom… (fix round 2): a menu item, not a permanent row —
                    picking it opens the small command dialog below. */}
                <button
                  className="menu-item"
                  role="menuitem"
                  data-testid="shell-custom-item"
                  onClick={() => {
                    setShellPickerOpen(false)
                    setCustomOpen(true)
                  }}
                >
                  Custom…
                </button>
              </div>
            )}
          </div>
        </div>

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
        {/* Close of the view==='main' fragment — everything above is the
            main settings page only. */}
        </>
      )}

        {/* umux Storestation (#86, v1.7.0; 2026-10-02 rework, Adam): the
            main settings page keeps only ONE entry control — a full-width,
            plain-text button in the workspace folder-open button's style
            (no description line). It opens the dedicated Storestation view
            below. Sits OUTSIDE the view==='main' fragment — opening it
            replaces the whole settings page. The parent still owns the whole
            flow: toggle ON spawns/connects the daemon before the setting
            persists; toggle OFF with live sessions asks for a confirmation
            THERE (this component stays invoke-free). */}
        {onStorestationToggle != null && view === 'main' && (
          <button
            type="button"
            className="settings-nav-entry"
            data-testid="storestation-open"
            aria-label="umux Storestation"
            onClick={() => setView('storestation')}
          >
            umux Storestation
          </button>
        )}

        {/* umux Core (v1.7.5 phase 2, #94): the second entry button, same
            full-width plain-text pattern — "umux Core (Always-On)". Like
            Storestation, opening it replaces the whole settings page; the
            parent owns the whole flow (spawn daemon if absent → core.set →
            mirror the daemon's answer). */}
        {onCoreToggle != null && view === 'main' && (
          <button
            type="button"
            className="settings-nav-entry"
            data-testid="core-open"
            aria-label="umux Core (Always-On)"
            onClick={() => setView('core')}
          >
            umux Core (Always-On)
          </button>
        )}

        {/* The Storestation view itself (2026-10-02 rework, Adam): the Back
            button, the two switches and the live status line — nothing else,
            the main settings page is fully replaced. Left via Back or
            Escape. Same upward contracts as before — nothing about the flow
            changed, only where the controls live. */}
        {view === 'storestation' && onStorestationToggle != null && (
          <>
            <button
              type="button"
              className="settings-nav-entry"
              data-testid="settings-back"
              onClick={() => setView('main')}
            >
              ← Back
            </button>
            <div className="settings-row" data-testid="storestation-daemon-row">
              <div className="settings-row__text">
                <span className="settings-row__label">umux Storestation</span>
                <span className="settings-row__description">
                  Keep terminal sessions running after umux closes. When off,
                  umux behaves exactly as before.
                </span>
              </div>
              <SettingsToggle
                label="umux Storestation daemon"
                checked={settings.storestation.daemonEnabled}
                testId="toggle-storestation"
                onToggle={onStorestationToggle}
              />
            </div>
            {/* The second control (#89, story 108 complete): start the
                daemon headless at login. Same pessimistic flow as the
                daemon toggle — the parent installs/removes the OS mechanism
                first, and only success persists the switch. */}
            {onStorestationAutostartToggle != null && (
              <div className="settings-row" data-testid="storestation-autostart-row">
                <div className="settings-row__text">
                  <span className="settings-row__label">Start daemon at login</span>
                  <span className="settings-row__description">
                    Launches the umux Storestation daemon in the background
                    when you log in, no window, sessions keep running even
                    before umux opens.
                  </span>
                </div>
                <SettingsToggle
                  label="umux Storestation autostart"
                  checked={settings.storestation.autostartEnabled}
                  testId="toggle-storestation-autostart"
                  onToggle={onStorestationAutostartToggle}
                />
              </div>
            )}
            {storestationStatus != null && (
              <p className="settings-status" data-testid="storestation-status">
                {storestationStatus.running
                  ? `Daemon running${storestationStatus.version ? ` (v${storestationStatus.version})` : ''} — ${storestationStatus.sessions ?? 0} session${(storestationStatus.sessions ?? 0) === 1 ? '' : 's'}.`
                  : 'Daemon stopped.'}
              </p>
            )}
          </>
        )}

        {/* The Core view itself (v1.7.5 phase 2, #94): Back, the Always-On
            switch, the daemon status line and (when present) the platform
            instruction — nothing else, same shape as the Storestation view.
            The switch mirrors the DAEMON's reported state (coreStatus), not
            a local setting: a failed op leaves the mirror untouched and the
            error renders below, so nothing is ever silent. */}
        {view === 'core' && onCoreToggle != null && (
          <>
            <button
              type="button"
              className="settings-nav-entry"
              data-testid="settings-back"
              onClick={() => setView('main')}
            >
              ← Back
            </button>
            <div className="settings-row" data-testid="core-row">
              <div className="settings-row__text">
                <span className="settings-row__label">umux Core (Always-On)</span>
                <span className="settings-row__description">
                  Keeps your machine awake, even if you close the laptop
                  cover. The screen may turn off.
                </span>
              </div>
              <SettingsToggle
                label="umux Core (Always-On)"
                checked={coreStatus?.enabled ?? false}
                testId="toggle-core"
                onToggle={onCoreToggle}
              />
            </div>
            {storestationStatus != null && (
              <p className="settings-status" data-testid="core-status">
                {storestationStatus.running
                  ? `Daemon running${storestationStatus.version ? ` (v${storestationStatus.version})` : ''} — sleep prevention ${coreStatus?.held ? 'held.' : 'NOT held.'}`
                  : 'Daemon stopped.'}
              </p>
            )}
            {coreStatus?.instruction != null && coreStatus.instruction !== '' && (
              <p className="settings-status" data-testid="core-instruction">
                {coreStatus.instruction}
              </p>
            )}
            {coreStatus?.error != null && coreStatus.error !== '' && (
              <p className="settings-status settings-status--error" data-testid="core-error">
                {coreStatus.error}
              </p>
            )}
          </>
        )}

        {/* Factory reset (#74; 2026-10-02 rework, Adam — same shape as the
            Storestation view): the main page keeps only the "Reset umux…"
            entry button UNDER the Storestation one. Clicking opens a
            dedicated screen with the description and the two-click confirm;
            the arm/confirm flow itself is unchanged. */}
        {onResetAll != null && view === 'main' && (
          <button
            type="button"
            className="settings-nav-entry"
            data-testid="reset-open"
            aria-label="Reset umux"
            onClick={() => setView('reset')}
          >
            Reset umux…
          </button>
        )}
        {view === 'reset' && onResetAll != null && (
          <>
            <button
              type="button"
              className="settings-nav-entry"
              data-testid="settings-back"
              onClick={() => setView('main')}
            >
              ← Back
            </button>
            <div className="settings-row" data-testid="reset-row">
              <div className="settings-row__text">
                <span className="settings-row__label">Reset umux</span>
                <span className="settings-row__description">
                  Removes all workspaces, layouts and settings, then restarts
                  umux. This cannot be undone.
                </span>
              </div>
              <div className="settings-import">
                <button
                  type="button"
                  className={resetArmed ? 'btn-danger' : 'btn-secondary'}
                  data-testid="reset-button"
                  onClick={() => {
                    if (!resetArmed) {
                      setResetArmed(true)
                      return
                    }
                    setResetArmed(false)
                    onResetAll()
                  }}
                >
                  {resetArmed ? 'Really reset everything?' : 'Reset umux…'}
                </button>
              </div>
            </div>
          </>
        )}

        {/* The footnote is ALWAYS the last line of the dialog — on the main
            page and inside both sub-views (2026-10-02, Adam). */}
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

      {/* Custom shell command dialog (#77, fix round 2): opened by the
          picker's "Custom…" menu item. Escape/overlay/Cancel back out without
          changing anything; Use applies the command trimmed. */}
      {customOpen && (
        <div
          className="modal-overlay"
          role="dialog"
          aria-modal="true"
          aria-label="Custom shell command"
          data-testid="shell-custom-dialog"
          onClick={() => setCustomOpen(false)}
        >
          <div className="modal-card" onClick={(e) => e.stopPropagation()}>
            <div className="modal-card__header">
              <span className="modal-card__title">Custom shell command</span>
            </div>
            <div className="modal-card__message">
              Any command the list misses — arguments allowed. Quote paths that
              contain spaces.
            </div>
            <input
              className="text-input"
              data-testid="shell-custom-input"
              aria-label="Custom shell command"
              placeholder="e.g. wsl.exe ~"
              autoFocus
              value={customShell}
              onChange={(e) => setCustomShell(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  const next = customShell.trim()
                  if (next !== '') {
                    setCustomOpen(false)
                    onChange({ defaultShell: next })
                  }
                }
              }}
            />
            <div className="modal-card__actions">
              <button
                type="button"
                className="btn-secondary"
                data-testid="shell-custom-cancel"
                onClick={() => setCustomOpen(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn-primary"
                data-testid="shell-custom-apply"
                onClick={() => {
                  const next = customShell.trim()
                  if (next !== '') {
                    setCustomOpen(false)
                    onChange({ defaultShell: next })
                  }
                }}
              >
                Use
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  )
}

// Re-export so WorkspaceShell's boot path can seed from defaults without a
// second import site; keeps the wire shape owned by one module.
export { defaultSettings }
