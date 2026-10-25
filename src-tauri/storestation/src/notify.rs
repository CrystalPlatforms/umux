//! The daemon-side desktop notification (#114, story 145): a Core Session
//! expiry must reach the user with EVERY umux window closed — the whole
//! point of Core — so the daemon posts it itself when the deadline tick
//! settles an expiry. Expiry ONLY, never start/extend (the plan's durable
//! decision): start and extend always have a client looking at the answer.
//!
//! Same notification path as `umux notify` (cli/src/notify.rs, #62) and the
//! app's NativeNotifier, where it can run standalone: shell-outs to the
//! platform's notification tool, never through a shell (every argument
//! travels as ONE argv element). The three crates share the mechanism by
//! CONVENTION, not by a code dependency — the app crate pulls in Tauri,
//! which neither the CLI nor the daemon must. Script builders here are kept
//! line-for-line identical to the CLI's; the parity tests on both sides pin
//! that.
//!
//! Unlike the CLI (which reports failure), the daemon SWALLOWS notifier
//! failures with a stderr note — a missing backend can never break the
//! serve loop or an op answer, the app's own notifier convention.

/// The notification label — the app's, so the expiry toast reads as umux.
pub const EXPIRY_TITLE: &str = "umux";

/// The expiry notification's body, built from the end reason's vocabulary
/// (pure — the tests pin it; only `expired` reaches this path today, a stop
/// has the user's own click behind it and never notifies).
pub fn expiry_body(end_reason: &str) -> String {
    match end_reason {
        "expired" => "Core Session ended — the scheduled awake time is up. The machine can sleep again.".to_string(),
        other => format!("Core Session ended ({other}). The machine can sleep again."),
    }
}

/// Post one notification, SWALLOWING failure with a stderr note — the
/// daemon's notifier convention: a missing backend can never break the
/// serve loop or an op answer (the expiry state is settled before this
/// runs; the toast is best-effort delivery).
pub fn show_swallowed(title: &str, body: &str) {
    if let Err(message) = platform_show(title, body) {
        eprintln!("umux-storestation: the notification could not be shown: {message}");
    }
}

/// The expiry notification, end to end: the umux label, the body built from
/// the end reason, swallowed delivery.
pub fn session_expired(end_reason: &str) {
    show_swallowed(EXPIRY_TITLE, &expiry_body(end_reason));
}

/// Run a platform notification tool and translate every failure mode into a
/// clear message: a missing tool ("unavailable notification system") and a
/// tool that ran but failed both read as human sentences, with the tool's
/// own stderr appended when it has one. Identical to the CLI's run_tool.
fn run_tool(mut command: std::process::Command, tool: &str) -> Result<(), String> {
    let output = command.output().map_err(|e| {
        format!("notification failed: {tool} is not available ({e})")
    })?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            Err(format!("notification failed: {tool} exited {}", output.status))
        } else {
            Err(format!("notification failed: {tool}: {stderr}"))
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_show(summary: &str, body: &str) -> Result<(), String> {
    let mut command = std::process::Command::new("notify-send");
    command.arg("--app-name=umux").arg(summary).arg(body);
    run_tool(command, "notify-send")
}

// macOS attribution (issue #68 decision, inherited from the CLI): a bare
// non-app process cannot post through UNUserNotificationCenter — the system
// refuses unbundled processes, so the daemon keeps the osascript path with
// its "Script Editor" attribution. The BUNDLED APP shows umux attribution
// (lib.rs, BundledNotifier); revisit on demand, exactly like the CLI's.
#[cfg(target_os = "macos")]
fn platform_show(summary: &str, body: &str) -> Result<(), String> {
    let mut command = std::process::Command::new("osascript");
    command.arg("-e").arg(apple_notification_script(summary, body));
    run_tool(command, "osascript")
}

#[cfg(target_os = "windows")]
fn platform_show(summary: &str, body: &str) -> Result<(), String> {
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(windows_toast_script(summary, body));
    run_tool(command, "powershell")
}

/// Build the AppleScript `display notification` statement for summary/body
/// (macOS). IDENTICAL to the CLI's builder (cli/src/notify.rs) — AppleScript
/// string literals escape backslash and double-quote; the script travels as
/// ONE argv element (no shell), so no other quoting is needed.
#[cfg(target_os = "macos")]
fn apple_notification_script(summary: &str, body: &str) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(summary)
    )
}

/// Build the PowerShell statement that posts a ToastText02 toast with
/// summary/body (Windows). IDENTICAL to the CLI's builder (cli/src/notify.rs)
/// — text goes in through CreateTextNode (the XML DOM API escapes content
/// itself), so the ONLY quoting layer is the PowerShell single-quoted
/// literal (a literal ' doubles up).
#[cfg(target_os = "windows")]
fn windows_toast_script(summary: &str, body: &str) -> String {
    fn ps(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }
    format!(
        "[Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime]> $null; \
         $t=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
         $x=$t.GetElementsByTagName('text'); \
         $null=$x.Item(0).AppendChild($t.CreateTextNode({})); \
         $null=$x.Item(1).AppendChild($t.CreateTextNode({})); \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('umux').Show([Windows.UI.Notifications.ToastNotification]::new($t))",
        ps(summary),
        ps(body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expiry notification's text contract: the umux label, and a body
    // that says WHAT happened and WHAT CHANGES (the machine can sleep) —
    // the user reads it with every window closed, no context around it.
    #[test]
    fn the_expiry_texts_say_what_happened_and_what_changes() {
        assert_eq!(EXPIRY_TITLE, "umux");
        let body = expiry_body("expired");
        assert!(body.contains("Core Session ended"), "{body}");
        assert!(body.contains("sleep"), "{body}");
    }

    // The vocabulary escape hatch stays honest for a reason it does not
    // know (defensive — the pinned vocabulary is expired|stopped).
    #[test]
    fn an_unknown_end_reason_still_names_the_session() {
        let body = expiry_body("mystery");
        assert!(body.contains("Core Session ended"), "{body}");
        assert!(body.contains("mystery"), "{body}");
    }

    // The tool wrapper reports a missing backend as unavailable (the same
    // contract the CLI's tests pin — mirrored here so a copy drift breaks
    // THIS crate's suite too).
    #[test]
    fn a_missing_tool_reads_as_unavailable() {
        let command = std::process::Command::new("umux-no-such-notification-tool");
        let message = run_tool(command, "umux-no-such-notification-tool")
            .expect_err("missing tool must fail");
        assert!(
            message.contains("not available") && message.contains("umux-no-such-notification-tool"),
            "{message}"
        );
    }

    // macOS script builder — hostile text survives the AppleScript quoting
    // layer (the parity contract with the CLI's builder).
    #[cfg(target_os = "macos")]
    #[test]
    fn apple_script_escapes_quotes_and_backslashes() {
        let script = apple_notification_script("he said \"hi\"", "back\\slash \"q\"");
        assert!(
            script.contains("display notification \"back\\\\slash \\\"q\\\"\" with title \"he said \\\"hi\\\"\""),
            "{script}"
        );
    }

    // Windows script builder — a literal ' doubles in the PowerShell layer
    // (the parity contract with the CLI's builder).
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_script_doubles_single_quotes() {
        let script = windows_toast_script("it's done", "plain");
        assert!(script.contains("'it''s done'"), "{script}");
    }
}
