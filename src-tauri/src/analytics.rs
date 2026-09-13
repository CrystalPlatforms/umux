// Analytics — anonymous install/usage counting via Aptabase
// (v0.2 Phase 6 / #30, scoped down by product decision 2026-08-25).
//
// umux sends exactly ONE event: `app_open` on startup. That is all — it
// answers the only question we ask ("how many people installed and use
// umux?", with Aptabase's unique-user counting). No workspace, panel, or
// notification events; no payloads; no commands, output, names, or paths
// can ever leave the machine.
//
// Always on with NO flag anywhere (quickupdate 2026-09-12, Adam — supersedes
// the old analyticsEnabled kill switch, which is gone from the settings
// schema on both sides): the Aptabase plugin registers on every startup and
// nothing can turn it off. Analytics failure of any kind is logged and
// swallowed.

/// The Aptabase app key (public by design — it identifies the app, not the
/// user). `A-EU-…` routes events to the EU datacenter.
pub const APTABASE_APP_KEY: &str = "A-EU-6161652811";

/// The single event umux ever reports: one aggregate counter, no payload.
pub const APP_OPEN_EVENT: &str = "app_open";

/// The Aptabase plugin, built once in run(). ONLY called when analytics is
/// enabled at startup (#30 AC2: when off, the SDK is never initialized —
/// no network call, not merely "no events").
pub fn aptabase_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_aptabase::Builder::new(APTABASE_APP_KEY).build()
}
