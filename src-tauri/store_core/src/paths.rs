//! Config-directory resolution — moved verbatim out of the app crate's
//! `lib.rs` (#58): the app and the future `umux` CLI must agree on WHERE the
//! store lives, not just on its format, so the resolution belongs next to the
//! stores it locates.
//!
//! Pure helpers take the base directory (home / APPDATA) as an argument so
//! they are trivially unit-testable; the env-reading `config_dir()` is the
//! only impure entry point.

use std::path::PathBuf;

/// Pure: the macOS config directory under a given home directory
/// (v1.0 Phase 8 / #32). `~/Library/Application Support/umux` — where a Mac
/// app is expected to keep its state.
pub fn macos_config_dir(home: &std::path::Path) -> PathBuf {
    home.join("Library").join("Application Support").join("umux")
}

/// Pure: the XDG (Linux) config directory under a given home directory —
/// the unchanged v0.1 location.
pub fn xdg_config_dir(home: &std::path::Path) -> PathBuf {
    home.join(".config").join("umux")
}

/// Pure: the Windows config directory under a given `%APPDATA%` value
/// (v1.0 Phase 9 / #33) — `...\AppData\Roaming\umux`, where a Windows app is
/// expected to keep its per-user state.
pub fn windows_config_dir(appdata: &std::path::Path) -> PathBuf {
    appdata.join("umux")
}

/// The per-user config directory shared by every umux config file.
/// Platform-correct since v1.0 Phase 8 / #32 (macOS) and Phase 9 / #33
/// (Windows):
///  - macOS: `$HOME/Library/Application Support/umux` (XDG vars are ignored
///    there — the Library path is where Mac users and backup tools look)
///  - Windows: `%APPDATA%\umux` (Roaming — the per-user, roaming-profile
///    location; umux never ran on Windows before, so there is no legacy
///    directory to migrate and migrate_legacy_config no-ops)
///  - Linux and everything else: `$XDG_CONFIG_HOME/umux` or `~/.config/umux`
///
/// `UMUX_CONFIG_DIR` (when set and non-empty) moves the whole store root —
/// the escape hatch the `umux` CLI test suite uses to point spawned binaries
/// at a tempdir (#60; the app never sets it). Tested via spawned processes:
/// an env var is process-global, so in-process unit tests would race.
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("UMUX_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .map(|h| macos_config_dir(std::path::Path::new(&h)))
            .unwrap_or_else(|_| PathBuf::from("."))
    } else if cfg!(windows) {
        std::env::var("APPDATA")
            .map(|a| windows_config_dir(std::path::Path::new(&a)))
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        legacy_config_dir()
    }
}

/// Where v0.1 wrote config on every platform — the pre-#32 config_dir. After
/// #32 only macOS reads from a different place, so this is the migration
/// SOURCE there; on Linux it is still config_dir() itself and migrating is
/// a no-op.
pub fn legacy_config_dir() -> PathBuf {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(xdg) if !xdg.is_empty() => PathBuf::from(xdg).join("umux"),
        _ => std::env::var("HOME")
            .map(|h| xdg_config_dir(std::path::Path::new(&h)))
            .unwrap_or_else(|_| PathBuf::from(".")),
    }
}

/// One-time config relocation for the macOS path change (v1.0 Phase 8 /
/// #32): move the config files a v0.1 dev build left in `~/.config/umux`
/// into the platform-correct directory, so an upgrading user keeps their
/// workspaces and settings. Runs BEFORE anything reads config (the settings
/// seed in the app's run() is the first reader). Logged best-effort — at this
/// point no logger is installed yet, so failures continue silently, which is
/// the same guarantee as v0.1's fallback: worst case the user starts with
/// defaults, never a crash.
///
/// Safety rails: only the two known file names move, nothing is ever
/// overwritten (a file already at the destination leaves the source alone),
/// and same-path calls (Linux) return immediately.
pub fn migrate_legacy_config(new_dir: &std::path::Path, legacy_dir: &std::path::Path) {
    if new_dir == legacy_dir {
        return; // same location (Linux): nothing to relocate
    }
    for name in ["workspaces.json", "settings.json"] {
        let from = legacy_dir.join(name);
        let to = new_dir.join(name);
        if !from.is_file() || to.exists() {
            continue;
        }
        let result = std::fs::create_dir_all(new_dir).and_then(|_| std::fs::rename(&from, &to));
        match result {
            Ok(()) => log::info!(
                "[config] migrated {name} from {} to {}",
                legacy_dir.display(),
                new_dir.display()
            ),
            Err(e) => log::warn!("[config] could not migrate {name}: {e} (continuing)"),
        }
    }
}

/// Where the workspace config file lives: `<config_dir>/workspaces.json`.
pub fn config_path() -> PathBuf {
    config_dir().join("workspaces.json")
}

