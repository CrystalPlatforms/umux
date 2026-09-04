//! `umux notify` (#62) — a desktop notification WITHOUT the
//! umux app running. One-shot: the CLI process posts the notification and
//! exits; no daemon, no socket (v1.7.0/v2.0 territory).
//!
//! Same notification path as the app's NativeNotifier (src/lib.rs), where it
//! can run standalone: shell-outs to the platform's notification tool, never
//! through a shell (every argument travels as ONE argv element). The two
//! crates share the mechanism by CONVENTION, not by a code dependency — the
//! app crate pulls in Tauri, which the CLI must not. Script builders here are
//! kept line-for-line identical to the app's; the parity tests on both sides
//! pin that.
//!
//! Unlike the app (which swallows notifier failures so a missing backend can
//! never break the terminal stream), the CLI REPORTS failure: the trait
//! returns `Result`, and an unavailable notification system becomes a clear
//! error and a non-zero exit in main.
//!
//! The `Notifier` trait is the system boundary — unit tests drive `send` with
//! recording/failing fakes and never touch the real platform tools.

/// The system boundary: whatever actually shows a desktop notification.
/// The real impl is [`PlatformNotifier`]; tests substitute fakes.
pub trait Notifier {
    /// Post one notification. `Err` carries a user-facing explanation —
    /// the tool was missing, or it ran and failed.
    fn show(&self, summary: &str, body: &str) -> Result<(), String>;
}

/// The real per-platform notifier (the only non-test construct in main).
pub struct PlatformNotifier;

/// Post one notification labeled `umux` with `text` as its body.
/// The entry point main.rs calls; tests call it with fakes.
pub fn send(backend: &dyn Notifier, text: &str) -> Result<(), String> {
    backend.show("umux", text)
}

impl Notifier for PlatformNotifier {
    fn show(&self, summary: &str, body: &str) -> Result<(), String> {
        platform_show(summary, body)
    }
}

/// Run a platform notification tool and translate every failure mode into a
/// clear message: a missing tool ("unavailable notification system") and a
/// tool that ran but failed both read as human sentences, with the tool's own
/// stderr appended when it has one.
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

// --- Platform dispatch (same tools as the app's NativeNotifier) ------------

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_show(summary: &str, body: &str) -> Result<(), String> {
    let mut command = std::process::Command::new("notify-send");
    command.arg("--app-name=umux").arg(summary).arg(body);
    run_tool(command, "notify-send")
}

