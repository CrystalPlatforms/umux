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
      analyticsEnabled: true,
    }

    const next = coerceSettings(legacy)

    expect(next.portsTooltipEnabled).toBe(true)
    expect(next.notificationsEnabled).toBe(false)
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
})