/// Where the settings file lives: `<config_dir>/settings.json` (v0.2 Phase 3 /
/// #27 — same directory, same corruption fallback as workspaces).
pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// Factory reset (#74): remove EVERY umux state file from `dir` —
/// workspaces.json (workspace/layout/tree definitions) and settings.json —
/// so the next launch boots first-run clean. A missing file is already
/// reset (normal on first run); anything else in the directory that is not
/// a umux state file is left alone.
pub fn reset_store_files(dir: &std::path::Path) -> std::io::Result<()> {
    for name in ["workspaces.json", "settings.json"] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            // Already reset — the normal first-run shape.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The terminal-UI store root (#60 assumption): a `term/` sibling under the
/// same config root, so the `--desk` and `--term` stores can never be
/// confused yet live in one place a user can find. The TUI that reads it
/// ships in v1.7.0; the CLI manages it offline today.
pub fn term_config_dir() -> PathBuf {
    config_dir().join("term")
}

/// Where the terminal-UI workspace config lives:
/// `<config_dir>/term/workspaces.json` — same file name as the desktop
/// store, same format, different directory.
pub fn term_config_path() -> PathBuf {
    term_config_dir().join("workspaces.json")
}

/// Where the terminal-UI settings live:
/// `<config_dir>/term/settings.json`.
pub fn term_settings_path() -> PathBuf {
    term_config_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-P1 (AC2 — macOS config lives under Application Support, not ~/.config):
    //   Input:  home "/Users/adam"
    //   Output: /Users/adam/Library/Application Support/umux — the path a Mac
    //           app is expected to use, independent of XDG env vars.
    #[test]
    fn macos_config_dir_is_application_support() {
        assert_eq!(
            macos_config_dir(std::path::Path::new("/Users/adam")),
            PathBuf::from("/Users/adam/Library/Application Support/umux")
        );
    }

    // T-P2 (Linux unchanged — the XDG default shape survives the split):
    //   Input:  home "/home/adam"
    //   Output: /home/adam/.config/umux — exactly the v0.1 location.
    #[test]
    fn xdg_config_dir_is_dot_config() {
        assert_eq!(
            xdg_config_dir(std::path::Path::new("/home/adam")),
            PathBuf::from("/home/adam/.config/umux")
        );
    }

    // T-P3 (regression guard, #32 HITL: the first build shipped
    //   legacy_config_dir with `~/.config/umux/umux` — xdg_config_dir already
    //   carries the trailing "umux", and an extra join silently broke BOTH
    //   the Linux config path and the macOS migration source). The helper
    //   must carry exactly one umux segment; nothing may nest it deeper.
    #[test]
    fn xdg_config_dir_carries_exactly_one_umux_segment() {
        let dir = xdg_config_dir(std::path::Path::new("/home/adam"));
        assert!(dir.ends_with("umux"), "ends in the umux segment: {dir:?}");
        assert!(
            !dir.ends_with("umux/umux"),
            "double-joined umux segment (the #32 regression): {dir:?}"
        );
    }

    // T-M1 (an upgrading Mac user keeps their config):
    //   Input:  legacy dir holding both config files, new dir absent.
    //   Output: both files live in the new dir and are gone from the legacy
    //           one — the move, not a copy.
    #[test]
    fn migrate_moves_both_files_into_new_dir() {
        let legacy = tempfile::tempdir().unwrap();
        let fresh = tempfile::tempdir().unwrap();
        std::fs::write(legacy.path().join("workspaces.json"), "{}").unwrap();
        std::fs::write(legacy.path().join("settings.json"), "{}").unwrap();

        migrate_legacy_config(fresh.path(), legacy.path());

        assert!(fresh.path().join("workspaces.json").is_file());
        assert!(fresh.path().join("settings.json").is_file());
        assert!(!legacy.path().join("workspaces.json").exists());
        assert!(!legacy.path().join("settings.json").exists());
    }

    // T-M2 (never overwrite — the user's existing config wins):
    //   Input:  destination already has workspaces.json; legacy also has one.
    //   Output: the destination file is untouched and the legacy file stays
    //           put — a re-run of the migration can never clobber anything.
    #[test]
    fn migrate_never_overwrites_existing_destination() {
        let legacy = tempfile::tempdir().unwrap();
        let fresh = tempfile::tempdir().unwrap();
        std::fs::write(legacy.path().join("workspaces.json"), "legacy").unwrap();
        std::fs::write(fresh.path().join("workspaces.json"), "current").unwrap();

        migrate_legacy_config(fresh.path(), legacy.path());

        assert_eq!(
            std::fs::read_to_string(fresh.path().join("workspaces.json")).unwrap(),
            "current",
            "existing destination file is kept verbatim"
        );
        assert!(
            legacy.path().join("workspaces.json").is_file(),
            "legacy file is left in place rather than deleted"
        );
    }

    // T-M3 (no-op cases — a fresh or Linux install must not even create dirs):
    //   Input:  (a) same dir for both arguments (Linux), (b) no legacy files.
    //   Output: nothing moves, nothing crashes, and the new directory is NOT
    //           created when there was nothing to move — no empty-dir spam.
    #[test]
    fn migrate_is_noop_without_legacy_files_or_with_same_dir() {
        let legacy = tempfile::tempdir().unwrap();
        let untouched = tempfile::tempdir().unwrap();
        let never_created = untouched.path().join("does-not-exist");

        // No legacy files: silent no-op, destination untouched.
        migrate_legacy_config(&never_created, legacy.path());
        assert!(!never_created.exists(), "no empty dir created when nothing moved");

        // Same path (Linux): returns before touching the filesystem.
        migrate_legacy_config(legacy.path(), legacy.path());
        assert!(legacy.path().is_dir(), "same-dir call left everything alone");
    }

    // T-R1 (#74 factory reset): reset_store_files removes BOTH state files
    // and nothing else — a foreign file in the directory survives.
    #[test]
    fn reset_store_files_removes_both_and_leaves_others() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("workspaces.json"), "{}").unwrap();
        std::fs::write(dir.path().join("settings.json"), "{}").unwrap();
        std::fs::write(dir.path().join("unrelated.txt"), "keep").unwrap();

        reset_store_files(dir.path()).unwrap();

        assert!(!dir.path().join("workspaces.json").exists());
        assert!(!dir.path().join("settings.json").exists());
        assert!(
            dir.path().join("unrelated.txt").is_file(),
            "foreign files are never touched"
        );
    }

    // T-R2 (#74): an already-reset directory is fine — missing files are the
    // normal first-run shape, not an error.
    #[test]
    fn reset_store_files_tolerates_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        reset_store_files(dir.path()).unwrap();
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(entries.is_empty(), "nothing was created by the reset");
    }
}
