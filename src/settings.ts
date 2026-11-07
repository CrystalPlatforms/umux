// settings — pure types + defaults for the feature toggles (v0.2 Phase 3 / #27).
//
// The persisted form lives in the Rust SettingsStore (settings.json); this
// module only mirrors the wire shape so the frontend and backend agree
// byte-for-byte (camelCase keys). Defaults match the Rust `Settings::default`:
// notifications on, agent status on, session restore on, ports tooltip on
// (#43). Analytics is ALWAYS ON with no flag anywhere (quickupdate
// 2026-09-12, Adam — the old analyticsEnabled kill switch is gone; a stale
// `analyticsEnabled` key in an old settings.json is ignored by both sides).

export type Settings = {
  notificationsEnabled: boolean
  agentStatusEnabled: boolean
  sessionRestoreEnabled: boolean
  portsTooltipEnabled: boolean
  // #60: written by `umux config set default-launch-mode`; the v1.7.0 TUI
  // launcher reads it. Carried through here so an app save never erases a
  // CLI-written value. Defaults to "gui" (Rust Settings::default).
  defaultLaunchMode: string
  // #80 (v1.6.0): shows the git-branch labels on tab rows. ON by default —
  // OFF blanks them display-only; the branch refresh keeps running.
  showTabBranch: boolean
  // #81 (v1.6.0): one folder line per tab (agent chip + folder) on each
  // workspace row in the sidebar. Default off = rows look exactly as before.
  showTabFolders: boolean
  // #77 (v1.6.0): the default shell every newly opened LOCAL tab spawns
  // through. null = "Auto" — the backend fallback chain untouched (today's
  // behavior). A non-empty string is a shell path / custom command, passed
  // to pty_open verbatim. SSH tabs never read it.
  defaultShell: string | null
  // Quickupdate 2026-09-12: the sidebar's dragged width in px, persisted so
  // a resize survives restart. null = the CSS default; the shell clamps the
  // applied value to its own min/max at render time (a width dragged in a
  // larger window must not eat a smaller one).
  sidebarWidth: number | null
  // #86 (v1.7.0): the umux Storestation block. daemonEnabled = the daemon
  // toggle, default OFF — Storestation is opt-in and OFF behaves exactly
  // like the pre-Storestation app. Additive: a pre-#86 settings.json (or a
  // hand-trimmed one) loads with the daemon off. autostartEnabled (#89,
  // v1.7.0 phase 7) = the second toggle, default OFF — the OS starts the
  // daemon headless at login; the setting only REMEMBERS the choice (the
  // mechanism itself is installed/removed by the dedicated invoke).
  storestation: { daemonEnabled: boolean; autostartEnabled: boolean }
}

export const defaultSettings: Settings = {
  notificationsEnabled: true,
  agentStatusEnabled: true,
  sessionRestoreEnabled: true,
  portsTooltipEnabled: true,
  defaultLaunchMode: 'gui',
  showTabBranch: true,
  showTabFolders: false,
  defaultShell: null,
  sidebarWidth: null,
  storestation: { daemonEnabled: false, autostartEnabled: false },
}

/// Coerce an unknown invoke payload into a complete Settings object: missing
/// keys fall back to the defaults, so a partial/hand-edited settings.json can
/// never introduce `undefined` into the toggle state.
export function coerceSettings(raw: unknown): Settings {
  const r = (raw ?? {}) as Partial<Settings>
  return {
    notificationsEnabled: r.notificationsEnabled ?? defaultSettings.notificationsEnabled,
    agentStatusEnabled: r.agentStatusEnabled ?? defaultSettings.agentStatusEnabled,
    sessionRestoreEnabled: r.sessionRestoreEnabled ?? defaultSettings.sessionRestoreEnabled,
    portsTooltipEnabled: r.portsTooltipEnabled ?? defaultSettings.portsTooltipEnabled,
    defaultLaunchMode: r.defaultLaunchMode ?? defaultSettings.defaultLaunchMode,
    showTabBranch: r.showTabBranch ?? defaultSettings.showTabBranch,
    showTabFolders: r.showTabFolders ?? defaultSettings.showTabFolders,
    // #77: null is a MEANINGFUL value here (Auto), so the usual `?? default`
    // pattern would erase an explicit Auto on reload — coerce by shape
    // instead: a non-blank string survives, everything else is Auto.
    defaultShell:
      typeof r.defaultShell === 'string' && r.defaultShell.trim() !== ''
        ? r.defaultShell
        : null,
    // Quickupdate 2026-09-12: same shape coercion as defaultShell — only a
    // positive number is a width; junk (or a legacy file without the key)
    // falls back to null = the CSS default. Rounded to whole px (2026-09-16):
    // the backend's u32 field rejects floats, and a fractional payload would
    // fail the entire save_settings write.
    sidebarWidth:
      typeof r.sidebarWidth === 'number' && Number.isFinite(r.sidebarWidth) && r.sidebarWidth > 0
        ? Math.round(r.sidebarWidth)
        : null,
    // #86: coerce by shape — the block itself OR just the flag may be
    // missing (a pre-#86 file); anything else defaults to OFF. The phase-7
    // (#89) autostart flag coerces the same way (a pre-#89 file loads with
    // autostart off).
    storestation: {
      daemonEnabled: r.storestation?.daemonEnabled === true,
      autostartEnabled: r.storestation?.autostartEnabled === true,
    },
  }
}

