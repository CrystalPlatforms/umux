// SettingsStore — persists the feature toggles (v0.2 Phase 3 / #27).
//
// Deep module split mirroring workspace_store:
//   - parse_settings / serialize_settings: pure, no I/O — trivially
//     unit-testable. parse_settings MUST return defaults rather than panic on
//     a corrupted file (same corruption fallback as workspaces, PRD story 38).
//   - SettingsStore::load / save: thin wrapper over a config path. A missing
//     file is normal on first run -> defaults.
//
// The serialized shape is shared with the TS frontend
// ({ notificationsEnabled, agentStatusEnabled, sessionRestoreEnabled,
// analyticsEnabled, portsTooltipEnabled }), so Rust and JS agree
// byte-for-byte. Defaults: notifications on, agent status on, session restore
// on, analytics on (always — the Settings screen exposes no switch for it;
// nothing is collected until Phase 6 wires Aptabase), ports tooltip on.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::workspace_store::ConfigStatus;

/// Serde default for toggles that default to ON (so a `{}` or partially
/// written file loads them enabled, matching `Settings::default()`).
fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Master gate for desktop notifications. Seeds the app-wide mute flag at
    /// startup (v0.2 Phase 3 / #27 supersedes the session-only mute button:
    /// the bell button and this toggle are one persisted source of truth).
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    /// Hides/shows the per-panel agent status indicators (frontend-only gate;
    /// the status machines keep running either way so re-enabling is instant).
    #[serde(default = "default_true")]
    pub agent_status_enabled: bool,
    /// Session restore (v0.2 Phase 5 / #29 + HITL follow-up): snapshot each
    /// panel's cwd on close and reopen panels in those directories. Off =
    /// layouts and workspace definitions still round-trip (Phase 1
    /// persistence), but every shell starts in the default cwd — the v0.1
    /// behavior.
    #[serde(default = "default_true")]
    pub session_restore_enabled: bool,
    /// Placeholder for the anonymous analytics flag (Phase 6 / #30). Always
    /// ON by default and deliberately NOT user-controllable (product decision,
    /// HITL follow-up: the Settings screen ships no analytics switch). umux
    /// still collects nothing today — Phase 6's Aptabase init is what will
    /// make this flag mean anything.
    #[serde(default = "default_true")]
    pub analytics_enabled: bool,
    /// Master gate for the hover-pulled listening-ports tooltip (#43): tab
    /// rows and sidebar workspace rows. Frontend-only (the tab_ports backend
    /// is only ever called from the tooltip's hover handlers), so off = no
    /// query and no tooltip.
    #[serde(default = "default_true")]
    pub ports_tooltip_enabled: bool,
    /// What to launch when the user starts "umux": `"gui"` (the desktop app,
    /// today's only reality) or `"tui"` (the terminal UI, v1.3.0 / #60).
    /// Written today by `umux config set default-launch-mode`; nothing reads
    /// it until the TUI launcher lands, and a pre-#60 file loads as `"gui"`.
    #[serde(default = "default_launch_mode")]
    pub default_launch_mode: String,
}

/// Serde default for `default_launch_mode`: the GUI is what umux launches
/// until the TUI exists (v1.3.0).
fn default_launch_mode() -> String {
    "gui".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notifications_enabled: true,
            agent_status_enabled: true,
            session_restore_enabled: true,
            analytics_enabled: true,
            ports_tooltip_enabled: true,
            default_launch_mode: default_launch_mode(),
        }
    }
}

/// Parse settings from their JSON text form. A corrupted file yields defaults
/// rather than an error: a bad write must never block startup.
pub fn parse_settings(text: &str) -> Settings {
    parse_settings_with_status(text).0
}

/// Parse settings, also reporting whether a fallback to defaults occurred
/// (same status vocabulary as the workspace config so the caller can warn once).
pub fn parse_settings_with_status(text: &str) -> (Settings, ConfigStatus) {
    match serde_json::from_str::<Settings>(text) {
        Ok(settings) => (settings, ConfigStatus::Ok),
        Err(_) => (Settings::default(), ConfigStatus::Corrupted),
    }
}

