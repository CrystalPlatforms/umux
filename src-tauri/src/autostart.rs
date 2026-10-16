// autostart — the login-time launcher for umux Storestation (#89, v1.7.0
// phase 7, PRD stories 108/109).
//
// The OS starts the daemon HEADLESS at login (`umux-storestation run`), per
// the /carve decision: Windows HKCU `Run` key · macOS LaunchAgent plist ·
// Linux systemd user unit. OFF by default everywhere; disabling removes the
// mechanism fully (no orphan keys/plists/units).
//
// Split (deep-module style): the ARTIFACT GENERATORS below are pure string
// builders — unit-testable on every OS (the AC's "unit tests generate a
// valid Run-key value / LaunchAgent plist / systemd unit per platform" runs
// everywhere). Only the INSTALL/REMOVE halves are platform-gated; they are
// thin file-writes plus one well-known command each, HITL-verified per OS
// (macOS + Windows first, Ubuntu after — PO decision 2026-09-12).
//
// v1.7.6 P4 (#103) adds two hardenings:
// - a DEV GUARD: enabling refuses in dev builds — dev paths poisoned the
//   OS login entry once already (the 1.7.5-app-meets-1.7.1-daemon bug
//   started with a Run key pointing into the dev tree);
// - a SELF-HEAL (`heal_autostart`): a release build verifies at startup
//   that the platform artifact agrees with the persisted intent AND points
//   at the installed daemon, repairing/removing it when it does not — the
//   already-poisoned machines are fixed by the update itself, with no
//   manual registry editing.

use std::path::PathBuf;

/// The LaunchAgent label / plist file stem — derived from the app identifier
/// (com.umux.app) so the daemon's agent is recognizably umux's own.
const PLIST_LABEL: &str = "com.umux.storestation";
/// The HKCU Run value name and the systemd unit name.
const MECHANISM_NAME: &str = "umux-storestation";

// --- Pure artifact generators (unit-tested on every OS) ----------------------

/// The Windows HKCU Run value: launch headless, hidden console (`--hidden` —
/// the daemon hides its own console window on start; a Run-key launch of a
/// console binary would otherwise flash one at every login). The quoted path
/// survives spaces (NSIS's default per-user install dir does).
pub fn run_key_value(daemon_path: &str) -> String {
    format!("\"{daemon_path}\" run --hidden")
}

/// The macOS LaunchAgent plist (`~/Library/{PLIST_LABEL}.plist`): run the
/// daemon at login, no keepalive — autostart is a login trigger, NOT a
/// supervisor (no auto-restart loop, per the issue's out-of-scope list).
pub fn launchagent_plist(daemon_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{PLIST_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{daemon_path}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    )
}

/// The plist's on-disk path (the user's own LaunchAgents directory).
pub fn launchagent_plist_path(home: &std::path::Path) -> PathBuf {
    home.join("Library")
        .join("LaunchAgents")
        .join(format!("{PLIST_LABEL}.plist"))
}

/// The Linux systemd USER unit (`~/.config/systemd/user/…`): start at login
/// through `default.target`, plain process (no restart supervision — the
/// daemon's own lifecycle stays the user's via Settings / `stop`).
pub fn systemd_unit(daemon_path: &str) -> String {
    format!(
        r#"[Unit]
Description=umux Storestation (headless daemon)
Documentation=https://github.com/CrystalPlatforms/umux

[Service]
ExecStart={daemon_path} run

[Install]
WantedBy=default.target
"#
    )
}

/// The unit's on-disk path (the user's own systemd dir).
pub fn systemd_unit_path(home: &std::path::Path) -> PathBuf {
    home.join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{MECHANISM_NAME}.service"))
}

// --- Runtime plumbing --------------------------------------------------------

/// The user's home directory (USERPROFILE on Windows, HOME elsewhere) — the
/// base for the LaunchAgents and systemd paths. Only the macOS/Linux
/// autostart paths need it (Windows writes the registry directly), so the
/// Windows build does not even compile it.
#[cfg(not(target_os = "windows"))]
fn home_dir() -> Result<PathBuf, String> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .ok_or_else(|| format!("environment variable {var} is not set — cannot locate the home directory"))
}

/// Where the daemon binary lives for the autostart artifact: beside THIS app
/// executable (the installer layout — phase 8 ships `umux-storestation`
/// inside every installer) or the bare name, resolved through PATH by the
/// OS. Mirrors `spawn_daemon_if_absent`'s lookup in lib.rs.
pub fn daemon_binary_for_autostart() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent().map(|dir| {
                dir.join(format!(
                    "umux-storestation{}",
                    std::env::consts::EXE_SUFFIX
                ))
            })
        })
        .filter(|path| path.is_file())
}

