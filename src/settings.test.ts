// settings — coercion of the persisted wire shape (#43 adds the ports
// tooltip toggle). This module had no dedicated suite before; these tests
// pin the contract WorkspaceShell's boot path depends on: an unknown/partial
// payload can never introduce `undefined` into toggle state, and every
// missing key falls back to its default (matching the Rust Settings::default
// and the per-field serde defaults).
//
// Assumptions encoded:
//  - Input: anything the invoke boundary may hand over — a full settings
//    object, a partial one (pre-#43 settings.json without the new key), an
//    empty object, null/undefined.
//  - Output: a complete Settings object; explicit values survive, missing
//    ones take the defaults (portsTooltipEnabled defaults ON, #43).
//  - NOT tested here: persistence itself (Rust SettingsStore, cargo tests).

import { describe, it, expect } from 'vitest'
import { coerceSettings, defaultSettings, type Settings } from './settings'

describe('coerceSettings', () => {
  it('fills a pre-#43 payload (no portsTooltipEnabled key) with the default ON', () => {
    const legacy = {
      notificationsEnabled: false,
      agentStatusEnabled: true,
      sessionRestoreEnabled: true,
    }

    const next = coerceSettings(legacy)

    expect(next.portsTooltipEnabled).toBe(true)
    expect(next.notificationsEnabled).toBe(false)
  })

  // Quickupdate 2026-09-12: analytics is ALWAYS on with no flag — the old
  // analyticsEnabled key is gone from the schema; a stale key in an old
  // settings.json is ignored, not coerced into a kill switch.
  it('ignores a stale analyticsEnabled key (always-on analytics)', () => {
    expect(coerceSettings({ analyticsEnabled: false } as unknown as Partial<Settings>)).not.toHaveProperty('analyticsEnabled')
    expect('analyticsEnabled' in coerceSettings({ analyticsEnabled: true })).toBe(false)
  })

  it('keeps an explicit portsTooltipEnabled=false (#43 toggle off survives load)', () => {
    const raw: Partial<Settings> = { portsTooltipEnabled: false }

    expect(coerceSettings(raw).portsTooltipEnabled).toBe(false)
  })

  it('falls back to every default for an empty payload', () => {
    expect(coerceSettings({})).toEqual(defaultSettings)
  })

  it('never returns undefined toggles for null/undefined input', () => {
    const next = coerceSettings(null)

    expect(Object.values(next)).toEqual(expect.arrayContaining([expect.any(Boolean)]))
    expect(Object.values(next).length).toBe(Object.keys(defaultSettings).length)
  })

  // #60: `umux config set default-launch-mode tui` writes defaultLaunchMode
  // straight into settings.json. coerceSettings is the app's load path — if
  // it dropped the key, the next app save would silently erase the CLI's
  // value, so it must carry through and default to "gui" (the Rust
  // Settings::default) when absent.
  it('carries a CLI-written defaultLaunchMode through and defaults to gui (#60)', () => {
    expect(coerceSettings({ defaultLaunchMode: 'tui' }).defaultLaunchMode).toBe('tui')
    expect(coerceSettings({}).defaultLaunchMode).toBe('gui')
    expect(defaultSettings.defaultLaunchMode).toBe('gui')
  })

  // #80 / #81: the two v1.6.0 sidebar-display switches. Branch labels are
  // ON by default (the fresh-install look keeps them); folder lines default
  // OFF (no change to the pre-v1.6.0 workspace rows). Missing keys in an old
  // settings.json coerce to those defaults.
  it('defaults showTabBranch ON and showTabFolders OFF (#80, #81)', () => {
    expect(defaultSettings.showTabBranch).toBe(true)
    expect(defaultSettings.showTabFolders).toBe(false)
    expect(coerceSettings({}).showTabBranch).toBe(true)
    expect(coerceSettings({}).showTabFolders).toBe(false)
  })

  it('keeps explicit showTabBranch / showTabFolders values through coerce (#80, #81)', () => {
    const next = coerceSettings({ showTabBranch: true, showTabFolders: true })
    expect(next.showTabBranch).toBe(true)
    expect(next.showTabFolders).toBe(true)
  })

  // #77 (v1.6.0): defaultShell — null is "Auto" (today's backend fallback
  // chain), a non-empty string is a concrete shell/custom command that must
  // survive save/reload verbatim (it later rides pty_open untouched).
  it('defaults defaultShell to null (Auto) for empty and legacy payloads (#77)', () => {
    expect(defaultSettings.defaultShell).toBe(null)
    expect(coerceSettings({}).defaultShell).toBe(null)
    expect(coerceSettings(null)?.defaultShell).toBe(null)
  })

  it('keeps an explicit defaultShell=null and a saved custom command through coerce (#77)', () => {
    expect(coerceSettings({ defaultShell: null }).defaultShell).toBe(null)
    expect(coerceSettings({ defaultShell: '/usr/bin/fish' }).defaultShell).toBe('/usr/bin/fish')
    expect(coerceSettings({ defaultShell: 'wsl.exe ~' }).defaultShell).toBe('wsl.exe ~')
  })

  // A hand-edited settings.json must never put a useless shell value on the
  // spawn path: junk types and blank strings coerce back to Auto.
  it('coerces a non-string or blank defaultShell back to Auto (#77)', () => {
    expect(coerceSettings({ defaultShell: 42 as unknown as string }).defaultShell).toBe(null)
    expect(coerceSettings({ defaultShell: '   ' }).defaultShell).toBe(null)
  })

  // Quickupdate 2026-09-12: the dragged sidebar width persists across
  // restarts. null = the CSS default; only a positive finite number is a
  // width — junk and legacy files without the key coerce back to null.
  it('defaults sidebarWidth to null for empty and legacy payloads', () => {
    expect(defaultSettings.sidebarWidth).toBe(null)
    expect(coerceSettings({}).sidebarWidth).toBe(null)
    expect(coerceSettings(null)?.sidebarWidth).toBe(null)
  })

  it('keeps a saved sidebarWidth and coerces junk back to null', () => {
    expect(coerceSettings({ sidebarWidth: 320 }).sidebarWidth).toBe(320)
    expect(coerceSettings({ sidebarWidth: null }).sidebarWidth).toBe(null)
    expect(coerceSettings({ sidebarWidth: 0 }).sidebarWidth).toBe(null)
    expect(coerceSettings({ sidebarWidth: -5 }).sidebarWidth).toBe(null)
    expect(
      coerceSettings({ sidebarWidth: 'wide' as unknown as number }).sidebarWidth,
    ).toBe(null)
  })

  it('rounds a fractional sidebarWidth to whole px (Rust u32 rejects floats)', () => {
    // The backend stores sidebar_width as u32 — serde rejects 600.5 and one
    // bad field fails the WHOLE save_settings write. A fractional value may
    // only ever come from a drag, so load-side coercion rounds it away.
    expect(coerceSettings({ sidebarWidth: 600.5 }).sidebarWidth).toBe(601)
    expect(coerceSettings({ sidebarWidth: 337.2 }).sidebarWidth).toBe(337)
  })

  // #89 (v1.7.0 phase 7): the autostart toggle rides the Storestation
  // block. Default OFF everywhere (opt-in like the daemon toggle), a pre-#89
  // file (the block without the flag, or no block at all) coerces to OFF,
  // and an explicit value survives — the setting only REMEMBERS the choice;
  // the mechanism itself lives in the OS.
  it('defaults the Storestation block to both toggles OFF, autostart included (#89)', () => {
    expect(defaultSettings.storestation).toEqual({ daemonEnabled: false, autostartEnabled: false })
    expect(coerceSettings({}).storestation).toEqual({
      daemonEnabled: false,
      autostartEnabled: false,
    })
  })

  it('coerces a pre-#89 Storestation block (no autostartEnabled) to autostart OFF (#89)', () => {
    expect(coerceSettings({ storestation: { daemonEnabled: true } }).storestation).toEqual({
      daemonEnabled: true,
      autostartEnabled: false,
    })
  })

  it('keeps an explicit autostartEnabled value through coerce (#89)', () => {
    expect(
      coerceSettings({ storestation: { daemonEnabled: false, autostartEnabled: true } })
        .storestation,
    ).toEqual({ daemonEnabled: false, autostartEnabled: true })
  })
})

