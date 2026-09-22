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
/// base for the LaunchAgents and systemd paths.
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

/// A command that never flashes a console window (Windows).
#[cfg(windows)]
fn hidden(mut command: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
#[allow(dead_code)]
fn hidden(command: std::process::Command) -> std::process::Command {
    command
}

/// Install (enable) or remove (disable) the login mechanism. `Ok` = the
/// artifact is in place / gone; an `Err` names what refused (no systemd, no
/// home dir, a failed OS command) so Settings can show it.
pub fn set_autostart(enable: bool) -> Result<(), String> {
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
}
