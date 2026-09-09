// shellDetector — the pure core of the shell picker (v1.6.0 / issue #77).
//
// Deep-module split: the Rust `list_shells` command only PROBES the OS (PATH
// scan; /etc/shells + the login shell on Unix; registry App Paths on Windows)
// and returns raw { path, source } results — no ranking, no naming. This
// module receives those results and makes every decision: dedup, ranking,
// display names, and the Settings picker's option list. No I/O, no clock —
// trivially unit-testable with synthetic fixtures (shellDetector.test.ts).
//
// Nothing is assumed to exist: "nothing found" yields an empty list, and a
// setup the probes miss is covered by the Settings custom entry (a plain
// command string persisted in settings.defaultShell).

export type ShellProbeSource = 'path' | 'etcShells' | 'loginShell' | 'registry' | 'wslRegistry'

/// One raw probe result as returned by the `list_shells` invoke command.
/// `path` is whatever the probe found, verbatim — an absolute path in the
/// usual case, the $SHELL value for the login-shell probe.
export type ShellProbe = {
  path: string
  source: ShellProbeSource
}

/// One picker-ready shell: a human name plus the exact command pty_open
/// receives. launchCommand is the probe path, never rewritten.
export type ShellEntry = {
  displayName: string
  launchCommand: string
}

/// One row of the Settings "Default shell" picker. `value: null` is "Auto"
/// (today's backend fallback chain); any string value is the launch command
/// (or custom command) passed to pty_open verbatim.
export type ShellPickerOption = {
  value: string | null
  label: string
}

export type DetectorPlatform = 'windows' | 'unix'

/// Known-shell preference order per platform: the picker lists these first,
/// in this order; anything else follows, stable by first-seen probe order.
/// The Windows list ends with the WSL distro launchers (ubuntu.exe,
/// ubuntu2204.exe, … from WindowsApps) so a named distro outranks truly
/// unknown binaries.
const PREFERENCE: Record<DetectorPlatform, string[]> = {
  windows: [
    'pwsh', 'powershell', 'cmd', 'wsl', 'bash', 'nu',
    'ubuntu', 'debian', 'kali', 'alpine', 'fedora', 'opensuse', 'arch',
  ],
  unix: ['bash', 'zsh', 'fish', 'pwsh', 'ksh', 'tcsh', 'dash', 'sh', 'csh', 'nu'],
}

/// Friendly display names for the known basenames (lowercased, extension
/// stripped). Anything else shows its own basename.
const DISPLAY_NAMES: Record<string, string> = {
  pwsh: 'PowerShell',
  powershell: 'Windows PowerShell',
  cmd: 'Command Prompt',
  wsl: 'WSL',
  bash: 'Bash',
  zsh: 'Zsh',
  fish: 'Fish',
  ksh: 'Ksh',
  tcsh: 'Tcsh',
  csh: 'Csh',
  dash: 'Dash',
  sh: 'sh',
  nu: 'Nushell',
  debian: 'Debian',
  kali: 'Kali',
  alpine: 'Alpine',
  fedora: 'Fedora',
  opensuse: 'openSUSE',
  arch: 'Arch',
}

/// WSL distro launchers carry the release in the basename (ubuntu2204.exe =
/// Ubuntu 22.04). Returns the display name, or undefined when `base` is not
/// a versioned ubuntu launcher.
function ubuntuDistroName(base: string): string | undefined {
  const m = /^ubuntu(\d{1,2})(\d{2})$/.exec(base)
  if (m == null) return base === 'ubuntu' ? 'Ubuntu' : undefined
  return `Ubuntu ${m[1]}.${m[2]}`
}

/// The executable's basename: extension stripped, lowercased for matching
/// (Windows paths are case-insensitive; lowering a Unix basename only ever
/// affects shells that are already lowercase).
function basenameOf(path: string): string {
  const last = path.split(/[\\/]/).pop() ?? path
  return last.replace(/\.(exe|com|bat|cmd)$/i, '').toLowerCase()
}