// --- CLI-facing settings surface (#127) --------------------------------------
//
// The parity claim: `umux settings set <key> <value>` (live) lands the SAME
// patch applySettings receives from the matching UI toggle, because both go
// through applySettingByKey. These tests pin every app-domain key's patch —
// the Rust CLI side carries mirror tests over the same keys offline.

import { applySettingByKey, readAllSettings, readSettingByKey, SETTING_KEYS } from './settings'

describe('applySettingByKey: the CLI→UI shared path (#127)', () => {
  it('the four booleans map onto their camelCase fields', () => {
    expect(applySettingByKey('notifications-enabled', 'false')).toEqual({
      ok: true,
      patch: { notificationsEnabled: false },
      value: 'false',
    })
    expect(applySettingByKey('agent-status-enabled', 'true')).toEqual({
      ok: true,
      patch: { agentStatusEnabled: true },
      value: 'true',
    })
    expect(applySettingByKey('session-restore-enabled', 'false')).toEqual({
      ok: true,
      patch: { sessionRestoreEnabled: false },
      value: 'false',
    })
    expect(applySettingByKey('ports-tooltip-enabled', 'true')).toEqual({
      ok: true,
      patch: { portsTooltipEnabled: true },
      value: 'true',
    })
  })

  it('booleans are strict — anything but true/false is badParams', () => {
    for (const bad of ['yes', '1', 'TRUE', 'on', '']) {
      const out = applySettingByKey('notifications-enabled', bad)
      expect(out.ok).toBe(false)
      if (!out.ok) {
        expect(out.error.code).toBe('badParams')
        expect(out.error.message).toContain('true or false')
      }
    }
  })

  it('default-launch-mode accepts gui|tui and refuses the rest', () => {
    expect(applySettingByKey('default-launch-mode', 'tui')).toEqual({
      ok: true,
      patch: { defaultLaunchMode: 'tui' },
      value: 'tui',
    })
    const out = applySettingByKey('default-launch-mode', 'headless')
    expect(out.ok).toBe(false)
    if (!out.ok) expect(out.error.message).toContain('gui or tui')
  })

  it('show-tab-branch / show-tab-folders patch the display toggles', () => {
    expect(applySettingByKey('show-tab-branch', 'false')).toEqual({
      ok: true,
      patch: { showTabBranch: false },
      value: 'false',
    })
    expect(applySettingByKey('show-tab-folders', 'true')).toEqual({
      ok: true,
      patch: { showTabFolders: true },
      value: 'true',
    })
  })

  it('default-shell keeps a path verbatim and "auto" clears to null', () => {
    expect(applySettingByKey('default-shell', '/usr/bin/fish')).toEqual({
      ok: true,
      patch: { defaultShell: '/usr/bin/fish' },
      value: '/usr/bin/fish',
    })
    expect(applySettingByKey('default-shell', 'AUTO')).toEqual({
      ok: true,
      patch: { defaultShell: null },
      value: 'auto',
    })
    const out = applySettingByKey('default-shell', '   ')
    expect(out.ok).toBe(false)
    if (!out.ok) expect(out.error.code).toBe('badParams')
  })

  it('sidebar-width takes whole pixels only', () => {
    expect(applySettingByKey('sidebar-width', '280')).toEqual({
      ok: true,
      patch: { sidebarWidth: 280 },
      value: '280',
    })
    for (const bad of ['0', '-3', '12.5', 'wide']) {
      const out = applySettingByKey('sidebar-width', bad)
      expect(out.ok).toBe(false)
      if (!out.ok) expect(out.error.code).toBe('badParams')
    }
    expect(applySettingByKey('sidebar-width', 'auto')).toEqual({
      ok: true,
      patch: { sidebarWidth: null },
      value: 'auto',
    })
  })

  it('an unknown key is settingNotFound with the enumerated key list', () => {
    const out = applySettingByKey('storestation-enabled', 'true')
    expect(out.ok).toBe(false)
    if (!out.ok) {
      expect(out.error.code).toBe('settingNotFound')
      expect(out.error.next[0]).toContain('notifications-enabled')
    }
  })
})

