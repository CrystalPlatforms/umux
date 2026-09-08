// settings — pure types + defaults for the feature toggles (v0.2 Phase 3 / #27).
//
// The persisted form lives in the Rust SettingsStore (settings.json); this
// module only mirrors the wire shape so the frontend and backend agree
// byte-for-byte (camelCase keys). Defaults match the Rust `Settings::default`:
// notifications on, agent status on, session restore on, analytics on,
// ports tooltip on (#43). Analytics has NO Settings switch (product decision,
// HITL follow-up) — the field only gains meaning when Phase 6 initializes
// Aptabase.

export type Settings = {
  notificationsEnabled: boolean
  agentStatusEnabled: boolean
  sessionRestoreEnabled: boolean
  analyticsEnabled: boolean
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
}

export const defaultSettings: Settings = {
  notificationsEnabled: true,
  agentStatusEnabled: true,
  sessionRestoreEnabled: true,
  analyticsEnabled: true,
  portsTooltipEnabled: true,
  defaultLaunchMode: 'gui',
  showTabBranch: true,
  showTabFolders: false,
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
    analyticsEnabled: r.analyticsEnabled ?? defaultSettings.analyticsEnabled,
    portsTooltipEnabled: r.portsTooltipEnabled ?? defaultSettings.portsTooltipEnabled,
    defaultLaunchMode: r.defaultLaunchMode ?? defaultSettings.defaultLaunchMode,
    showTabBranch: r.showTabBranch ?? defaultSettings.showTabBranch,
    showTabFolders: r.showTabFolders ?? defaultSettings.showTabFolders,
  }
}