/// A command that never flashes a console window (Windows). Shared by the
/// reg.exe calls here and the app-side daemon spawn (v1.7.6 P4, #103: the
/// Settings toggle must flash nothing).
#[cfg(windows)]
pub(crate) fn hidden(mut command: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub(crate) fn hidden(command: std::process::Command) -> std::process::Command {
    command
}

/// The dev-build refusal (#103, story 6) — `Some(message)` in dev builds,
/// where enabling autostart must refuse, `None` in release builds, where it
/// proceeds. A pure function so the guard is unit-testable in BOTH
/// configurations: `cargo test` runs debug (refusal present), a release
/// test run sees none.
fn dev_refusal_message() -> Option<&'static str> {
    if cfg!(debug_assertions) {
        Some(
            "autostart cannot be enabled from a dev build — dev paths poison the OS \
             login entry (the v1.7.5 stale-daemon bug started exactly there). \
             Install a release build and enable autostart from it.",
        )
    } else {
        None
    }
}

/// Install (enable) or remove (disable) the login mechanism. `Ok` = the
/// artifact is in place / gone; an `Err` names what refused (no systemd, no
/// home dir, a failed OS command, a dev build) so Settings can show it.
pub fn set_autostart(enable: bool) -> Result<(), String> {
    if let Some(message) = dev_refusal_message().filter(|_| enable) {
        return Err(message.into());
    }
    let Some(daemon) = daemon_binary_for_autostart() else {
        return Err(
            "could not locate umux-storestation next to the umux app — reinstall or repair the app"
                .into(),
        );
    };
    let daemon_path = daemon.display().to_string();
    #[cfg(windows)]
    {
        set_windows(enable, &daemon_path)
    }
    #[cfg(target_os = "macos")]
    {
        set_macos(enable, &daemon_path)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        set_linux(enable, &daemon_path)
    }
}

/// Windows: the HKCU Run key. `reg add ... /f` overwrites; `reg delete`
/// tolerates an absent value (disabling twice is fine). HKEY_CURRENT_USER
/// needs no elevation (no admin paths, per the out-of-scope list).
#[cfg(windows)]
fn set_windows(enable: bool, daemon_path: &str) -> Result<(), String> {
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    if enable {
        let value = run_key_value(daemon_path);
        let status = hidden(std::process::Command::new("reg"))
            .args(["add", RUN_KEY, "/v", MECHANISM_NAME, "/t", "REG_SZ", "/d", &value, "/f"])
            .status()
            .map_err(|e| format!("could not run reg add: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("reg add exited with {status}"))
        }
    } else {
        let status = hidden(std::process::Command::new("reg"))
            .args(["delete", RUN_KEY, "/v", MECHANISM_NAME, "/f"])
            .status()
            .map_err(|e| format!("could not run reg delete: {e}"))?;
        // Exit 1 = the value simply isn't there — removal achieved.
        if status.success() || status.code() == Some(1) {
            Ok(())
        } else {
            Err(format!("reg delete exited with {status}"))
        }
    }
}

/// macOS: a LaunchAgent plist in the user's own LaunchAgents directory.
/// `launchctl load` activates it for THIS session too (the reboot is the
/// HITL proof, not a requirement); `unload` before removal deactivates.
#[cfg(target_os = "macos")]
fn set_macos(enable: bool, daemon_path: &str) -> Result<(), String> {
    let home = home_dir()?;
    let plist = launchagent_plist_path(&home);
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create LaunchAgents: {e}"))?;
    }
    if enable {
        std::fs::write(&plist, launchagent_plist(daemon_path))
            .map_err(|e| format!("could not write {}: {e}", plist.display()))?;
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg(&plist)
            .status(); // not yet loaded — fine
        let status = std::process::Command::new("launchctl")
            .arg("load")
            .arg(&plist)
            .status()
            .map_err(|e| format!("could not run launchctl: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("launchctl load exited with {status}"))
        }
    } else {
        if plist.exists() {
            let _ = std::process::Command::new("launchctl")
                .arg("unload")
                .arg(&plist)
                .status(); // best effort — removal continues regardless
        }
        std::fs::remove_file(&plist)
            .map_err(|e| format!("could not remove {}: {e}", plist.display()))
    }
}

/// Linux: a systemd user unit. Enabling needs a user systemd instance —
/// where the OS refuses (no systemd, WSL without it), the error says so and
/// nothing is half-installed (the unit file is only kept on success).
#[cfg(all(unix, not(target_os = "macos")))]
fn set_linux(enable: bool, daemon_path: &str) -> Result<(), String> {
    let home = home_dir()?;
    let unit = systemd_unit_path(&home);
    if enable {
        if let Some(parent) = unit.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create the systemd user dir: {e}"))?;
        }
        std::fs::write(&unit, systemd_unit(daemon_path))
            .map_err(|e| format!("could not write {}: {e}", unit.display()))?;
        for args in [
            vec!["--user", "daemon-reload"],
            vec!["--user", "enable", MECHANISM_NAME],
        ] {
            let status = std::process::Command::new("systemctl")
                .args(&args)
                .status()
                .map_err(|e| {
                    let _ = std::fs::remove_file(&unit);
                    format!(
                        "could not run systemctl (is systemd running as a user manager?): {e}"
                    )
                })?;
            if !status.success() {
                let _ = std::fs::remove_file(&unit);
                return Err(format!("systemctl {} failed", args.join(" ")));
            }
        }
        Ok(())
    } else {
        // Disable + stop first, then the file — the "removed fully" rule:
        // no orphan units, no leftover enabled symlink, nothing running from
        // a file that no longer exists.
        if unit.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", MECHANISM_NAME])
                .status();
        }
        match std::fs::remove_file(&unit) {
            Ok(()) => {
                let _ = std::process::Command::new("systemctl")
                    .args(["--user", "daemon-reload"])
                    .status();
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("could not remove {}: {e}", unit.display())),
        }
    }
}

