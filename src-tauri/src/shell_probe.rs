// shell_probe — raw installed-shell probes for the v1.6.0 shell picker
// (issue #77 / stories #88–#90; plan Phase 1).
//
// Deep-module split (mirrors ShellDetector): THIS module only PROBES the OS
// and returns raw { path, source } results — no ranking, no naming, no
// decisions. The pure TS ShellDetector (src/shellDetector.ts) receives the
// injected results and does dedup, ranking, and display names. Nothing here
// assumes any specific shell exists; an empty PATH with no other hits simply
// yields an empty list (the Settings custom entry is the only fallback).
//
// Probes per platform:
//   - PATH scan (all platforms): known shell executables found in the PATH
//     directories, as absolute paths.
//   - /etc/shells (unix only): each non-comment line, verbatim.
//   - login shell (unix only): $SHELL, falling back to the passwd database
//     (the same truth resolve_shell uses for the Auto path).
//   - registry (windows only): the App Paths entries for PowerShell — the
//     one shell family an MSI install may leave off PATH.
//   - WSL distro registry (windows only): every registered WSL distro gets a
//     `wsl.exe -d <name>` entry. Modern Store WSL registers distros without
//     per-distro launcher exes (no ubuntu.exe anywhere), so this is the only
//     place "Ubuntu" is visible — and `-d` beats bash.exe, which launches
//     whatever distro happens to be the WSL default (often docker-desktop).

use serde::Serialize;

/// One raw probe result. `path` is whatever the probe found, verbatim (the
/// TS side never rewrites it — it later rides `pty_open` as the command).
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ShellProbe {
    pub path: String,
    pub source: ShellProbeSource,
}

/// Where a probe result came from. The TS detector only uses it as metadata
/// today; carrying it keeps the raw-results contract honest and lets the
/// detector prefer the login shell within a tie if it ever needs to.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ShellProbeSource {
    Path,
    EtcShells,
    LoginShell,
    Registry,
    WslRegistry,
}

/// Shell basenames the PATH scan looks for (extension added per platform).
/// The ubuntuNNNN entries are the WSL distro launchers from WindowsApps
/// (ubuntu2204.exe = Ubuntu 22.04); the TS ShellDetector names them. A shell
/// missing from this list is NOT lost: /etc/shells and the login shell
/// probes still surface it on Unix, and the Settings custom entry covers
/// everything else on every platform.
const KNOWN_SHELL_STEMS: &[&str] = &[
    "bash", "zsh", "fish", "pwsh", "powershell", "cmd", "wsl", "nu", "ksh", "tcsh", "csh",
    "dash", "sh", "elvish", "xonsh",
    // WSL distro launchers (fix round 2026-09-09: "Ubuntu" was invisible).
    "ubuntu", "ubuntu2004", "ubuntu2204", "ubuntu2404",
    "debian", "kali", "alpine", "fedora", "opensuse", "arch",
];

