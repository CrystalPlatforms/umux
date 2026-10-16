//! The daemon version gate (v1.7.6 P2, #102) — the pure half of the
//! decision the daemon-client driver makes at its ONE choke point (the
//! hello handshake). The driver compares the handshake's `daemonVersion`
//! against the app's own version:
//!
//! - same version, or a NEWER daemon on the SAME protocol major →
//!   [`GateVerdict::Compatible`] — the protocol grows additively, so a
//!   same-major newer daemon still serves this client;
//! - an OLDER daemon (any older triple) → [`GateVerdict::DaemonOlder`] —
//!   the driver replaces it with the bundled daemon (the 1.7.5 app meeting
//!   the stale 1.7.1 daemon must never crash again);
//! - a NEWER-MAJOR daemon → [`GateVerdict::DaemonNewerMajor`] — no
//!   downgrade attempt: refuse with a clear message (a Settings error
//!   card), leaving the running daemon alone;
//! - a missing/unparseable version → [`GateVerdict::Unparseable`] — a
//!   malformed handshake reply is a typed outcome, never a panic.
//!
//! No I/O here — fixture handshakes in, verdict out (the driver-crate unit
//! tests feed exactly those).

/// The verdict for one (daemon version, app version) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// Serve from this daemon.
    Compatible,
    /// The daemon predates the app — replace it with the bundled daemon.
    DaemonOlder,
    /// The daemon speaks a newer protocol major than this app — refuse, no
    /// downgrade attempt.
    DaemonNewerMajor,
    /// The handshake carried no usable `daemonVersion` — treat as
    /// incompatible (refuse with a repair hint), never crash.
    Unparseable,
}

/// Parse a "major.minor.patch" prefix (a trailing `-pre`/`+meta` is
/// ignored; missing components read as 0; a non-numeric component rejects
/// the whole string). `None` = not a version.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.split(['-', '+']).next()?.trim();
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = match parts.next() {
        Some(p) => p.parse().ok()?,
        None => 0,
    };
    let patch = match parts.next() {
        Some(p) => p.parse().ok()?,
        None => 0,
    };
    Some((major, minor, patch))
}

/// Classify a handshake's daemon version against this app's version.
pub fn classify_daemon_version(daemon: Option<&str>, app: &str) -> GateVerdict {
    use std::cmp::Ordering;
    let Some(daemon_v) = daemon.and_then(parse_version) else {
        // No usable version from the daemon — the malformed-reply outcome.
        return GateVerdict::Unparseable;
    };
    let Some(app_v) = parse_version(app) else {
        // An unparseable APP version is a build bug with the same result.
        return GateVerdict::Unparseable;
    };
    match daemon_v.0.cmp(&app_v.0) {
        Ordering::Less => GateVerdict::DaemonOlder,
        Ordering::Greater => GateVerdict::DaemonNewerMajor,
        Ordering::Equal => {
            if daemon_v < app_v {
                GateVerdict::DaemonOlder
            } else {
                GateVerdict::Compatible
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The happy path: app and daemon from the same build.
    #[test]
    fn same_version_is_compatible() {
        assert_eq!(
            classify_daemon_version(Some("1.7.6"), "1.7.6"),
            GateVerdict::Compatible
        );
    }

    // The bug this gate exists for: the stale daemon (1.7.1) must classify
    // as OLDER — replaceable, never crash-worthy.
    #[test]
    fn older_daemon_classifies_for_replacement() {
        assert_eq!(
            classify_daemon_version(Some("1.7.1"), "1.7.5"),
            GateVerdict::DaemonOlder
        );
        // An older MAJOR is equally "older".
        assert_eq!(
            classify_daemon_version(Some("0.9.0"), "1.0.0"),
            GateVerdict::DaemonOlder
        );
        // Same major, older triple too.
        assert_eq!(
            classify_daemon_version(Some("1.7.5"), "1.7.6"),
            GateVerdict::DaemonOlder
        );
    }

    // A newer daemon on the SAME major is compatible: the protocol grows
    // additively, the client treats unknown ops as generic errors.
    #[test]
    fn newer_same_major_daemon_is_compatible() {
        assert_eq!(
            classify_daemon_version(Some("1.7.7"), "1.7.6"),
            GateVerdict::Compatible
        );
        assert_eq!(
            classify_daemon_version(Some("1.8.0"), "1.7.6"),
            GateVerdict::Compatible
        );
    }

    // A NEWER-MAJOR daemon refuses with the no-downgrade outcome.
    #[test]
    fn newer_major_daemon_refuses() {
        assert_eq!(
            classify_daemon_version(Some("2.0.0"), "1.7.6"),
            GateVerdict::DaemonNewerMajor
        );
    }

    // Fixture handshake shapes — including the malformed ones — classify
    // without panicking.
    #[test]
    fn fixture_handshakes_classify() {
        use serde_json::json;
        let version_of = |fixture: serde_json::Value| -> GateVerdict {
            let version = crate::client::hello_daemon_version(&fixture);
            classify_daemon_version(version.as_deref(), "1.7.6")
        };
        // The real daemon answer.
        assert_eq!(
            version_of(json!({ "proto": 1, "daemonVersion": "1.7.6", "daemonPid": 42 })),
            GateVerdict::Compatible
        );
        // Malformed replies: missing field, wrong type, empty string.
        assert_eq!(
            version_of(json!({ "proto": 1, "daemonPid": 42 })),
            GateVerdict::Unparseable
        );
        assert_eq!(
            version_of(json!({ "proto": 1, "daemonVersion": 17, "daemonPid": 42 })),
            GateVerdict::Unparseable
        );
        assert_eq!(
            version_of(json!({ "daemonVersion": "" })),
            GateVerdict::Unparseable
        );
    }

    // Version parsing tolerances: pre-release suffixes and short triples.
    #[test]
    fn parse_version_tolerates_suffixes_and_short_forms() {
        assert_eq!(parse_version("1.7.6"), Some((1, 7, 6)));
        assert_eq!(parse_version("1.7.6-beta.1"), Some((1, 7, 6)));
        assert_eq!(parse_version("1.7"), Some((1, 7, 0)));
        assert_eq!(parse_version("1"), Some((1, 0, 0)));
        assert_eq!(parse_version("dev"), None);
        assert_eq!(parse_version(""), None);
    }
}