// --- Self-heal (v1.7.6 P4, #103 story 7) -------------------------------------

/// What `heal_autostart` decided — the log line the app writes (silent by
/// contract: Settings already shows the daemon status, the heal adds no UI).
#[derive(Debug, PartialEq, Eq)]
pub enum HealOutcome {
    /// The artifact exists, matches the intent, and points at the installed
    /// daemon — nothing to do.
    AlreadyCorrect,
    /// Intent ON but no artifact existed (a user who never enabled it is
    /// NEVER touched; this is the post-update restore: settings.json
    /// remembers ON, the uninstaller half of an in-app update removed the
    /// artifact) — installed fresh.
    Installed,
    /// Artifact existed but pointed anywhere other than the installed daemon
    /// (the poisoned dev-tree key) — rewritten to the installed daemon.
    Rewritten,
    /// Intent OFF but an artifact existed — removed (a leftover must not
    /// survive).
    Removed,
    /// Intent ON but no daemon binary beside the app — nothing to point the
    /// artifact at (a broken install); reported, never half-installed.
    NoDaemon,
}

/// The action the heal executor takes for one artifact.
#[derive(Debug, PartialEq, Eq)]
enum HealAction {
    Leave,
    Install,
    Rewrite,
    Remove,
}

/// Pure: the heal decision (#103) for one (intent, artifact state) pair —
/// the four reconciliations of the release startup check. Unit-tested on
/// every OS; `heal_autostart` only executes what this returns.
fn heal_decision(
    intent_enabled: bool,
    artifact_present: bool,
    points_here: bool,
    daemon_available: bool,
) -> (HealAction, HealOutcome) {
    match (intent_enabled, artifact_present) {
        // Intent OFF: an artifact must not survive its own setting.
        (false, true) => (HealAction::Remove, HealOutcome::Removed),
        (false, false) => (HealAction::Leave, HealOutcome::AlreadyCorrect),
        // Intent ON: repair toward the installed daemon — but only when
        // there IS one (a broken install is reported, never half-fixed).
        (true, _) if !daemon_available => (HealAction::Leave, HealOutcome::NoDaemon),
        (true, false) => (HealAction::Install, HealOutcome::Installed),
        (true, true) if !points_here => (HealAction::Rewrite, HealOutcome::Rewritten),
        (true, true) => (HealAction::Leave, HealOutcome::AlreadyCorrect),
    }
}

/// Pure: the daemon path inside a Run-key value. The generator always writes
/// a QUOTED path (`"C:\…\umux-storestation.exe" run --hidden`); an unquoted
/// legacy form is tolerated (path up to the last token boundary before
/// `run`). `None` = the value is too mangled to trust — heal rewrites it.
pub fn run_key_daemon_path(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(&rest[..end]);
    }
    // Unquoted legacy form: everything before the " run" argument separator.
    match trimmed.find(" run") {
        Some(at) => Some(trimmed[..at].trim_end()),
        None if !trimmed.is_empty() => Some(trimmed),
        None => None,
    }
}