describe('readSettingByKey: `settings get` agreement', () => {
  it('every key reads back what set wrote (one round trip per key)', () => {
    const s = { ...defaultSettings }
    for (const key of SETTING_KEYS) {
      const outcome = applySettingByKey(key, readSettingByKey(s, key) as string)
      expect(outcome.ok).toBe(true)
    }
    // Spot checks on the non-trivial coercions.
    expect(readSettingByKey(s, 'default-shell')).toBe('auto')
    expect(readSettingByKey(s, 'sidebar-width')).toBe('auto')
    expect(readSettingByKey({ ...s, sidebarWidth: 312 }, 'sidebar-width')).toBe('312')
    expect(readSettingByKey({ ...s, defaultShell: '/bin/zsh' }, 'default-shell')).toBe('/bin/zsh')
  })

  it('readAllSettings covers exactly the documented keys', () => {
    const all = readAllSettings(defaultSettings)
    expect(Object.keys(all).sort()).toEqual([...SETTING_KEYS].sort())
    expect(all['show-tab-folders']).toBe('false')
    expect(all['notifications-enabled']).toBe('true')
  })

  it('an unknown key reads as null (the caller answers settingNotFound)', () => {
    expect(readSettingByKey(defaultSettings, 'analytics-enabled')).toBeNull()
  })
})