/// The first token of a command string, quote-aware (the Rust PTY side's
/// split_shell_command semantics): double quotes group characters so a
/// quoted Windows path with spaces stays one token. Needed to RANK command
/// entries like `"C:\Program Files\WSL\wsl.exe" -d Ubuntu` as wsl.
function firstTokenOf(command: string): string {
  let cur = ''
  let inQuotes = false
  for (const ch of command) {
    if (ch === '"') {
      inQuotes = !inQuotes
    } else if (!inQuotes && (ch === ' ' || ch === '\t')) {
      if (cur !== '') break
    } else {
      cur += ch
    }
  }
  return cur
}

/// The distro name of a `wsl.exe -d <name>` command, or undefined. This
/// pattern — not merely "contains a space" — is what marks a probe as a
/// command line: plain Windows paths legitimately contain spaces.
function wslDistroName(command: string): string | undefined {
  return /(?:^|\s)-d\s+([^\s"]+)\s*$/.exec(command)?.[1]
}

/// Turn one raw probe into a picker entry. The launch command is the probe
/// path verbatim — it rides pty_open untouched (issue #77 AC).
function toEntry(probe: ShellProbe): ShellEntry {
  const distro = wslDistroName(probe.path)
  if (distro != null) return { displayName: distro, launchCommand: probe.path }
  const base = basenameOf(probe.path)
  const raw = probe.path.split(/[\\/]/).pop()?.replace(/\.(exe|com|bat|cmd)$/i, '') ?? probe.path
  const displayName = DISPLAY_NAMES[base] ?? ubuntuDistroName(base) ?? raw
  return { displayName, launchCommand: probe.path }
}

/// Rank + dedup the raw probe results into the installed-shell list.
/// Known shells come first in the platform's preference order, unknown ones
/// after (first-seen stable). Duplicates collapse by shell IDENTITY: the
/// basename for plain program paths (one shell installed in two places
/// appears once, keeping the first/highest-ranked path), the WHOLE command
/// for `wsl.exe -d <distro>` entries — each distro is its own shell.
/// Genuinely different shells stay distinct even when their names rhyme
/// (PowerShell vs Windows PowerShell).
export function detectShells(platform: DetectorPlatform, probes: ShellProbe[]): ShellEntry[] {
  const order = PREFERENCE[platform]
  const identityOf = (probe: ShellProbe): string =>
    wslDistroName(probe.path) != null ? probe.path.toLowerCase() : basenameOf(probe.path)
  const rankOf = (probe: ShellProbe): number => {
    // Distro commands rank as wsl (the first token names the real program);
    // plain paths rank by their own basename.
    const base =
      wslDistroName(probe.path) != null
        ? basenameOf(firstTokenOf(probe.path))
        : basenameOf(probe.path)
    const idx = order.indexOf(base)
    return idx === -1 ? order.length : idx
  }

  const ranked: Array<{ probe: ShellProbe; rank: number; seq: number }> = probes.map(
    (probe, seq) => ({ probe, rank: rankOf(probe), seq }),
  )
  ranked.sort((a, b) => a.rank - b.rank || a.seq - b.seq)

  const seen = new Set<string>()
  const out: ShellEntry[] = []
  for (const { probe } of ranked) {
    const identity = identityOf(probe)
    if (seen.has(identity)) continue
    seen.add(identity)
    out.push(toEntry(probe))
  }
  return out
}

/// The Settings "Default shell" picker's full option list: "Auto" first
/// (null = the backend fallback chain, today's behavior), then one option per
/// detected shell, then a saved custom command when it is set and not already
/// covered by a detected entry. Values are what pty_open receives — verbatim.
export function pickerOptions(
  entries: ShellEntry[],
  savedDefault: string | null,
): ShellPickerOption[] {
  const options: ShellPickerOption[] = [{ value: null, label: 'Auto' }]
  for (const e of entries) {
    options.push({ value: e.launchCommand, label: e.displayName })
  }
  const custom = savedDefault?.trim()
  if (custom) {
    const covered = entries.some(
      (e) => e.launchCommand === custom || e.displayName === custom,
    )
    if (!covered) options.push({ value: custom, label: custom })
  }
  return options
}
