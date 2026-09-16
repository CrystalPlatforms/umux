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
  }
}