/// User-facing warning text for a settings fallback, or `None` when silent.
/// Only `Corrupted` downgraded the user's choices without their action.
pub fn settings_fallback_warning(status: ConfigStatus) -> Option<&'static str> {
    match status {
        ConfigStatus::Corrupted => Some(
            "umux could not read its settings file (it was corrupt or unreadable) \
             and fell back to default settings.",
        ),
        ConfigStatus::Ok | ConfigStatus::Missing => None,
    }
}

/// Serialize settings to their JSON text form.
pub fn serialize_settings(settings: &Settings) -> String {
    serde_json::to_string(settings).expect("Settings is always serializable")
}

/// Thin fs wrapper: owns the settings file path and (de)serializes via the
/// pure layer. Lives next to workspaces.json in the same config directory.
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Load the settings, or defaults if the file is missing or unreadable.
    pub fn load(&self) -> Settings {
        self.load_with_status().0
    }

    /// Load the settings and report whether a fallback occurred. `Missing` =
    /// no file (normal first run, silent); `Corrupted` = the file existed but
    /// could not be parsed (caller should warn the user); `Ok` = parsed.
    pub fn load_with_status(&self) -> (Settings, ConfigStatus) {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => parse_settings_with_status(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Settings::default(), ConfigStatus::Missing)
            }
            // Unreadable for any other reason (permissions, I/O) is treated as
            // a fallback too: the user should know their settings were reset.
            Err(_) => (Settings::default(), ConfigStatus::Corrupted),
        }
    }

    /// Persist the settings: ensure the parent dir exists, serialize, write.
    pub fn save(&self, settings: &Settings) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serialize_settings(settings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-S1 (#27 — defaults: everything on). Analytics is always on and has
    //   no Settings switch (product decision, HITL follow-up); the flag only
    //   gains meaning when Phase 6 initializes Aptabase.
    #[test]
    fn defaults_enable_every_feature() {
        let d = Settings::default();
        assert!(d.notifications_enabled, "notifications default ON");
        assert!(d.agent_status_enabled, "agent status default ON");
        assert!(d.session_restore_enabled, "session restore default ON");
        assert!(d.analytics_enabled, "analytics default ON (always, no switch)");
        assert!(d.ports_tooltip_enabled, "ports tooltip default ON (#43)");
    }

    // T-S2 (#27 AC4 — settings round-trip through the pure layer):
    //   Input:  explicitly non-default Settings.
    //   Output: serialize -> parse yields identical values.
    #[test]
    fn serialize_round_trips_settings() {
        let s = Settings {
            notifications_enabled: false,
            agent_status_enabled: false,
            session_restore_enabled: false,
            analytics_enabled: false,
            ports_tooltip_enabled: false,
            default_launch_mode: "tui".into(),
        };

        let text = serialize_settings(&s);
        let back = parse_settings(&text);

        assert_eq!(back, s);
    }

    // T-S3 (#27 AC4 — corrupt file falls back to defaults, no panic):
    //   Input:  garbage text.
    //   Output: default Settings AND status Corrupted (so startup can warn).
    #[test]
    fn parse_corrupted_returns_defaults_and_corrupted() {
        let (s, status) = parse_settings_with_status("this is { not settings");

        assert_eq!(s, Settings::default());
        assert_eq!(status, ConfigStatus::Corrupted);
    }

    // T-S4 (partial file — a `{}` or hand-trimmed file loads the serde
    //   defaults, NOT an error): the per-field `default = "default_true"`
    //   wiring is what keeps a half-written file from disabling notifications.
    #[test]
    fn parse_empty_object_uses_field_defaults() {
        let (s, status) = parse_settings_with_status("{}");

        assert_eq!(s, Settings::default());
        assert_eq!(status, ConfigStatus::Ok, "valid JSON shape — no fallback");
    }

    // T-S5 (first run — settings file does not exist yet):
    //   Output: defaults AND status Missing — silent, distinct from Corrupted.
    #[test]
    fn load_missing_file_returns_defaults_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.json"));

        let (s, status) = store.load_with_status();

        assert_eq!(s, Settings::default());
        assert_eq!(status, ConfigStatus::Missing);
    }

    // T-S6 (#27 AC4 — persistence survives the filesystem):
    //   Input:  non-default settings saved to a real temp file.
    //   Output: a fresh store at the same path loads them back.
    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let s = Settings {
            notifications_enabled: false,
            agent_status_enabled: true,
            session_restore_enabled: true,
            analytics_enabled: false,
            ports_tooltip_enabled: true,
            default_launch_mode: "gui".into(),
        };

        SettingsStore::new(path.clone()).save(&s).unwrap();
        let back = SettingsStore::new(path).load();

        assert_eq!(back, s);
    }

    // T-S7 (wire format — the TS frontend reads camelCase keys):
    //   Input:  default Settings.
    //   Output: JSON carries notificationsEnabled etc., never snake_case.
    #[test]
    fn serialize_uses_camel_case_matching_frontend() {
        let text = serialize_settings(&Settings::default());

        assert!(
            text.contains("\"notificationsEnabled\""),
            "expected camelCase notificationsEnabled, got: {text}"
        );
        assert!(
            text.contains("\"agentStatusEnabled\""),
            "expected camelCase agentStatusEnabled, got: {text}"
        );
        assert!(
            text.contains("\"sessionRestoreEnabled\""),
            "expected camelCase sessionRestoreEnabled, got: {text}"
        );
        assert!(
            text.contains("\"analyticsEnabled\""),
            "expected camelCase analyticsEnabled, got: {text}"
        );
        assert!(
            text.contains("\"portsTooltipEnabled\""),
            "expected camelCase portsTooltipEnabled, got: {text}"
        );
        assert!(
            !text.contains("notifications_enabled"),
            "snake_case leaked into wire JSON: {text}"
        );
    }

    // T-S8 (#27 AC4 — only a Corrupted fallback speaks up):
    //   Mirrors fallback_warning's contract: Missing (first run) and Ok
    //   stay silent; only Corrupted produces a user-facing message.
    #[test]
    fn settings_fallback_warning_only_for_corrupted() {
        assert_eq!(settings_fallback_warning(ConfigStatus::Ok), None);
        assert_eq!(settings_fallback_warning(ConfigStatus::Missing), None);
        let msg = settings_fallback_warning(ConfigStatus::Corrupted);
        assert!(msg.is_some(), "Corrupted must yield a warning");
        assert!(msg.unwrap().to_lowercase().contains("settings"));
    }

    // T-S9 (#60 — default-launch-mode: `umux config set default-launch-mode`
    // persists the launch choice the v1.3.0 TUI launcher will read; until
    // then nothing consumes it, and the GUI stays the default).
    #[test]
    fn default_launch_mode_defaults_to_gui_and_round_trips() {
        assert_eq!(
            Settings::default().default_launch_mode,
            "gui",
            "the GUI is the launch mode until the TUI exists"
        );

        let s = Settings {
            default_launch_mode: "tui".into(),
            ..Settings::default()
        };
        let text = serialize_settings(&s);
        assert!(
            text.contains("\"defaultLaunchMode\""),
            "wire key is camelCase like the rest, got: {text}"
        );
        assert_eq!(parse_settings(&text).default_launch_mode, "tui");

        // A file written before the field existed (or hand-trimmed) loads
        // with the gui default, same as the other defaulted fields.
        let (back, status) = parse_settings_with_status("{\"notificationsEnabled\":false}");
        assert_eq!(back.default_launch_mode, "gui");
        assert_eq!(status, ConfigStatus::Ok, "valid JSON shape — no fallback");
    }
}
