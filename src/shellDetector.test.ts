// shellDetector — the pure core of the v1.6.0 shell picker (issue #77,
// stories #88–#90; plan `plans/umux-v1.6.0-shell-picker-plan.md`, Phase 1).
//
// The Rust `list_shells` command only PROBES (PATH scan, /etc/shells, login
// shell, Windows registry) and returns raw results; every decision lives in
// this pure module: dedup, ranking, display names, and the picker's option
// list. Fully unit-testable with synthetic fixtures — no shell is ever
// assumed to exist, and "nothing found" yields an empty list (the custom
// entry is the only fallback).
//
// Assumptions encoded:
//  - Input shape: `detectShells(platform, probes)` where platform is
//    'windows' | 'unix' and probes are the raw `list_shells` payload —
//    { path, source } with source 'path' | 'etcShells' | 'loginShell' |
//    'registry', in the backend's probe order (PATH scan first, then the
//    platform extras). `path` is whatever the probe found verbatim — usually
//    an absolute path, possibly a bare command for the login shell.
//  - Output shape: ranked, deduped { displayName, launchCommand } entries.
//    launchCommand is the probe path verbatim (it is what rides pty_open).
//  - Ranking: known shells first, by a per-platform preference list
//    (windows: pwsh, powershell, cmd, wsl, bash, nu + the WSL distro
//    launchers; unix: bash, zsh, fish, pwsh, ksh, tcsh, dash, sh, csh, nu);
//    unknown shells follow, stable by first-seen order. Display name derives
//    from the basename (case-insensitive on Windows); an unknown basename is
//    shown as-is (.exe stripped).
//  - Dedup (fix round 2026-09-09): by shell IDENTITY — the basename — not by
//    full path. One shell installed in two places (WSL's System32 shim AND
//    the Program Files store build, /bin/bash + /usr/bin/bash) must appear
//    ONCE, keeping the first (highest-ranked) path. Genuinely different
//    shells stay distinct even when their names rhyme (pwsh = "PowerShell"
//    vs powershell = "Windows PowerShell").
//  - pickerOptions(entries, savedDefault): the Settings picker's option list
//    — "Auto" (value null) first, then one option per entry (value =
//    launchCommand verbatim), then the saved custom command when set and not
//    already covered by a detected entry.
//  - #78 (Phase 2): newTabArrowVisible(entries) is the arrow-visibility rule
//    — true iff the detector found MORE THAN ONE shell (a saved custom
//    command never affects visibility; it is not a detection result).
//    tabShellOptions(entries, savedDefault) is the "+ New tab" dropdown's
//    list — one option per detected shell plus the saved custom command,
//    with NO "Auto" row (a dropdown pick always names an exact command; the
//    plain "+" is what keeps the Settings default).
//  - NOT tested here: the Rust probes themselves (cargo tests), the
//    pty_open handoff (TerminalSurface tests), the UI (SettingsDialog tests).

import { describe, it, expect } from 'vitest'
import {
  detectShells,
  newTabArrowVisible,
  pickerOptions,
  tabShellOptions,
} from './shellDetector'