// macOS attribution (issue #68, deliberate decision): a bare CLI process
// cannot post through UNUserNotificationCenter — the system refuses unbundled
// processes, so the banner keeps the "Script Editor" attribution. The
// alternative (a terminal-notifier-style helper .app) was evaluated and
// deferred: it means shipping and locating a second bundle next to every CLI
// install path (dmg / .deb / NSIS) plus a notification-permission prompt for
// an unsigned helper — heavy packaging for a cosmetic gain, since title and
// body are already correct. The BUNDLED APP does show umux attribution (see
// lib.rs, BundledNotifier); this binary keeps osascript. Revisit on demand.
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
/// (macOS). IDENTICAL to the app's apple_notification_script (src/lib.rs) —
/// AppleScript string literals escape backslash and double-quote; the script
/// travels as ONE argv element (no shell), so no other quoting is needed.
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
/// summary/body (Windows). IDENTICAL to the app's windows_toast_script
/// (src/lib.rs) — text goes in through CreateTextNode (the XML DOM API
/// escapes content itself), so the ONLY quoting layer is the PowerShell
/// single-quoted literal (a literal ' doubles up).
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

    /// A fake Notifier recording every show() call, succeeding or failing
    /// as constructed.
    struct RecordingNotifier {
        calls: std::cell::RefCell<Vec<(String, String)>>,
        result: Result<(), String>,
    }

    impl RecordingNotifier {
        /// A backend that succeeds.
        fn ok() -> Self {
            Self { calls: std::cell::RefCell::new(vec![]), result: Ok(()) }
        }

        /// A backend that always fails with `message`.
        fn failing(message: &str) -> Self {
            Self { calls: std::cell::RefCell::new(vec![]), result: Err(message.into()) }
        }
    }

    impl Notifier for RecordingNotifier {
        fn show(&self, summary: &str, body: &str) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push((summary.to_string(), body.to_string()));
            self.result.clone()
        }
    }

    // T-N1 (AC — exits 0 and invokes the platform API; text passes through
    // INTACT with spaces and quotes — the AC's passthrough requirement):
    //   Input:  send(backend, `he said "hello world" — done \ done`)
    //   Output: one show("umux", text) — the text reaches the backend
    //           byte-for-byte, label is the app label.
    #[test]
    fn send_passes_text_through_intact() {
        let backend = RecordingNotifier::ok();
        let text = "he said \"hello world\" — done \\ done";

        send(&backend, text).expect("success backend must return Ok");

        let calls = backend.calls.borrow();
        assert_eq!(calls.len(), 1, "exactly one notification per invocation");
        assert_eq!(calls[0].0, "umux", "summary is the app label");
        assert_eq!(calls[0].1, text, "body is the text, byte-for-byte");
    }

    // T-N2 (AC — unavailable notification system gives a clear error, which
    // main turns into a non-zero exit):
    //   Input:  a backend that fails (missing tool / tool error).
    //   Output: Err carrying the failure message — send never swallows it.
    #[test]
    fn failing_backend_returns_clear_error() {
        let backend = RecordingNotifier::failing(
            "notification failed: osascript is not available (No such file or directory)",
        );

        let result = send(&backend, "test");

        assert_eq!(backend.calls.borrow().len(), 1, "the attempt was made");
        let message = result.expect_err("failing backend must return Err");
        assert!(
            message.contains("osascript"),
            "error names the failing tool: {message}"
        );
    }

    // T-N3 (run_tool — a spawn failure reads as "tool not available", the
    // simulated missing-backend case from the issue):
    //   Input:  a Command whose program cannot exist.
    //   Output: Err whose message says the tool is not available.
    #[test]
    fn run_tool_reports_missing_tool_as_unavailable() {
        let command = std::process::Command::new("umux-no-such-notification-tool");
        let message = run_tool(command, "umux-no-such-notification-tool")
            .expect_err("missing tool must fail");

        assert!(
            message.contains("not available"),
            "missing tool reads as unavailable: {message}"
        );
        assert!(
            message.contains("umux-no-such-notification-tool"),
            "error names the tool: {message}"
        );
    }

    // T-N4 (run_tool — a tool that ran but failed surfaces its stderr):
    //   Input:  `false`-like tool: exits non-zero with a message on stderr.
    //   Output: Err carrying the tool's stderr, so "why" is visible.
    #[cfg(unix)]
    #[test]
    fn run_tool_surfaces_stderr_of_failed_tool() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "echo backend says no >&2; exit 3"]);
        let message = run_tool(command, "faketool").expect_err("failed tool must fail");

        assert!(
            message.contains("backend says no"),
            "error carries the tool's stderr: {message}"
        );
    }

    // T-N5 (macOS script builder — hostile text survives the AppleScript
    // quoting layer, same parity contract as the app's builder):
    #[cfg(target_os = "macos")]
    #[test]
    fn apple_script_escapes_quotes_and_backslashes() {
        let script = apple_notification_script("he said \"hi\"", "back\\slash \"q\"");

        assert!(
            script.contains("display notification \"back\\\\slash \\\"q\\\"\" with title \"he said \\\"hi\\\"\""),
            "quotes and backslashes escaped: {script}"
        );
    }

    // T-N6 (Windows script builder — a literal ' doubles in the
    // PowerShell layer, same parity contract as the app's builder):
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_script_doubles_single_quotes() {
        let script = windows_toast_script("it's done", "plain");

        assert!(
            script.contains("'it''s done'"),
            "single quote doubled: {script}"
        );
    }
}