/// Pure: does the stored Run value launch THIS daemon? Windows paths are
/// case-insensitive and tolerate a trailing separator difference.
fn install_paths_agree(stored: &str, installed: &str) -> bool {
    let normalize = |p: &str| p.trim().trim_end_matches(['/', '\\']).to_lowercase();
    normalize(stored) == normalize(installed)
}

/// Pure: does a Run-key value point at `daemon_path`?
pub fn run_key_points_at(value: &str, daemon_path: &str) -> bool {
    match run_key_daemon_path(value) {
        Some(stored) => install_paths_agree(stored, daemon_path),
        None => false,
    }
}

/// Pure: does a LaunchAgent plist's ProgramArguments carry `daemon_path`?
/// (macOS builds call it from `artifact_points_at`; on other platforms only
/// the tests do — the pure heal checks are unit-tested on every OS.)
#[allow(dead_code)]
fn plist_points_at(plist: &str, daemon_path: &str) -> bool {
    let needle = format!("<string>{daemon_path}</string>");
    plist.contains(&needle)
}

/// Pure: does a systemd unit's ExecStart run `daemon_path`? (Same
/// cross-platform test story as `plist_points_at`.)
#[allow(dead_code)]
fn unit_points_at(unit: &str, daemon_path: &str) -> bool {
    unit.lines()
        .map(str::trim)
        .any(|line| line == format!("ExecStart={daemon_path} run"))
}

/// The release startup check (#103): reconcile the platform artifact with
/// the persisted intent (`storestation.autostartEnabled`) and the installed
/// daemon. Runs ONLY in release builds (a dev build must never touch the OS
/// artifact — the same rule the dev guard enforces for the toggle); the app
/// calls it from a background thread at setup, logging the outcome.
///
/// The four reconciliations:
///   intent ON  + artifact missing  → install (the post-update restore)
///   intent ON  + artifact wrong    → rewrite to the installed daemon
///   intent OFF + artifact present  → remove (a leftover must not survive)
///   otherwise                      → leave alone
pub fn heal_autostart(intent_enabled: bool) -> Result<HealOutcome, String> {
    let current: Option<String> = read_current_artifact()?;
    let daemon_available = daemon_binary_for_autostart().is_some();
    let daemon_path = daemon_binary_for_autostart().map(|d| d.display().to_string());
    let points_here = match (&current, &daemon_path) {
        (Some(artifact), Some(path)) => artifact_points_at(artifact, path),
        _ => false,
    };
    let (action, outcome) = heal_decision(
        intent_enabled,
        current.is_some(),
        points_here,
        daemon_available,
    );
    match action {
        HealAction::Leave => Ok(outcome),
        HealAction::Remove => set_autostart(false).map(|_| outcome),
        HealAction::Install | HealAction::Rewrite => {
            set_autostart(true).map(|_| outcome)
        }
    }
}

/// The artifact's current form, per platform: the Run value verbatim
/// (Windows), the plist/unit file text (macOS/Linux). `None` = absent.
fn read_current_artifact() -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        read_windows_run_value()
    }
    #[cfg(target_os = "macos")]
    {
        let home = home_dir()?;
        let plist = launchagent_plist_path(&home);
        std::fs::read_to_string(&plist)
            .map(Some)
            .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(None) } else { Err(format!("could not read {}: {e}", plist.display())) })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = home_dir()?;
        let unit = systemd_unit_path(&home);
        std::fs::read_to_string(&unit)
            .map(Some)
            .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(None) } else { Err(format!("could not read {}: {e}", unit.display())) })
    }
}

/// Which comparison the current artifact undergoes (the Run value's daemon
/// path vs the plist/unit text search — see the pure helpers above).
fn artifact_points_at(artifact: &str, daemon_path: &str) -> bool {
    #[cfg(windows)]
    {
        run_key_points_at(artifact, daemon_path)
    }
    #[cfg(target_os = "macos")]
    {
        plist_points_at(artifact, daemon_path)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        unit_points_at(artifact, daemon_path)
    }
}