/// Parse /etc/shells text into its path lines: trimmed, blanks and `#`
/// comments dropped, order preserved. Pure — cargo-testable.
pub fn parse_etc_shells(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

/// Extract the distro names from `reg query <Lxss> /s /v DistributionName`
/// output — the REG_SZ value on each DistributionName line, in output order.
/// Pure — cargo-testable.
pub fn parse_lxss_distros(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.lines() {
        if !line.contains("DistributionName") {
            continue;
        }
        if let Some(idx) = line.find("REG_SZ") {
            let value = line[idx + "REG_SZ".len()..].trim();
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
    }
    out
}

/// Does `path` exist as something spawnable? On Windows the Store launchers
/// (pwsh.exe / bash.exe / wsl.exe under ...\WindowsApps) are app-execution
/// ALIASES — reparse points `fs::metadata` refuses to follow, so `is_file()`
/// says "no" and whole shells vanish from the picker. `symlink_metadata`
/// reads the reparse point itself and succeeds. On Unix a plain `is_file()`
/// is the whole truth.
#[cfg(windows)]
fn exists_as_executable(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(not(windows))]
fn exists_as_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Extract the default value from `reg query <key> /ve` output — the text
/// after REG_SZ on the value line — or None when the key is missing (reg
/// prints an ERROR line and exits non-zero) or no REG_SZ value exists.
/// Pure — cargo-testable.
pub fn parse_reg_default(output: &str) -> Option<String> {
    for line in output.lines() {
        if let Some(idx) = line.find("REG_SZ") {
            let value = line[idx + "REG_SZ".len()..].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Executable filename suffix on this platform (".exe" on Windows, nothing
/// on Unix) so one KNOWN_SHELL_STEMS list serves both probes.
#[cfg(windows)]
const EXEC_SUFFIX: &str = ".exe";
#[cfg(not(windows))]
const EXEC_SUFFIX: &str = "";

/// Every known shell executable found in the PATH directories, as absolute
/// paths. Outer loop over PATH dirs keeps results grouped per directory;
/// the TS detector does all ordering decisions anyway.
fn probe_path() -> Vec<ShellProbe> {
    let dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut out = Vec::new();
    for dir in dirs {
        for stem in KNOWN_SHELL_STEMS {
            let candidate = dir.join(format!("{stem}{EXEC_SUFFIX}"));
            if exists_as_executable(&candidate) {
                out.push(ShellProbe {
                    path: candidate.to_string_lossy().into_owned(),
                    source: ShellProbeSource::Path,
                });
            }
        }
    }
    out
}

/// The platform-specific extras: /etc/shells + the login shell on Unix,
/// the registry App Paths probes on Windows (see the module comment).
#[cfg(unix)]
fn platform_extras() -> Vec<ShellProbe> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/etc/shells") {
        for path in parse_etc_shells(&text) {
            out.push(ShellProbe { path, source: ShellProbeSource::EtcShells });
        }
    }
    // $SHELL first (a dev run from a terminal), then the passwd database —
    // the same truth the Auto fallback chain uses for the login shell.
    let login = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(crate::passwd_shell);
    if let Some(path) = login {
        out.push(ShellProbe { path, source: ShellProbeSource::LoginShell });
    }
    out
}

#[cfg(not(unix))]
fn platform_extras() -> Vec<ShellProbe> {
    let mut out = Vec::new();
    // App Paths entries for the PowerShell family: the one common case an
    // MSI install leaves off PATH. HKCU first (per-user install), then HKLM.
    for (root, exe) in [
        ("HKCU", "pwsh.exe"),
        ("HKLM", "pwsh.exe"),
        ("HKCU", "powershell.exe"),
        ("HKLM", "powershell.exe"),
    ] {
        let key = format!(
            "{root}\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe}"
        );
        let Ok(output) = std::process::Command::new("reg").arg("query").arg(&key).arg("/ve").output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(path) = parse_reg_default(&text) {
            out.push(ShellProbe { path, source: ShellProbeSource::Registry });
        }
    }

    // Registered WSL distros (#77 fix round 2026-09-09): modern Store WSL
    // registers distros WITHOUT per-distro launcher exes (there is no
    // ubuntu.exe anywhere on such machines), so the PATH scan can never see
    // them. The Lxss registry lists every distro; each gets a launch command
    // through wsl.exe -d <name>. Distros register under HKCU normally, but a
    // machine-wide WSL install puts them under HKLM (seen in the wild), so
    // both hives are queried and merged, first wins. wsl.exe's path is
    // quoted when it contains spaces so the PTY's quote-aware split keeps it
    // as one token.
    let wsl = out
        .iter()
        .map(|p| &p.path)
        .find(|p| {
            p.rsplit(['\\', '/'])
                .next()
                .is_some_and(|b| b.eq_ignore_ascii_case("wsl.exe"))
        })
        .cloned()
        .unwrap_or_else(|| "wsl.exe".to_string());
    let mut seen_distros: std::collections::HashSet<String> = std::collections::HashSet::new();
    for root in ["HKCU", "HKLM"] {
        let Ok(output) = std::process::Command::new("reg")
            .arg("query")
            .arg(format!("{root}\\SOFTWARE\\Microsoft\\Windows\\NT\\CurrentVersion\\Lxss"))
            .arg("/s")
            .arg("/v")
            .arg("DistributionName")
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        for name in parse_lxss_distros(&text) {
            if !seen_distros.insert(name.clone()) {
                continue;
            }
            let command = if wsl.contains(' ') {
                format!("\"{wsl}\" -d {name}")
            } else {
                format!("{wsl} -d {name}")
            };
            out.push(ShellProbe { path: command, source: ShellProbeSource::WslRegistry });
        }
    }
    out
}

/// Run every probe for the current platform and return the raw results in
/// probe order (PATH scan first, then the platform extras). Raw only — the
/// TS ShellDetector owns ranking, dedup, and display names (#77).
pub fn probe_shells() -> Vec<ShellProbe> {
    let mut out = probe_path();
    out.extend(platform_extras());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-P1 (#77 — /etc/shells parse): lines survive verbatim and in order;
    // blanks and # comments are dropped.
    #[test]
    fn parse_etc_shells_keeps_paths_skips_comments_and_blanks() {
        let text = "# /etc/shells: valid login shells\n/bin/sh\n\n/bin/bash\n  /usr/bin/fish  \n#/bin/commented\n";
        assert_eq!(
            parse_etc_shells(text),
            vec!["/bin/sh".to_string(), "/bin/bash".to_string(), "/usr/bin/fish".to_string()]
        );
    }

    // T-P2 (#77 — empty/absent /etc/shells content yields nothing):
    #[test]
    fn parse_etc_shells_empty_text_yields_nothing() {
        assert_eq!(parse_etc_shells(""), Vec::<String>::new());
        assert_eq!(parse_etc_shells("# only a comment\n"), Vec::<String>::new());
    }

    // T-P3 (#77 — reg query output parse): the default value is the text
    // after REG_SZ, trimmed.
    #[test]
    fn parse_reg_default_extracts_value_after_reg_sz() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\pwsh.exe\r\n    (Default)    REG_SZ    C:\\Program Files\\PowerShell\\7\\pwsh.exe\r\n\r\n";
        assert_eq!(
            parse_reg_default(out),
            Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_string())
        );
    }

    // T-P4 (#77 — a missing key reg query output): ERROR text, no REG_SZ —
    // the probe must yield None, never a bogus path.
    #[test]
    fn parse_reg_default_error_output_yields_none() {
        let out = "\r\nERROR: The system was unable to find the specified registry key or value.\r\n";
        assert_eq!(parse_reg_default(out), None);
        assert_eq!(parse_reg_default(""), None);
    }

    // T-P5 (#77 fix round — Lxss registry parse): each DistributionName REG_SZ
    // line yields one distro name, in output order; header/noise lines and a
    // missing key yield nothing. Modern Store WSL registers distros without
    // per-distro launcher exes, so this is the only way to see "Ubuntu".
    #[test]
    fn parse_lxss_distros_extracts_names_in_order() {
        let out = "\r\nHKEY_CURRENT_USER\\SOFTWARE\\Microsoft\\Windows\\NT\\CurrentVersion\\Lxss\r\n    DefaultUid    REG_DWORD    0x3e8\r\n\r\nHKEY_CURRENT_USER\\...\\Lxss\\{guid-1}\r\n    DistributionName    REG_SZ    docker-desktop\r\n\r\nHKEY_CURRENT_USER\\...\\Lxss\\{guid-2}\r\n    DistributionName    REG_SZ    Ubuntu\r\n";
        assert_eq!(
            parse_lxss_distros(out),
            vec!["docker-desktop".to_string(), "Ubuntu".to_string()]
        );
        assert_eq!(parse_lxss_distros(""), Vec::<String>::new());
        assert_eq!(
            parse_lxss_distros("\r\nERROR: The system was unable to find the specified registry key or value.\r\n"),
            Vec::<String>::new()
        );
    }

    // T-P6 (#77 fix round — existence predicate): a real file exists, a
    // missing path does not. On Windows this goes through symlink_metadata
    // because Store app-execution aliases (pwsh.exe/bash.exe/wsl.exe in
    // WindowsApps) are reparse points fs::metadata refuses to follow.
    #[test]
    fn exists_as_executable_true_for_real_file_false_for_missing() {
        let f = std::env::temp_dir().join(format!("umux-probe-{}.exe", std::process::id()));
        std::fs::write(&f, b"x").expect("write probe fixture");
        assert!(exists_as_executable(&f));
        assert!(!exists_as_executable(
            &std::env::temp_dir().join("umux-probe-definitely-missing-xyz.exe")
        ));
        let _ = std::fs::remove_file(&f);
    }
}