describe('detectShells', () => {
  // Tracer bullet: a typical Windows probe — PowerShell 7 and cmd found on
  // PATH, Windows PowerShell via the registry, and pwsh seen twice (PATH +
  // registry). One ranked, deduped list with display names must come out.
  it('ranks and dedups a windows probe fixture into named entries', () => {
    const found = detectShells('windows', [
      { path: 'C:\\Windows\\System32\\cmd.exe', source: 'path' },
      { path: 'C:\\Program Files\\PowerShell\\7\\pwsh.exe', source: 'path' },
      { path: 'C:\\Windows\\System32\\wsl.exe', source: 'path' },
      { path: 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe', source: 'registry' },
      { path: 'C:\\Program Files\\PowerShell\\7\\pwsh.exe', source: 'registry' },
    ])

    expect(found).toEqual([
      { displayName: 'PowerShell', launchCommand: 'C:\\Program Files\\PowerShell\\7\\pwsh.exe' },
      {
        displayName: 'Windows PowerShell',
        launchCommand: 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe',
      },
      { displayName: 'Command Prompt', launchCommand: 'C:\\Windows\\System32\\cmd.exe' },
      { displayName: 'WSL', launchCommand: 'C:\\Windows\\System32\\wsl.exe' },
    ])
  })

  // "Nothing found" must stay empty — the custom entry is the only fallback,
  // and no shell name may be invented for a machine that has none.
  it('yields an empty list for an empty probe fixture', () => {
    expect(detectShells('windows', [])).toEqual([])
    expect(detectShells('unix', [])).toEqual([])
  })

  // Unix: PATH scan finds /usr/bin binaries, /etc/shells lists canonical
  // paths, the login shell overlaps both. Same-named shells collapse into
  // ONE entry (first path wins); fish ranks after zsh regardless of order.
  it('ranks and dedups a unix probe fixture (PATH + /etc/shells + login shell)', () => {
    const found = detectShells('unix', [
      { path: '/usr/bin/fish', source: 'path' },
      { path: '/usr/bin/bash', source: 'path' },
      { path: '/bin/bash', source: 'etcShells' },
      { path: '/bin/bash', source: 'loginShell' },
      { path: '/usr/bin/zsh', source: 'etcShells' },
    ])

    expect(found).toEqual([
      { displayName: 'Bash', launchCommand: '/usr/bin/bash' },
      { displayName: 'Zsh', launchCommand: '/usr/bin/zsh' },
      { displayName: 'Fish', launchCommand: '/usr/bin/fish' },
    ])
  })

  // A shell the preference lists do not know is still offered — labeled by
  // its basename, ranked after every known shell, stable by first-seen.
  it('lists unknown shells after known ones, labeled by basename', () => {
    const found = detectShells('unix', [
      { path: '/usr/local/bin/elvish', source: 'path' },
      { path: '/usr/bin/bash', source: 'path' },
      { path: '/opt/superfish', source: 'path' },
    ])

    expect(found).toEqual([
      { displayName: 'Bash', launchCommand: '/usr/bin/bash' },
      { displayName: 'elvish', launchCommand: '/usr/local/bin/elvish' },
      { displayName: 'superfish', launchCommand: '/opt/superfish' },
    ])
  })

  // Fix round 2026-09-09: the same shell in two installs (WSL store build in
  // Program Files + the System32 shim) collapsed to ONE entry; WSL distro
  // launchers from WindowsApps (ubuntu.exe, ubuntu2204.exe, …) surface as
  // their own named entries.
  it('collapses same-named installs and names WSL distro launchers (windows)', () => {
    const found = detectShells('windows', [
      { path: 'C:\\Program Files\\WSL\\wsl.exe', source: 'path' },
      { path: 'C:\\Windows\\System32\\wsl.exe', source: 'path' },
      { path: 'C:\\Users\\adam\\AppData\\Local\\Microsoft\\WindowsApps\\ubuntu.exe', source: 'path' },
      {
        path: 'C:\\Users\\adam\\AppData\\Local\\Microsoft\\WindowsApps\\ubuntu2404.exe',
        source: 'path',
      },
    ])

    expect(found).toEqual([
      {
        displayName: 'WSL',
        launchCommand: 'C:\\Program Files\\WSL\\wsl.exe',
      },
      {
        displayName: 'Ubuntu',
        launchCommand: 'C:\\Users\\adam\\AppData\\Local\\Microsoft\\WindowsApps\\ubuntu.exe',
      },
      {
        displayName: 'Ubuntu 24.04',
        launchCommand: 'C:\\Users\\adam\\AppData\\Local\\Microsoft\\WindowsApps\\ubuntu2404.exe',
      },
    ])
  })

  // Fix round 2 (2026-09-09): modern Store WSL registers distros WITHOUT any
  // per-distro launcher exe, so the backend surfaces each registered distro
  // as a `wsl.exe -d <name>` command (source wslRegistry). Those commands are
  // distinct entries (identity = the whole command, they carry arguments),
  // named by their distro, ranked with the plain wsl.exe entry.
  it('keeps wsl -d distro commands distinct and names them by distro (windows)', () => {
    const found = detectShells('windows', [
      { path: 'C:\\Windows\\System32\\wsl.exe', source: 'path' },
      { path: '"C:\\Program Files\\WSL\\wsl.exe" -d docker-desktop', source: 'wslRegistry' },
      { path: '"C:\\Program Files\\WSL\\wsl.exe" -d Ubuntu', source: 'wslRegistry' },
    ])

    expect(found).toEqual([
      { displayName: 'WSL', launchCommand: 'C:\\Windows\\System32\\wsl.exe' },
      {
        displayName: 'docker-desktop',
        launchCommand: '"C:\\Program Files\\WSL\\wsl.exe" -d docker-desktop',
      },
      { displayName: 'Ubuntu', launchCommand: '"C:\\Program Files\\WSL\\wsl.exe" -d Ubuntu' },
    ])
  })
})

describe('pickerOptions', () => {
  const entries = detectShells('unix', [
    { path: '/usr/bin/fish', source: 'path' },
    { path: '/bin/bash', source: 'loginShell' },
  ])

  // "Auto" (null = today's backend fallback chain) is always the first
  // option; every detected shell follows with its command as the value.
  it('lists Auto first, then one option per detected shell', () => {
    expect(pickerOptions(entries, null)).toEqual([
      { value: null, label: 'Auto' },
      { value: '/bin/bash', label: 'Bash' },
      { value: '/usr/bin/fish', label: 'Fish' },
    ])
  })

  // A saved custom command (from the text field) must appear in the list
  // verbatim so it round-trips and later rides pty_open untouched.
  it('appends a saved custom command verbatim', () => {
    expect(pickerOptions(entries, 'C:\\tools\\mysh.exe --login')).toContainEqual({
      value: 'C:\\tools\\mysh.exe --login',
      label: 'C:\\tools\\mysh.exe --login',
    })
  })

  // A saved default that IS a detected shell must not produce a second,
  // duplicate option next to the detector's own entry.
  it('does not duplicate a saved default already covered by a detected entry', () => {
    const options = pickerOptions(entries, '/usr/bin/fish')
    expect(options.filter((o) => o.value === '/usr/bin/fish')).toHaveLength(1)
  })
})

// --- #78 (v1.6.0 Phase 2): the "+ New tab" arrow -----------------------------

describe('newTabArrowVisible', () => {
  // AC: two or more detected shells → the arrow renders next to "+ New tab".
  it('shows the arrow when the detector finds two or more shells', () => {
    const two = detectShells('unix', [
      { path: '/bin/bash', source: 'loginShell' },
      { path: '/usr/bin/fish', source: 'path' },
    ])
    expect(newTabArrowVisible(two)).toBe(true)

    const three = detectShells('unix', [
      { path: '/bin/bash', source: 'loginShell' },
      { path: '/usr/bin/zsh', source: 'path' },
      { path: '/usr/bin/fish', source: 'path' },
    ])
    expect(newTabArrowVisible(three)).toBe(true)
  })

  // AC: exactly one detected shell → arrow not rendered; the bar is
  // byte-identical to the pre-#78 UI. Duplicate probes of the same shell
  // collapse to ONE entry, so a twice-seen bash still hides the arrow.
  it('hides the arrow for exactly one detected shell', () => {
    const one = detectShells('unix', [
      { path: '/bin/bash', source: 'loginShell' },
      { path: '/usr/bin/bash', source: 'path' },
    ])
    expect(one).toHaveLength(1)
    expect(newTabArrowVisible(one)).toBe(false)
  })

  // "Nothing found" (and a failed probe — the UI glue leaves the list empty)
  // must hide the arrow too: today's UI, no dropdown.
  it('hides the arrow when nothing was detected', () => {
    expect(newTabArrowVisible([])).toBe(false)
    expect(newTabArrowVisible(detectShells('windows', []))).toBe(false)
  })
})

describe('tabShellOptions', () => {
  const entries = detectShells('unix', [
    { path: '/usr/bin/fish', source: 'path' },
    { path: '/bin/bash', source: 'loginShell' },
  ])

  // Unlike the Settings picker there is NO "Auto" row: a dropdown pick must
  // name an exact command — the plain "+" is what keeps the Settings default.
  it('lists exactly one option per detected shell, no Auto row', () => {
    expect(tabShellOptions(entries, null)).toEqual([
      { value: '/bin/bash', label: 'Bash' },
      { value: '/usr/bin/fish', label: 'Fish' },
    ])
  })

  // The saved custom command rides the dropdown verbatim so it stays
  // reachable even though no probe found it.
  it('appends a saved custom command verbatim', () => {
    expect(tabShellOptions(entries, 'C:\\tools\\mysh.exe --login')).toEqual([
      { value: '/bin/bash', label: 'Bash' },
      { value: '/usr/bin/fish', label: 'Fish' },
      { value: 'C:\\tools\\mysh.exe --login', label: 'C:\\tools\\mysh.exe --login' },
    ])
  })

  // A saved default that IS a detected shell must not appear twice in the
  // dropdown either (same identity rule as the Settings picker).
  it('does not duplicate a saved default already covered by a detected entry', () => {
    const options = tabShellOptions(entries, '/usr/bin/fish')
    expect(options.filter((o) => o.value === '/usr/bin/fish')).toHaveLength(1)
    expect(options).toHaveLength(2)
  })
})