/// Windows: read the HKCU Run value (`reg query`, windowless). `None` when
/// the value is absent — the query exits 1 for a missing value.
#[cfg(windows)]
fn read_windows_run_value() -> Result<Option<String>, String> {
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let output = hidden(std::process::Command::new("reg"))
        .args(["query", RUN_KEY, "/v", MECHANISM_NAME])
        .output()
        .map_err(|e| format!("could not run reg query: {e}"))?;
    if !output.status.success() {
        return Ok(None); // value not present (or key gone) — that is "absent"
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // A query answer's value line: `    umux-storestation    REG_SZ    <data>`.
    Ok(text
        .lines()
        .find(|line| line.contains("REG_SZ"))
        .and_then(|line| line.split("REG_SZ").nth(1))
        .map(str::trim)
        .map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    // AC (#89): the generated Run-key value launches the daemon HEADLESS —
    // quoted path (installs with spaces), `run`, `--hidden`. The quoted path
    // is the NSIS reality ("C:\Users\Adam\AppData\Local\Programs\umux\...").
    #[test]
    fn run_key_value_quotes_path_and_runs_hidden() {
        let value = run_key_value(r"C:\Program Files\umux\umux-storestation.exe");
        assert_eq!(
            value,
            r#""C:\Program Files\umux\umux-storestation.exe" run --hidden"#
        );
    }

    // AC (#89): the LaunchAgent plist is well-formed, runs the daemon at
    // login, and carries NO keepalive (autostart is a login trigger, not a
    // supervisor).
    #[test]
    fn launchagent_plist_is_valid_and_run_at_load() {
        let plist = launchagent_plist("/Applications/umux.app/Contents/MacOS/umux-storestation");
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains("<string>com.umux.storestation</string>"));
        // The program arguments are the daemon path + `run`, in order.
        let args_at = plist.find("<key>ProgramArguments</key>").expect("args key");
        let daemon_at = plist[args_at..]
            .find("<string>/Applications/umux.app/Contents/MacOS/umux-storestation</string>")
            .expect("daemon path in args");
        let run_at = plist[args_at..].find("<string>run</string>").expect("run arg");
        assert!(daemon_at < run_at, "`run` must follow the daemon path");
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(!plist.contains("KeepAlive"), "no supervisor semantics");
        // The path helper lands in the user's own LaunchAgents dir.
        let path = launchagent_plist_path(std::path::Path::new("/Users/adam"));
        assert_eq!(
            path,
            PathBuf::from("/Users/adam/Library/LaunchAgents/com.umux.storestation.plist")
        );
    }

    // AC (#89): the systemd user unit starts the daemon at login through
    // default.target and nothing else.
    #[test]
    fn systemd_unit_is_valid_and_starts_at_login() {
        let unit = systemd_unit("/usr/bin/umux-storestation");
        assert!(unit.starts_with("[Unit]"));
        assert!(unit.contains("ExecStart=/usr/bin/umux-storestation run"));
        assert!(unit.contains("WantedBy=default.target"));
        assert!(!unit.contains("Restart="), "no auto-restart loop");
        let path = systemd_unit_path(std::path::Path::new("/home/adam"));
        assert_eq!(
            path,
            PathBuf::from("/home/adam/.config/systemd/user/umux-storestation.service")
        );
    }

    // --- v1.7.6 P4 (#103): the self-heal's pure halves -----------------------

    // The Run value's daemon path parses in every form the artifact can
    // legitimately take (the generator always quotes; a legacy unquoted
    // form is tolerated), and refuses to guess at mangled values.
    #[test]
    fn run_key_daemon_path_parses_quoted_unquoted_and_mangled() {
        assert_eq!(
            run_key_daemon_path(
                r#""C:\Program Files\umux\umux-storestation.exe" run --hidden"#
            ),
            Some(r"C:\Program Files\umux\umux-storestation.exe")
        );
        assert_eq!(
            run_key_daemon_path(r#""C:\umux\umux-storestation.exe""#),
            Some(r"C:\umux\umux-storestation.exe")
        );
        assert_eq!(
            run_key_daemon_path(r"C:\umux\umux-storestation.exe run"),
            Some(r"C:\umux\umux-storestation.exe")
        );
        // Mangled → None (the heal treats "cannot parse" as "rewrite").
        assert_eq!(run_key_daemon_path(""), None);
        assert_eq!(run_key_daemon_path("\"unclosed"), None);
    }

    // THE POISONED-MACHINE CASE (story 7): the historical dev-tree Run key
    // must NOT count as pointing at the installed daemon, the healed value
    // must, and Windows case/trailing-separator tolerance holds.
    #[test]
    fn poisoned_dev_tree_key_is_detected_and_healed_value_accepted() {
        let installed = r"C:\Users\panad\AppData\Local\Programs\umux\umux-storestation.exe";
        let poisoned = r#""C:\Users\panad\Documents\umux\src-tauri\target\debug\umux-storestation.exe" run --hidden"#;
        assert!(!run_key_points_at(poisoned, installed));
        assert!(run_key_points_at(&run_key_value(installed), installed));
        // Case-insensitive + trailing separator tolerance.
        assert!(run_key_points_at(
            &run_key_value(r"C:\Users\PANAD\AppData\Local\Programs\umux\umux-storestation.exe\"),
            installed
        ));
    }

    // The plist check matches the daemon path inside ProgramArguments —
    // and only there.
    #[test]
    fn plist_points_at_checks_program_arguments() {
        let plist = launchagent_plist("/Applications/umux.app/Contents/MacOS/umux-storestation");
        assert!(plist_points_at(
            &plist,
            "/Applications/umux.app/Contents/MacOS/umux-storestation"
        ));
        assert!(!plist_points_at(
            &plist,
            "/Users/adam/dev/umux/target/debug/umux-storestation"
        ));
        // A path that merely CONTAINS the daemon path as a substring of
        // another path does not count.
        assert!(!plist_points_at(
            "<string>/Applications/umux.app/Contents/MacOS/umux-storestation-old</string>",
            "/Applications/umux.app/Contents/MacOS/umux-storestation"
        ));
    }

    // The unit check matches the exact ExecStart line the generator writes.
    #[test]
    fn unit_points_at_checks_exec_start() {
        let unit = systemd_unit("/usr/bin/umux-storestation");
        assert!(unit_points_at(&unit, "/usr/bin/umux-storestation"));
        assert!(!unit_points_at(
            &unit,
            "/home/adam/dev/umux/target/debug/umux-storestation"
        ));
        assert!(!unit_points_at(
            "ExecStart=/usr/bin/umux-storestation serve",
            "/usr/bin/umux-storestation"
        ));
    }

    // Story 6: the dev guard refuses ENABLING in dev builds (the `cargo
    // test` configuration IS a dev build) and the refusal precedes any OS
    // lookup — a release test run just proceeds to the ordinary path.
    #[test]
    fn dev_guard_refuses_enabling_in_dev_builds() {
        match dev_refusal_message() {
            Some(message) => {
                assert!(message.contains("dev build"));
                assert!(set_autostart(true).unwrap_err().contains("dev build"));
            }
            None => {
                // Release: no guard; set_autostart(true) reports the missing
                // daemon beside the TEST binary instead — never a real write
                // (nothing named umux-storestation lives in the test dir).
                assert!(set_autostart(true)
                    .unwrap_err()
                    .contains("could not locate"));
            }
        }
        // DISABLING is never guarded — cleanup must work from any build
        // (the failure here is the daemon lookup beside the TEST binary,
        // never the dev refusal).
        let disable_err = set_autostart(false).unwrap_err();
        assert!(!disable_err.contains("dev build"));
    }

    // The heal decision matrix (#103 story 7) — every reconciliation the
    // release startup check can take, including the fresh-install safety
    // rule (intent OFF + nothing present = untouched).
    #[test]
    fn heal_decision_covers_every_reconciliation() {
        use HealAction::*;
        // A user who never enabled it: untouched (default-OFF preserved).
        assert_eq!(
            heal_decision(false, false, false, true),
            (Leave, HealOutcome::AlreadyCorrect)
        );
        // Intent OFF + leftover artifact → removed.
        assert_eq!(
            heal_decision(false, true, false, true),
            (Remove, HealOutcome::Removed)
        );
        // Intent ON + artifact missing (the post-update restore) → installed.
        assert_eq!(
            heal_decision(true, false, false, true),
            (Install, HealOutcome::Installed)
        );
        // Intent ON + poisoned target (the dev-tree key) → rewritten.
        assert_eq!(
            heal_decision(true, true, false, true),
            (Rewrite, HealOutcome::Rewritten)
        );
        // Intent ON + already correct → untouched.
        assert_eq!(
            heal_decision(true, true, true, true),
            (Leave, HealOutcome::AlreadyCorrect)
        );
        // Intent ON + broken install (no daemon beside the app) → reported,
        // never half-installed.
        assert_eq!(
            heal_decision(true, false, false, false),
            (Leave, HealOutcome::NoDaemon)
        );
        assert_eq!(
            heal_decision(true, true, false, false),
            (Leave, HealOutcome::NoDaemon)
        );
    }
}