// --- CLI-facing settings surface (#127, v1.8.0 phase 5a) ---------------------
//
// One mapping between the CLI's kebab-case keys and the Settings fields, so
// `umux settings set` and the UI toggle CANNOT diverge: both go through
// applySettingByKey (the gateway op turns its patch into the SAME
// applySettings call a click uses). The daemon-domain switches (Storestation
// enable/autostart, Core, Core Session) are NOT here on purpose — they are
// daemon-owned state served by the existing `storestation`/`core`/
// `core-session` commands, and a settings key would suggest the app owned
// them.

/// The app-domain keys the CLI surface covers, kebab-case.
export const SETTING_KEYS: ReadonlyArray<string> = [
  'notifications-enabled',
  'agent-status-enabled',
  'session-restore-enabled',
  'ports-tooltip-enabled',
  'default-launch-mode',
  'show-tab-branch',
  'show-tab-folders',
  'default-shell',
  'sidebar-width',
]

export type SettingError = {
  code: string
  message: string
  next: string[]
}

export type SettingPatchOutcome =
  | { ok: true; patch: Partial<Settings>; value: string }
  | { ok: false; error: SettingError }

const strictBool = (value: string): boolean | null =>
  value === 'true' ? true : value === 'false' ? false : null

/// Validate + coerce one `settings set` (the UI toggle's twin). The error
/// codes mirror the CLI catalog: an unknown key is settingNotFound, a bad
/// value is badParams.
export function applySettingByKey(
  key: string,
  value: string,
): SettingPatchOutcome {
  const badValue = (message: string): SettingPatchOutcome => ({
    ok: false,
    error: { code: 'badParams', message, next: [] },
  })
  switch (key) {
    case 'notifications-enabled':
    case 'agent-status-enabled':
    case 'session-restore-enabled':
    case 'ports-tooltip-enabled': {
      const flag = strictBool(value)
      if (flag == null) {
        return badValue(`invalid value "${value}" for a boolean setting — use true or false`)
      }
      const camel = {
        'notifications-enabled': 'notificationsEnabled',
        'agent-status-enabled': 'agentStatusEnabled',
        'session-restore-enabled': 'sessionRestoreEnabled',
        'ports-tooltip-enabled': 'portsTooltipEnabled',
      }[key] as keyof Settings
      return { ok: true, patch: { [camel]: flag } as Partial<Settings>, value }
    }
    case 'default-launch-mode': {
      if (value !== 'gui' && value !== 'tui') {
        return badValue(`invalid launch mode "${value}" — use gui or tui`)
      }
      return { ok: true, patch: { defaultLaunchMode: value }, value }
    }
    case 'show-tab-branch':
    case 'show-tab-folders': {
      const flag = strictBool(value)
      if (flag == null) {
        return badValue(`invalid value "${value}" for a boolean setting — use true or false`)
      }
      return key === 'show-tab-branch'
        ? { ok: true, patch: { showTabBranch: flag }, value }
        : { ok: true, patch: { showTabFolders: flag }, value }
    }
    case 'default-shell': {
      // "auto" (case-insensitive) clears back to the fallback chain; any
      // non-blank string is a shell path / custom command, verbatim.
      const trimmed = value.trim()
      if (trimmed.toLowerCase() === 'auto') {
        return { ok: true, patch: { defaultShell: null }, value: 'auto' }
      }
      if (trimmed === '') {
        return {
          ok: false,
          error: {
            code: 'badParams',
            message: 'default-shell needs a shell path/command, or "auto"',
            next: [],
          },
        }
      }
      return { ok: true, patch: { defaultShell: trimmed }, value: trimmed }
    }
    case 'sidebar-width': {
      // "auto" (case-insensitive) clears back to the CSS default, mirroring
      // the CLI's offline writer.
      if (value.trim().toLowerCase() === 'auto') {
        return { ok: true, patch: { sidebarWidth: null }, value: 'auto' }
      }
      const n = Number(value)
      if (!Number.isInteger(n) || n <= 0) {
        return {
          ok: false,
          error: {
            code: 'badParams',
            message: `invalid value "${value}" for sidebar-width — a whole number of pixels`,
            next: [],
          },
        }
      }
      return { ok: true, patch: { sidebarWidth: n }, value: String(n) }
    }
    default:
      return {
        ok: false,
        error: {
          code: 'settingNotFound',
          message: `unknown setting "${key}"`,
          next: [`settings: ${SETTING_KEYS.join(', ')}`],
        },
      }
  }
}

/// Read one key for `settings get` — the stringified value the CLI prints,
/// or null for an unknown key (the caller answers settingNotFound).
export function readSettingByKey(settings: Settings, key: string): string | null {
  switch (key) {
    case 'notifications-enabled':
      return String(settings.notificationsEnabled)
    case 'agent-status-enabled':
      return String(settings.agentStatusEnabled)
    case 'session-restore-enabled':
      return String(settings.sessionRestoreEnabled)
    case 'ports-tooltip-enabled':
      return String(settings.portsTooltipEnabled)
    case 'default-launch-mode':
      return settings.defaultLaunchMode
    case 'show-tab-branch':
      return String(settings.showTabBranch)
    case 'show-tab-folders':
      return String(settings.showTabFolders)
    case 'default-shell':
      return settings.defaultShell ?? 'auto'
    case 'sidebar-width':
      return settings.sidebarWidth == null ? 'auto' : String(settings.sidebarWidth)
    default:
      return null
  }
}

/// Every app-domain key with its current value — the `settings get` (no key)
/// document.
export function readAllSettings(settings: Settings): Record<string, string> {
  return Object.fromEntries(SETTING_KEYS.map((k) => [k, readSettingByKey(settings, k) ?? '']))
}
