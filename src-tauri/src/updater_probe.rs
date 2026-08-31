// updater_probe — the honest pre-flight for the update check (issue #66).
//
// The Tauri updater plugin answers `check()` with ONE opaque error whether
// the machine is offline or the release feed simply has no latest.json yet
// (its "could not fetch a valid response" text fits both). Classifying that
// from the frontend by string-matching mislabeled a MISSING FEED as "you
// appear to be offline" — confusing exactly when everything is fine (the
// pre-signing state this repo ships in until the maintainer step runs).
//
// So the app asks its OWN backend first, in the same order a human would:
//   1. unconfigured — the signing pubkey is still the placeholder from
//      tauri.conf.json (the maintainer step hasn't run) → the frontend shows
//      one clear sentence in Settings and the startup check stays silent.
//   2. offline — GitHub itself is unreachable (transport error).
//   3. no-release — GitHub answered, but there is no latest.json (404):
//      a graceful no-op, never an error.
//   4. ok — a feed exists; the plugin check is worth running (and its
//      remaining errors are genuinely interesting: signature, parse, …).
//
// This stays inside the zero-cost policy: the only endpoint probed is the
// same GitHub Releases URL the updater is configured with — no extra server.

use serde::Serialize;

/// Same URL as `plugins.updater.endpoints[0]` in tauri.conf.json — read from
/// the config when available, this constant as the fallback.
pub const DEFAULT_LATEST_JSON_URL: &str =
    "https://github.com/CrystalPlatforms/umux/releases/latest/download/latest.json";

/// The placeholder string tauri.conf.json ships with before the maintainer
/// pastes the real `tauri signer generate` public key.
pub const PUBKEY_PLACEHOLDER: &str = "PASTE_PUBLIC_KEY_FROM_TAURI_SIGNER_GENERATE";

/// What the frontend should do next. `kebab-case` on the wire — the frontend
/// compares these exact strings.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum UpdaterStatus {
    Unconfigured,
    Offline,
    NoRelease,
    Ok,
}

/// Pure classifier for one probe outcome: `Some(status code)` when GitHub
/// answered, `None` when the request never got a response (transport error).
/// Any non-success status means "no usable feed" — 404 is the normal case,
/// and treating exotic codes the same keeps the app graceful either way.
pub fn classify_probe(status: Option<u16>) -> UpdaterStatus {
    match status {
        Some(code) if (200..300).contains(&code) => UpdaterStatus::Ok,
        Some(_) => UpdaterStatus::NoRelease,
        None => UpdaterStatus::Offline,
    }
}

/// The first updater endpoint from tauri.conf.json, falling back to the
/// constant. Config access is defensive: a missing/unshaped config just means
/// the default URL — the probe must never be the thing that breaks.
fn feed_url(app: &tauri::AppHandle) -> String {
    let cfg = app.config();
    cfg.plugins
        .0
        .get("updater")
        .and_then(|v| v.get("endpoints"))
        .and_then(|v| v.get(0))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_LATEST_JSON_URL.to_string())
}

/// Has the maintainer step run? The pubkey is configured when the config
/// carries a non-empty value that is not the placeholder. Defensive for the
/// same reason as `feed_url`: an unshaped config reads as unconfigured, so
/// the app stays quiet instead of surfacing cryptic plugin errors.
pub fn pubkey_configured(app: &tauri::AppHandle) -> bool {
    let cfg = app.config();
    let pubkey = cfg
        .plugins
        .0
        .get("updater")
        .and_then(|v| v.get("pubkey"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    !pubkey.trim().is_empty() && pubkey != PUBKEY_PLACEHOLDER
}

/// Probe the release feed: GET the manifest URL, keep only the status.
/// A manifest is a few KB, so a full GET is cheaper than any cleverness with
/// HEAD + redirect quirks; the body is dropped unread.
pub async fn probe_latest_json(app: &tauri::AppHandle) -> UpdaterStatus {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build();
    let client = match client {
        Ok(c) => c,
        // A client that cannot even be built is an environment failure —
        // the honest bucket is offline (we can't reach the feed).
        Err(_) => return UpdaterStatus::Offline,
    };
    let status = client
        .get(feed_url(app))
        .send()
        .await
        .map(|resp| resp.status().as_u16());
    classify_probe(status.ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-P1 (the user-facing bug this module fixes): GitHub answering 404 —
    // a release feed that simply does not exist yet — must read as
    // "no-release", NOT as offline. This is exactly the mislabel that showed
    // "You appear to be offline" to a user with perfect connectivity.
    #[test]
    fn missing_feed_is_no_release_not_offline() {
        assert_eq!(classify_probe(Some(404)), UpdaterStatus::NoRelease);
    }

    // T-P2 (AC: offline → clear state): no response at all (DNS failure,
    // refused, timeout) is the only thing that reads as offline.
    #[test]
    fn transport_failure_is_offline() {
        assert_eq!(classify_probe(None), UpdaterStatus::Offline);
    }

    // T-P3 (the happy path that unlocks the plugin check): any 2xx means a
    // feed exists and check() is worth running.
    #[test]
    fn success_feed_is_ok() {
        assert_eq!(classify_probe(Some(200)), UpdaterStatus::Ok);
        assert_eq!(classify_probe(Some(302)), UpdaterStatus::NoRelease);
    }
}
