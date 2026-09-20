//! Where a daemon instance's files live — derived FROM the config dir, so
//! `UMUX_CONFIG_DIR` isolates store + socket + pid together (test instances
//! for free, per the protocol design doc).
//!
//! - Unix (macOS/Linux): UDS at `<config_dir>/core.sock`, permissions 0600.
//! - Windows: named pipe `\\.\pipe\umux-core-<hash>` where `<hash>` is a
//!   short stable hash of the (canonicalized) config dir path — Windows has
//!   no socket filesystem, so the isolation comes from the name instead.

use std::path::{Path, PathBuf};

/// The unix domain socket path for a config dir (unix only).
#[cfg(unix)]
pub fn socket_path(config_dir: &Path) -> PathBuf {
    config_dir.join("core.sock")
}

/// The pid file: `<config_dir>/core.pid`, holding the daemon's pid in
/// decimal. Metadata for humans and the `coreAlreadyRunning` message —
/// liveness itself is always decided by connecting to the socket.
pub fn pid_path(config_dir: &Path) -> PathBuf {
    config_dir.join("core.pid")
}

/// The Windows named-pipe name for a config dir (windows only).
#[cfg(windows)]
pub fn pipe_name(config_dir: &Path) -> String {
    let canonical = std::fs::canonicalize(config_dir).unwrap_or_else(|_| config_dir.to_path_buf());
    format!(
        r"\\.\pipe\{DAEMON_NAME}-{}",
        fnv1a_hex(canonical.to_string_lossy().as_bytes())
    )
}

/// FNV-1a 64-bit as 16 hex chars — dependency-free, stable across runs, so
/// the same config dir always maps to the same pipe name.
#[cfg(windows)]
fn fnv1a_hex(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The pid file name is part of the on-disk contract (tests, the stop
    // cleanup and the stale detection all agree on it).
    #[test]
    fn pid_file_lives_in_the_config_dir() {
        assert_eq!(
            pid_path(Path::new("/tmp/umux")),
            PathBuf::from("/tmp/umux/core.pid")
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_lives_in_the_config_dir() {
        assert_eq!(
            socket_path(Path::new("/tmp/umux")),
            PathBuf::from("/tmp/umux/core.sock")
        );
    }

    // The pipe name embeds the config-dir hash: two different dirs → two
    // pipes (test isolation), the same dir → the same name every time.
    #[cfg(windows)]
    #[test]
    fn pipe_name_is_stable_and_config_dir_scoped() {
        let a = pipe_name(Path::new(r"C:\Users\adam\AppData\Roaming\umux"));
        let b = pipe_name(Path::new(r"C:\Users\adam\AppData\Roaming\umux"));
        let c = pipe_name(Path::new(r"C:\temp\umux-test"));
        assert_eq!(a, b, "same dir → same pipe name");
        assert_ne!(a, c, "different dir → different pipe name");
        assert!(a.starts_with(r"\\.\pipe\umux-core-"));
    }
}
