//! cmux_import — the read-only file bridge for the cmux importer (#54).
//!
//! The importer's logic is pure and lives in the frontend (`src/cmuxImport.ts`);
//! the frontend cannot read arbitrary paths itself, so this module is the thin
//! boundary: it reads cmux's two source files — the config
//! (`~/.config/cmux/cmux.json`) and the session store
//! (`session-*.json` under cmux's data directory) — and hands their CONTENTS
//! to the caller as strings. STRICTLY READ-ONLY: `fs::read_to_string` only,
//! a missing or unreadable file simply reports `None` (a flat import), and
//! nothing here ever writes, renames, or deletes anything under cmux's
//! directories.

use serde::Serialize;
use std::fs;
use std::path::PathBuf;

/// The two source files' contents; `None` = absent or unreadable (the
/// importer treats that as "flat import from whatever exists").
#[derive(Serialize)]
pub struct CmuxImportSources {
    pub config: Option<String>,
    pub session: Option<String>,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// cmux's config file, per its own cross-platform convention: the XDG-style
/// `~/.config/cmux/cmux.json` (the location observed on the PO's Mac too).
fn cmux_config_path(home: &std::path::Path) -> PathBuf {
    home.join(".config").join("cmux").join("cmux.json")
}

/// cmux's data directory holding `session-*.json`, best-effort per platform:
/// the Mac path is confirmed (`~/Library/Application Support/cmux`); the
/// others are the conventional data locations for a config in `~/.config`.
fn cmux_data_dir(home: &std::path::Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("cmux")
    } else if cfg!(target_os = "windows") {
        home.join("AppData").join("Roaming").join("cmux")
    } else {
        home.join(".local").join("share").join("cmux")
    }
}

/// Pick the LIVE session store out of a directory: `session-*.json`, skipping
/// backups (`*-previous.json`), alphabetically first otherwise. Deterministic,
/// no I/O — the caller owns the read.
fn pick_session_file(dir: &std::path::Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("session-") && name.ends_with(".json")
        })
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !name.contains("previous")
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

fn read_text(path: Option<PathBuf>) -> Option<String> {
    fs::read_to_string(path?).ok()
}

/// The Tauri command: hand over both source files' contents (read-only).
#[tauri::command]
pub fn read_cmux_import_sources() -> CmuxImportSources {
    match home_dir() {
        None => CmuxImportSources {
            config: None,
            session: None,
        },
        Some(home) => {
            let session = pick_session_file(&cmux_data_dir(&home)).and_then(|p| read_text(Some(p)));
            CmuxImportSources {
                config: read_text(Some(cmux_config_path(&home))),
                session,
            }
        }
    }
}
