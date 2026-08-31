pub mod analytics;
pub mod cmux_import;
pub mod git_branch;
pub mod listening_ports;
pub mod notification_service;
pub mod osc_parser;
pub mod pty_service;
pub mod ssh_manager;
pub mod updater_probe;

// The workspace/settings store (model, WorkspaceStore, SettingsStore, config
// paths) is shared library code since #58: StoreCore (`store_core` crate)
// owns it so the upcoming `umux` CLI can write the same files through the
// same implementation. The app just consumes it here.
use store_core::paths::{config_dir, config_path, legacy_config_dir, migrate_legacy_config, settings_path};
use store_core::settings_store::{settings_fallback_warning, Settings, SettingsStore};
use store_core::workspace_store::{fallback_warning, Group, Workspace, WorkspaceData, WorkspaceStore};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use notification_service::{NotificationService, Notifier, PanelOrigin};
use osc_parser::OscParser;
use pty_service::{PtyHandle, PtyService};
use ssh_manager::{parse_ssh_target, SshHandle, SshManager};

/// The app-wide notification mute flag. One instance is created in `run()` and
/// shared (via Arc) with every panel's NotificationService, so a single toggle
/// silences notifications across the whole app. `false` = audible (default).
type MuteFlag = Arc<AtomicBool>;

/// One chunk of PTY output, ferried to the renderer over the `pty_output` event.
/// `data` is raw bytes serialized as a JSON array of numbers; the frontend
/// rebuilds a `Uint8Array` from it.
#[derive(Serialize, Clone)]
struct PtyOutputPayload {
    id: u32,
    data: Vec<u8>,
}

/// Notifies the renderer that an SSH session ended. `error` is a clear,
/// user-readable connection-failure message when ssh exited with a failure code
/// (255), or `None` for a clean / remote-command exit — so the frontend can show
/// a diagnostic for a dead connection instead of leaving a blank, hung panel
/// (Phase 16 / Issue #17, AC1 + AC2).
#[derive(Serialize, Clone)]
struct SshExitPayload {
    id: u32,
    error: Option<String>,
}

/// Per-panel completion signal for the renderer (v0.2 Phase 2 / #26). Emitted
/// when the OSC parser surfaces completion event(s) in a PTY/SSH output chunk,
/// so the frontend's per-panel status machine can flip that panel to
/// needs-attention. Carries only the panel id — the desktop notification (with
/// its title/body) is fired separately and unchanged; the two channels are
/// independent (muting notifications must NOT mute the status dot).
#[derive(Serialize, Clone)]
struct PanelSignalPayload {
    id: u32,
}

#[tauri::command]
fn pty_open(
    app: AppHandle,
    state: State<'_, Mutex<PtyService>>,
    mute: State<'_, MuteFlag>,
    shell: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    label: Option<String>,
) -> Result<u32, String> {
    // Default to the user's $SHELL, falling back to /bin/sh; an explicit
    // `shell` override (from a future WorkspaceStore config) wins.
    let shell = resolve_shell(shell.as_deref());
    // A cwd from the session snapshot (v0.2 Phase 5 / #29) when it still
    // exists; otherwise the app's current dir (v0.1 behavior).
    let cwd = resolve_cwd(cwd.as_deref());

    // Open the PTY at the renderer's measured size from the very first moment.
    // If the frontend hasn't measured yet (or sends nothing), fall back to
    // 80x24 — but the normal path forwards the real cols/rows so the shell's
    // idea of the line width matches xterm immediately. Without this the PTY
    // opens at 80x24 and the first `fit()` doesn't change xterm's size (so
    // `onResize` never fires, `pty_resize` never lands), leaving the shell and
    // xterm disagreed about width — long lines overwrite the prompt and
    // backspace scrambles the line.
    let cols = cols.unwrap_or(80);
    let rows = rows.unwrap_or(24);

    let (handle, rx) = {
        let mut svc = state.lock().map_err(|e| e.to_string())?;
        svc.open(&shell, cwd, cols, rows).map_err(|e| e.to_string())?
    };
    let id = handle.id;

    // Drain the PTY's output channel on its own thread and forward each chunk
    // to the renderer. The channel disconnects (loop ends) when the PTY closes.
    //
    // Each chunk is first fed through the OSC parser: recognized completion
    // sequences are turned into a desktop notification (labeled with this
    // panel's origin), and the remaining bytes — everything the terminal should
    // actually see — are emitted as `pty_output`. Because the parser only
    // extracts events and passes other bytes through untouched, normal terminal
    // output is byte-identical whether or not a notification sequence appears.
    let emit_app = app.clone();
    let origin = PanelOrigin {
        workspace: None,
        panel: label,
    };
    // Clone the app-wide mute flag into this panel's service so the
    // `set_notifications_muted` command (which flips the shared flag) is observed
    // live by every panel's notification thread.
    let mute_flag = Arc::clone(&mute);
    std::thread::spawn(move || {
        let mut parser = OscParser::new();
        let service = NotificationService::with_mute(
            platform_notifier(&emit_app),
            Some("umux".to_string()),
            mute_flag,
        );
        while let Ok(bytes) = rx.recv() {
            let (passthrough, events) = process_pty_chunk(&mut parser, &service, &origin, &bytes);
            if !events.is_empty() {
                // Completion signal first, then the surviving output bytes: the
                // frontend status machine's grace window expects a TUI's
                // trailing redraw to land right AFTER its completion signal.
                let _ = emit_app.emit("pty_completion", PanelSignalPayload { id });
            }
            if !passthrough.is_empty() {
                let _ = emit_app.emit("pty_output", PtyOutputPayload { id, data: passthrough });
            }
        }
    });

    Ok(id)
}

#[tauri::command]
fn pty_write(state: State<'_, Mutex<PtyService>>, id: u32, data: String) -> Result<(), String> {
    let mut svc = state.lock().map_err(|e| e.to_string())?;
    svc.write(&PtyHandle { id }, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
fn pty_resize(state: State<'_, Mutex<PtyService>>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let mut svc = state.lock().map_err(|e| e.to_string())?;
    svc.resize(&PtyHandle { id }, cols, rows).map_err(|e| e.to_string())
}

#[tauri::command]
fn pty_close(state: State<'_, Mutex<PtyService>>, id: u32) -> Result<(), String> {
    let mut svc = state.lock().map_err(|e| e.to_string())?;
    svc.close(&PtyHandle { id });
    Ok(())
}

/// v0.2 Phase 4 / #28 — is a live process (not the idle shell) running in
/// this local panel? Every close path (X button, Ctrl+Shift+W, workspace
/// close) asks this BEFORE tearing a panel down; `true` means the frontend
/// must confirm with the user first. SSH panels have no equivalent: the local
/// `ssh` client is always the foreground group while connected, and the
/// remote side is opaque (OSC-only, no polling), so they close without asking.
#[tauri::command]
fn pty_is_busy(state: State<'_, Mutex<PtyService>>, id: u32) -> Result<bool, String> {
    let mut svc = state.lock().map_err(|e| e.to_string())?;
    Ok(svc.is_busy(&PtyHandle { id }))
}

// --- Session snapshot support (v0.2 Phase 5 / #29) ---------------------------
//
// The frontend owns the panelId(leaf)↔ptyId mapping (each TerminalSurface
// reports its handle via onOpened); the backend owns the OS. One command
// reads every live local shell's cwd in a single invoke so the frontend can
// merge them into workspaces.json before persisting a layout change or
// quitting. Remote panels are not queried — the remote cwd is not visible
// locally (OSC-only policy), so their snapshot keeps just the ssh target.

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CwdQuery {
    panel_id: String,
    pty_id: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CwdAnswer {
    panel_id: String,
    cwd: Option<String>,
}

#[tauri::command]
fn panel_cwds(
    state: State<'_, Mutex<PtyService>>,
    panels: Vec<CwdQuery>,
) -> Result<Vec<CwdAnswer>, String> {
    let svc = state.lock().map_err(|e| e.to_string())?;
    Ok(panels
        .into_iter()
        .map(|q| {
            let cwd = svc
                .cwd(&PtyHandle { id: q.pty_id })
                .map(|p| p.to_string_lossy().into_owned());
            CwdAnswer {
                panel_id: q.panel_id,
                cwd,
            }
        })
        .collect())
}

// --- Agent-status presence (model v2, HITL 2026-08-25) ------------------------
//
// The renderer polls this every couple of seconds: one invoke returns the
// foreground program NAME per local panel, which the frontend matches
// against known AI-CLI names (src/aiCli.ts) to drive the "opened and
// waiting -> needs-attention / exited -> idle" half of the status model.
// Reuses CwdQuery's wire shape ({panelId, ptyId}); remote (SSH) panels are
// not polled — the local foreground program is always the ssh client, which
// says nothing about the remote side (OSC-only there, as ever).

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PanelProcessAnswer {
    panel_id: String,
    process: Option<String>,
}

#[tauri::command]
fn panel_processes(
    state: State<'_, Mutex<PtyService>>,
    panels: Vec<CwdQuery>,
) -> Result<Vec<PanelProcessAnswer>, String> {
    let mut svc = state.lock().map_err(|e| e.to_string())?;
    Ok(panels
        .into_iter()
        .map(|q| {
            let process = svc.foreground_process_name(&PtyHandle { id: q.pty_id });
            PanelProcessAnswer { panel_id: q.panel_id, process }
        })
        .collect())
}

// --- Sidebar tab metadata: git branch (v1.0 Phase 14 / #41) -------------------
//
// One read-only batch query answers the branch label for every tab row at
// once: input is the list of DIRECTORIES to resolve (each tab's focused
// panel's starting workingDirectory, computed by the frontend), output echoes
// each directory back with its resolved label — a branch name or a short
// detached-HEAD sha, or None when no repository is present (the UI then shows
// nothing; this command can never fail). Parsing happens straight from `.git`
// in git_branch::resolve_branch — no `git` binary is spawned (plan decision,
// 2026-08-26). Refresh is PULL-ONLY on frontend UI events (tab set change /
// focus change / configured directory change) — there is deliberately no
// timer and no filesystem watching.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitBranchAnswer {
    dir: String,
    branch: Option<String>,
}

#[tauri::command]
fn git_branches(dirs: Vec<String>) -> Vec<GitBranchAnswer> {
    dirs.into_iter()
        .map(|dir| GitBranchAnswer {
            branch: git_branch::resolve_branch(std::path::Path::new(&dir)),
            dir,
        })
        .collect()
}

// --- Sidebar tab metadata: listening ports (v1.0 Phase 15 / #42) --------------
//
// One batch query answers "which TCP ports does this TAB listen on?" for
// every hovered tab at once: input is the tab id plus its LOCAL panel PTY
// handles (the frontend knows the local/ssh split — SSH panels are skipped,
// same reasoning as panel_processes: the local process is always just the
// ssh client and says nothing about remote listeners). The backend maps each
// handle to its shell pid, snapshots the OS socket tables + process tree ONCE
// per invoke, and matches each tab's trees against that one snapshot (so a
// multi-tab hover can never mix two different moments). Refresh is PULL-ONLY
// on tab hover — no timer, no background work while nothing is hovered.
// Ports are ascending + deduplicated by number; an empty list means the UI's
// explicit "No listening ports" state. Total failure policy: a vanished shell
// or unreadable socket table yields empty ports, never an invoke error.

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TabPortsQuery {
    tab_id: String,
    pty_ids: Vec<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TabPortsAnswer {
    tab_id: String,
    ports: Vec<u16>,
}

#[tauri::command]
fn tab_ports(
    state: State<'_, Mutex<PtyService>>,
    tabs: Vec<TabPortsQuery>,
) -> Result<Vec<TabPortsAnswer>, String> {
    let svc = state.lock().map_err(|e| e.to_string())?;
    let listeners = listening_ports::listening_sockets();
    let edges = listening_ports::parent_edges();
    Ok(tabs
        .into_iter()
        .map(|tab| {
            let roots: Vec<u32> = tab
                .pty_ids
                .iter()
                .filter_map(|id| svc.child_pid(&PtyHandle { id: *id }))
                .collect();
            TabPortsAnswer {
                ports: listening_ports::aggregate_ports(&listeners, &edges, &roots),
                tab_id: tab.tab_id,
            }
        })
        .collect())
}

// --- SSH panels (Phase 16 / Issue #17) ----------------------------------------
//
// Remote panels live in a separate `SshManager` (which owns its own PtyService),
// so they use a parallel command family — `ssh_open/write/resize/close` — and a
// separate `ssh_output` event. This is deliberate: the SshManager's PtyService
// mints its own ids from 0, so sharing the local `pty_output` channel would
// collide (local id 0 vs remote id 0). A parallel family keeps the two id
// spaces disjoint while giving remote panels the SAME shape (open → id; output
// event filtered by id; write/resize/close by id) so the frontend can treat
// them uniformly (AC3 parity).
//
// Synchronous errors (bad target string, empty host/user) come back as a
// rejected invoke whose string is already the friendly message produced by
// `parse_ssh_target` / `SshTarget::validate` (AC1). Async connection failures
// (ssh exits 255) are surfaced later via the `ssh_exit` event (Plaster 5).

#[tauri::command]
fn ssh_open(
    app: AppHandle,
    state: State<'_, Mutex<SshManager>>,
    mute: State<'_, MuteFlag>,
    target: String,
    cols: Option<u16>,
    rows: Option<u16>,
    label: Option<String>,
) -> Result<u32, String> {
    // v1.0 Phase 9 / #33: SSH panels are Linux/macOS-only until v2.0. On
    // Windows the command answers with a clear, user-readable error instead
    // of spawning a session that may misbehave — the panel surfaces the
    // message (same path as any connection failure), never a hung blank
    // surface. There is no SSH entry point in the UI to hide: targets come
    // from hand-edited configs only.
    if cfg!(windows) {
        return Err("SSH panels are not supported on Windows yet — planned for v2.0. Remove the panel's sshTarget to make it local.".to_string());
    }
    let parsed = parse_ssh_target(&target).map_err(|e| e.to_string())?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let cols = cols.unwrap_or(80);
    let rows = rows.unwrap_or(24);

    let (handle, rx) = {
        let mut mgr = state.lock().map_err(|e| e.to_string())?;
        mgr.open(&parsed, cwd, cols, rows).map_err(|e| e.to_string())?
    };
    let id = handle.id();

    // Same output-stream wiring as a local panel: feed each chunk through the
    // OSC parser (completion sequences → notification) and emit the surviving
    // bytes as `ssh_output`. Because remote and local panels share this exact
    // treatment, a long-running task finishing over SSH fires the same desktop
    // notification as a local one (AC3 parity for the notification path too).
    let emit_app = app.clone();
    let origin = PanelOrigin {
        workspace: None,
        panel: label,
    };
    let mute_flag = Arc::clone(&mute);
    let host = parsed.host.clone();
    std::thread::spawn(move || {
        let mut parser = OscParser::new();
        let service = NotificationService::with_mute(
            platform_notifier(&emit_app),
            Some("umux".to_string()),
            mute_flag,
        );
        while let Ok(bytes) = rx.recv() {
            let (passthrough, events) = process_pty_chunk(&mut parser, &service, &origin, &bytes);
            if !events.is_empty() {
                // Same per-panel completion signal as local panels (see the
                // pty_open thread) — remote status parity (#26).
                let _ = emit_app.emit("ssh_completion", PanelSignalPayload { id });
            }
            if !passthrough.is_empty() {
                let _ = emit_app.emit("ssh_output", PtyOutputPayload { id, data: passthrough });
            }
        }

        // The output stream ended (ssh exited). Poll its exit code and, if it's
        // a connection failure (255 / signal), emit a `ssh_exit` event carrying
        // a clear message so the frontend can show a diagnostic instead of a
        // dead, blank panel. A clean or remote-command exit yields error=None.
        let error = poll_ssh_exit(&emit_app, &handle, &host);
        let _ = emit_app.emit("ssh_exit", SshExitPayload { id, error });
    });

    Ok(id)
}

#[tauri::command]
fn ssh_write(state: State<'_, Mutex<SshManager>>, id: u32, data: String) -> Result<(), String> {
    let mut mgr = state.lock().map_err(|e| e.to_string())?;
    mgr.write(&SshHandle::from_pty_id(id), data.as_bytes())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn ssh_resize(state: State<'_, Mutex<SshManager>>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let mut mgr = state.lock().map_err(|e| e.to_string())?;
    mgr.resize(&SshHandle::from_pty_id(id), cols, rows)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn ssh_close(state: State<'_, Mutex<SshManager>>, id: u32) -> Result<(), String> {
    let mut mgr = state.lock().map_err(|e| e.to_string())?;
    mgr.close(&SshHandle::from_pty_id(id));
    Ok(())
}

/// Real `Notifier` — a thin adapter on the OS boundary. The spawn itself is
/// not unit-tested; its behavior is verified manually by Adam (Linux) and now
/// on macOS too. A failed spawn is logged but otherwise swallowed so a missing
/// notifier can never break the terminal stream.
///
/// Platform split (v0.2 Phase 2 / #26 HITL: "the notification never arrives"
/// on macOS — there is no notify-send there):
///  - Linux: unchanged v0.1 path — libnotify via the `notify-send` CLI. We
///    shell out rather than use the `notify-rust` D-Bus API because GNOME
///    silently drops banners from processes without a `.desktop` file (the
///    case during `tauri dev`), while `notify-send` is always shown.
///  - macOS: AppleScript `display notification` via the `osascript` CLI. This
///    works both for the unbundled `tauri dev` binary and the unsigned .app
///    bundle (UNUserNotificationCenter-based plugins can't post from an
///    unbundled dev process, and osascript needs no permission dance), which
///    keeps the zero-cost policy intact.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct NativeNotifier;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Notifier for NativeNotifier {
    fn show(&self, summary: &str, body: &str) {
        let result = std::process::Command::new("notify-send")
            .arg("--app-name=umux")
            .arg(summary)
            .arg(body)
            .output();
        match &result {
            Ok(out) if out.status.success() => {
                log::info!("[notify] dispatched: summary={summary:?} body={body:?}")
            }
            Ok(out) => log::error!(
                "[notify] notify-send exited {}: stderr={}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(e) => log::error!("[notify] failed to spawn notify-send: {e}"),
        }
    }
}

// macOS (issue #68): every notification used to go through `osascript`, so
// macOS attributed the banner to **Script Editor** — the AppleScript runtime —
// no matter who sent it. Two paths now live behind the same `Notifier` trait:
//  - Bundled app (running from umux.app/…/MacOS/umux): post through
//    UNUserNotificationCenter via tauri-plugin-notification, so the banner
//    carries the umux name and icon. UNUserNotificationCenter refuses to post
//    from an UNBUNDLED process — that is exactly why the split exists. On any
//    error (permission denied/revoked, runtime refusal) we fall back to
//    osascript, so a notification is never lost.
//  - Unbundled binary (`tauri dev`): straight to osascript — the pre-#68
//    behavior, which keeps the zero-cost policy (no codesign requirement).
// Linux and Windows keep their single `NativeNotifier` path, unchanged.
#[cfg(target_os = "macos")]
fn is_bundled_app(exe: &std::path::Path) -> bool {
    exe.to_string_lossy().contains(".app/Contents/MacOS")
}

#[cfg(target_os = "macos")]
struct OsascriptNotifier;

#[cfg(target_os = "macos")]
impl Notifier for OsascriptNotifier {
    fn show(&self, summary: &str, body: &str) {
        let script = apple_notification_script(summary, body);
        let result = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output();
        match &result {
            Ok(out) if out.status.success() => {
                log::info!("[notify] dispatched: summary={summary:?} body={body:?}")
            }
            Ok(out) => log::error!(
                "[notify] osascript exited {}: stderr={}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(e) => log::error!("[notify] failed to spawn osascript: {e}"),
        }
    }
}

/// Bundled-app notifier: UNUserNotificationCenter through the notification
/// plugin (needs the AppHandle, hence the field). Not unit-tested — its
/// behavior is Adam's HITL check on macOS (umux attribution in the banner).
#[cfg(target_os = "macos")]
struct BundledNotifier {
    app: AppHandle,
}

#[cfg(target_os = "macos")]
impl Notifier for BundledNotifier {
    fn show(&self, summary: &str, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        let result = self
            .app
            .notification()
            .builder()
            .title(summary)
            .body(body)
            .show();
        match result {
            Ok(()) => log::info!("[notify] dispatched (UNUserNotificationCenter): summary={summary:?} body={body:?}"),
            Err(e) => {
                // Fallback, not failure: attribution is cosmetic, delivery is
                // the contract. osascript always works (it is what `tauri dev`
                // uses), so the notification still arrives.
                log::warn!(
                    "[notify] UNUserNotificationCenter failed ({e}); falling back to osascript"
                );
                OsascriptNotifier.show(summary, body);
            }
        }
    }
}

/// Build the AppleScript `display notification` statement for summary/body
/// (macOS notifier). AppleScript string literals escape backslash and
/// double-quote; the script travels as ONE argv element (no shell), so no
/// other quoting is needed. Pure — unit-tested with hostile input.
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

/// The Windows notifier (v1.0 Phase 9 / #33): a native toast through the
/// WinRT toast API driven by PowerShell — zero extra crates, the same
/// shell-out pattern as notify-send (Linux) and osascript (macOS), keeping
/// the zero-cost policy. ToastText02 renders one bold heading (summary) and
/// one body line — the same two-line shape the other platforms show.
#[cfg(target_os = "windows")]
struct NativeNotifier;

#[cfg(target_os = "windows")]
impl Notifier for NativeNotifier {
    fn show(&self, summary: &str, body: &str) {
        let script = windows_toast_script(summary, body);
        let result = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output();
        match &result {
            Ok(out) if out.status.success() => {
                log::info!("[notify] dispatched: summary={summary:?} body={body:?}")
            }
            Ok(out) => log::error!(
                "[notify] powershell exited {}: stderr={}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(e) => log::error!("[notify] failed to spawn powershell.exe: {e}"),
        }
    }
}

/// Build the PowerShell statement that posts a ToastText02 toast with
/// summary/body (Windows notifier). Text goes in through CreateTextNode —
/// the XML DOM API escapes content itself — so the ONLY quoting layer is the
/// PowerShell single-quoted literal (a literal ' doubles up). The script
/// travels as ONE argv element (`-Command <script>`), never through a
/// shell. Pure — same testability contract as apple_notification_script.
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

/// The pre-flight the frontend runs BEFORE touching the updater plugin
/// (issue #66). One invoke answers, in order: is the signer key configured
/// at all, is GitHub reachable, does a release feed (latest.json) exist.
/// Only "ok" makes the frontend call the plugin — so "offline" can no longer
/// be shown while the real situation is simply "no feed published yet".
#[tauri::command]
async fn updater_status(app: AppHandle) -> updater_probe::UpdaterStatus {
    if !updater_probe::pubkey_configured(&app) {
        return updater_probe::UpdaterStatus::Unconfigured;
    }
    updater_probe::probe_latest_json(&app).await
}

/// Pick the notifier for a panel's output thread (issue #68 on macOS; other
/// platforms keep their single path). The platform swap stays behind the
/// `Notifier` trait — call sites and the service never care which backend is
/// live, and tests exercise each path through that same boundary.
fn platform_notifier(app: &AppHandle) -> Box<dyn Notifier + Send> {
    #[cfg(target_os = "macos")]
    {
        let bundled = std::env::current_exe()
            .map(|exe| is_bundled_app(&exe))
            .unwrap_or(false);
        if bundled {
            return Box::new(BundledNotifier { app: app.clone() });
        }
        Box::new(OsascriptNotifier)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Box::new(NativeNotifier)
    }
}

/// Payload for the `config_fallback` event (Phase 18 / Issue #19, AC3).
/// Emitted when the config file was corrupt/unreadable and umux fell back to
/// default workspaces, so the renderer can show the user a clear message
/// instead of silently downgrading their setup.
#[derive(Serialize, Clone)]
struct ConfigFallbackPayload {
    message: &'static str,
}

#[tauri::command]
fn load_workspaces(
    app: AppHandle,
    state: State<'_, WorkspaceStore>,
) -> WorkspaceData {
    let (data, status) = state.load_with_status();
    // AC3: a corrupted config must not be a silent downgrade. Surface a clear
    // message both in the backend log and as an event the frontend can render.
    // Missing is a normal first run -> silent; Ok is success.
    if let Some(message) = fallback_warning(status) {
        log::warn!("[config] fallback: {message}");
        let _ = app.emit("config_fallback", ConfigFallbackPayload { message });
    }
    data
}

#[tauri::command]
fn save_workspaces(
    state: State<'_, WorkspaceStore>,
    workspaces: Vec<Workspace>,
    groups: Vec<Group>,
    order: Vec<String>,
) -> Result<(), String> {
    // The params are named after the keys the frontend sends
    // (`invoke('save_workspaces', { workspaces, groups, order })`) — Tauri
    // maps invoke args by name, and an earlier `data: WorkspaceData`
    // signature silently rejected every save because `workspaces` never
    // reached `data`. Same rule, three keys since the tree (#48): every
    // invoke key must have its matching parameter here.
    state
        .save(&WorkspaceData {
            workspaces,
            groups,
            order,
        })
        .map_err(|e| e.to_string())
}

/// Toggle the app-wide notification mute. Returns the new state so the frontend
/// can update its indicator from the source of truth (the flag is shared with
/// every panel's notification thread, so this is the only way to flip it).
#[tauri::command]
fn set_notifications_muted(muted: bool, state: State<'_, MuteFlag>) -> bool {
    state.store(muted, Ordering::SeqCst);
    muted
}

/// Read the current mute state. The frontend calls this on mount to seed its
/// indicator (the flag lives in the backend; the UI must not assume a default).
#[tauri::command]
fn notifications_muted(state: State<'_, MuteFlag>) -> bool {
    state.load(Ordering::SeqCst)
}

/// Load the persisted feature toggles (v0.2 Phase 3 / #27). A corrupted file
/// falls back to defaults AND emits the same `config_fallback` warning the
/// workspace config uses, so the downgrade is surfaced, not silent.
#[tauri::command]
fn load_settings(app: AppHandle, state: State<'_, SettingsStore>) -> Settings {
    let (settings, status) = state.load_with_status();
    if let Some(message) = settings_fallback_warning(status) {
        log::warn!("[config] settings fallback: {message}");
        let _ = app.emit("config_fallback", ConfigFallbackPayload { message });
    }
    settings
}

/// Persist the feature toggles. The param is named `settings` to match the
/// invoke key the frontend sends (`invoke('save_settings', { settings })`) —
/// Tauri maps arguments by name (see the save_workspaces comment).
#[tauri::command]
fn save_settings(state: State<'_, SettingsStore>, settings: Settings) -> Result<(), String> {
    state.save(&settings).map_err(|e| e.to_string())
}

/// Open settings.json with the platform's default handler (Settings footnote
/// link): the file the toggles persist to, revealed in the user's own editor.
/// Fire-and-forget spawn — a GUI editor may stay open for hours, so we never
/// wait on it; a failed SPAWN (opener binary missing) is the only error the
/// frontend sees, and it logs it without breaking the dialog.
#[tauri::command]
fn open_settings_file() -> Result<(), String> {
    let path = settings_path();
    #[allow(unused_mut)]
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(&path);
        c
    } else if cfg!(target_os = "windows") {
        // One plain argument into explorer — no shell, no quoting pitfalls;
        // explorer opens the file with its default association.
        let mut c = std::process::Command::new("explorer");
        c.arg(&path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&path);
        c
    };
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("open settings file failed: {e}"))
}

/// Poll the ssh child's exit code (briefly, after its output stream ended) and
/// translate a connection-failure exit into a clear, user-readable message.
/// Returns `None` for a clean exit or a remote-command exit (the connection
/// itself was fine). Used by the `ssh_open` reader thread to emit `ssh_exit`.
///
/// This is the testable core of the async-error path; it composes the pure
/// `friendly_ssh_exit` translator with one bounded poll of the live handle.
fn poll_ssh_exit(app: &AppHandle, handle: &ssh_manager::SshHandle, host: &str) -> Option<String> {
    let state = app.state::<Mutex<SshManager>>();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let code = loop {
        match state.lock().ok()?.child_exit_code(handle) {
            Ok(Some(c)) => break c,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    };
    ssh_manager::friendly_ssh_exit(code, host)
}

/// Run one chunk of PTY output through the OSC parser and dispatch any
/// completion events it surfaces. Returns the bytes the terminal should still
/// see (everything that wasn't a recognized notification sequence) — those are
/// byte-identical to the input for non-OSC bytes, satisfying "normal terminal
/// output is unaffected by the parser being active".
///
/// The parser is held by the caller (one per PTY) so sequences split across
/// chunk boundaries are recognized across calls. This is the testable core of
/// the PTY-output wiring; `pty_open` plugs a real `Notifier` into it.
fn process_pty_chunk(
    parser: &mut OscParser,
    service: &NotificationService,
    origin: &PanelOrigin,
    bytes: &[u8],
) -> (Vec<u8>, Vec<osc_parser::NotificationEvent>) {
    let osc_parser::PushResult { passthrough, events } = parser.push(bytes);
    for event in &events {
        service.notify(event, origin);
    }
    // The events travel back to the caller so the reader thread can ALSO emit
    // the per-panel completion signal (`pty_completion` / `ssh_completion`).
    // The notify above (desktop notification) is unchanged — v0.1 behavior.
    (passthrough, events)
}

/// Decide which shell binary to launch for a panel.
///
/// An explicit override (from a future WorkspaceStore config) wins; otherwise
/// the default is per-OS (v1.0 Phase 9 / #33): Windows PowerShell on Windows
/// (the in-box Windows PowerShell 5.1 — always present on Windows 10+, which
/// pwsh is not), the user's `$SHELL` then `/bin/sh` on Linux/macOS.
pub fn resolve_shell(shell: Option<&str>) -> String {
    if let Some(s) = shell {
        return s.to_string();
    }
    if cfg!(windows) {
        return "powershell.exe".to_string();
    }
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Decide the working directory a new shell starts in (v0.2 Phase 5 / #29).
/// A cwd saved in the session snapshot wins when it still exists and is a
/// directory; anything else (none, empty, deleted, a plain file) falls back
/// to the app's current dir — exactly v0.1 behavior — so a stale or hostile
/// snapshot value can never break panel spawn.
pub fn resolve_cwd(cwd: Option<&str>) -> PathBuf {
    match cwd {
        Some(dir) if !dir.is_empty() && std::path::Path::new(dir).is_dir() => PathBuf::from(dir),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // tauri-plugin-aptabase 1.0.0 starts its flush loop with a bare
    // `tokio::spawn` from its setup hook, which needs an ambient Tokio
    // context on the main thread — without this guard the app PANICS at
    // startup before any window appears ("there is no reactor running").
    // Tauri keeps a global Tokio runtime but never *enters* it on the main
    // thread, so enter it here for the whole run(); the plugin's task then
    // lives on the same runtime as everything else. The guard drops when
    // run() returns (app exit).
    let rt_handle = match tauri::async_runtime::handle() {
        tauri::async_runtime::RuntimeHandle::Tokio(h) => h,
    };
    let _rt_guard = rt_handle.enter();

    // v1.0 Phase 8 / #32: relocate config from the legacy location BEFORE
    // anything reads it (the settings seed below is the first reader).
    // No-op on Linux (same directory) and on a fresh Mac (nothing to move).
    migrate_legacy_config(&config_dir(), &legacy_config_dir());

    // Seed the app-wide notification flag from the persisted settings (v0.2
    // Phase 3 / #27): the AtomicBool is the runtime gate every panel's
    // notification thread reads; settings.json is its persisted form, so a
    // disabled-notifications toggle survives a restart.
    let settings_store = SettingsStore::new(settings_path());
    let initial_settings = settings_store.load();
    let mute: MuteFlag = Arc::new(AtomicBool::new(!initial_settings.notifications_enabled));
    // v0.2 Phase 6 / #30: the analytics gate seeds from the same persisted
    // settings, BEFORE the Tauri builder runs — when false, the plugin below
    // is never registered, so the SDK makes no network call at all.
    let analytics_enabled = initial_settings.analytics_enabled;

    let builder = tauri::Builder::default()
        .manage(Mutex::new(PtyService::new()))
        .manage(Mutex::new(SshManager::new()))
        .manage(WorkspaceStore::new(config_path()))
        .manage(settings_store)
        .manage(mute)
        .invoke_handler(tauri::generate_handler![
            pty_open,
            pty_write,
            pty_resize,
            pty_close,
            pty_is_busy,
            panel_cwds,
            panel_processes,
            git_branches,
            tab_ports,
            ssh_open,
            ssh_write,
            ssh_resize,
            ssh_close,
            load_workspaces,
            save_workspaces,
            set_notifications_muted,
            notifications_muted,
            load_settings,
            save_settings,
            open_settings_file,
            updater_status,
            cmux_import::read_cmux_import_sources,
        ]);

    // #30 AC2: "when off, the SDK is never initialized (no network call)" —
    // this registration is the only place the plugin comes to life, and it
    // happens only on an enabled startup decision. (The plugin flushes its
    // queue on app exit by itself, so no exit hook is needed here.)
    let builder = if analytics_enabled {
        builder.plugin(analytics::aptabase_plugin())
    } else {
        builder
    };

    // Issue #66: in-app updates. The updater plugin serves `check()` to the
    // frontend (GitHub Releases latest.json is the only endpoint — zero-cost
    // policy); the process plugin provides `relaunch()` so "download + apply +
    // restart" is one click. Signature verification is enforced by the plugin
    // itself against `plugins.updater.pubkey` — an unsigned or tampered bundle
    // is rejected before anything is written to disk.
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Issue #68: the bundled macOS notifier posts through this plugin
        // (UNUserNotificationCenter); on Linux/Windows the plugin is idle —
        // their notifiers shell out as before, behavior unchanged.
        .plugin(tauri_plugin_notification::init());

    builder
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            // v0.2 Phase 6 / #30 — the ONLY event umux reports: one
            // aggregate app_open, so Aptabase can count installs/active
            // users. Guarded by the same decision that registered the
            // plugin (a true flag always implies the SDK exists above).
            if analytics_enabled {
                use tauri_plugin_aptabase::EventTracker;
                if let Err(e) = app.handle().track_event(analytics::APP_OPEN_EVENT, None) {
                    log::warn!("[analytics] track_event failed: {e}");
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    // T1 (AC3 — non-default shell can be used when configured):
    //   Input:  Some("/bin/dash")
    //   Output: "/bin/dash" verbatim — an explicit override wins, untouched.
    #[test]
    fn resolve_shell_with_override_returns_override() {
        assert_eq!(resolve_shell(Some("/bin/dash")), "/bin/dash");
    }

    // T2 (AC1 — default to the user's $SHELL):
    //   Input:  None
    //   Output: whatever $SHELL currently is in the process env (or "/bin/sh"
    //           if unset — that fallback branch is intentionally NOT tested
    //           here, since exercising it would require mutating the global
    //           SHELL var, unsafe under cargo's parallel test threads).
    #[test]
    fn resolve_shell_none_uses_shell_env() {
        let expected = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        assert_eq!(resolve_shell(None), expected);
    }

    // --- v0.2 Phase 5 / #29: restore cwd resolution ---------------------------

    // T-C1 (AC2 — a restored panel re-spawns in its saved cwd):
    //   Input:  Some(<a directory that exists>)
    //   Output: that directory, verbatim.
    #[test]
    fn resolve_cwd_valid_directory_wins() {
        let dir = std::env::temp_dir();
        assert_eq!(resolve_cwd(Some(dir.to_str().unwrap())), dir);
    }

    // T-C2 (AC3 — a stale snapshot value falls back to v0.1 behavior):
    //   Input:  Some(<a path that does not exist>)
    //   Output: the process's current dir — the panel still opens, in the
    //           same place v0.1 would have put it.
    #[test]
    fn resolve_cwd_missing_directory_falls_back() {
        let expected = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        assert_eq!(resolve_cwd(Some("/definitely/not/a/real/dir/umux-test")), expected);
    }

    // T-C3 (no saved cwd — panels created fresh this session):
    //   Input:  None
    //   Output: current dir, exactly as v0.1's pty_open did.
    #[test]
    fn resolve_cwd_none_falls_back() {
        let expected = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        assert_eq!(resolve_cwd(None), expected);
    }

    // T-C4 (a hostile/accidental value naming a FILE, not a directory):
    //   Input:  Some(<path of an existing regular file>)
    //   Output: current dir — spawning a shell "in" a file is nonsense, and
    //           is_dir() (not just exists()) is what prevents it.
    #[test]
    fn resolve_cwd_file_path_falls_back() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let expected = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        assert_eq!(resolve_cwd(Some(file.path().to_str().unwrap())), expected);
    }

    // --- Phase 13 wiring (OSC -> notification) ---

    // A recording Notifier shared between the test and the service it's boxed
    // into (NotificationService requires Notifier: Send, hence Arc<Mutex<..>>).
    #[derive(Default, Clone)]
    struct RecordingNotifier {
        calls: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    impl notification_service::Notifier for RecordingNotifier {
        fn show(&self, summary: &str, body: &str) {
            self.calls
                .lock()
                .unwrap()
                .push((summary.to_string(), body.to_string()));
        }
    }

    fn wiring_service(
    ) -> (NotificationService, RecordingNotifier) {
        let rec = RecordingNotifier::default();
        let svc = NotificationService::new(Box::new(rec.clone()), Some("umux".to_string()));
        (svc, rec)
    }

    // T3 (AC4 — normal terminal output unaffected by the parser being active):
    //   Input:  a chunk of plain (non-OSC) bytes.
    //   Output: passthrough is byte-identical to the input, and no notification
    //           fires — the parser leaves ordinary output alone.
    #[test]
    fn process_plain_chunk_passes_through_silently() {
        let (svc, rec) = wiring_service();
        let mut parser = OscParser::new();
        let bytes = b"ls -la\r\nhello world";

        let (out, events) = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), bytes);

        assert_eq!(out, bytes.to_vec(), "plain bytes pass through unchanged");
        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "no notification for ordinary output"
        );
        assert!(events.is_empty(), "no completion signal for ordinary output");
    }

    // T4 (AC1 + AC2 — a completion sequence triggers a notification, no AI-tool
    //  config needed): an OSC 9 sequence (`ESC ] 9 ; <msg> BEL`) embedded in a
    //  chunk fires exactly one notification carrying the message, and the OSC
    //  bytes are stripped from what the terminal sees.
    #[test]
    fn process_chunk_with_osc9_fires_notification() {
        let (svc, rec) = wiring_service();
        let mut parser = OscParser::new();
        // "before" ESC ] 9 ; build done BEL "after"
        let bytes: Vec<u8> = [
            b'b', b'e', b'f', 0x1b, b']', b'9', b';', b'b', b'u', b'i', b'l', b'd',
            b' ', b'd', b'o', b'n', b'e', 0x07, b'a', b'f', b't',
        ]
        .to_vec();

        let (out, events) = process_pty_chunk(
            &mut parser,
            &svc,
            &PanelOrigin {
                workspace: Some("main".to_string()),
                panel: None,
            },
            &bytes,
        );

        // The terminal still sees the surrounding text, but NOT the OSC bytes.
        assert_eq!(out, b"befaft".to_vec(), "OSC bytes stripped from passthrough");

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "exactly one notification");
        assert!(calls[0].1.contains("build done"), "body carries the message");

        // v0.2 Phase 2 / #26: the same chunk also surfaces the parsed event so
        // the reader thread can emit the per-panel completion signal. Both
        // channels (desktop notification + status signal) fire from one chunk.
        assert_eq!(events.len(), 1, "exactly one completion event surfaced");
        assert_eq!(events[0].protocol, osc_parser::OscProtocol::Nine);
        assert!(events[0].body.contains("build done"));
    }

    // T5 (regression guard — parser state must persist across chunks in the
    //  wiring): a completion sequence split across two process_pty_chunk calls
    //  (terminator arrives in a later chunk) still surfaces exactly one event.
    //  If the wiring rebuilt the parser per chunk, split sequences would be lost.
    #[test]
    fn process_split_sequence_fires_once_across_chunks() {
        let (svc, rec) = wiring_service();
        let mut parser = OscParser::new();
        let first: Vec<u8> = [0x1b, b']', b'9', b';', b'h', b'i'].to_vec();
        let second: Vec<u8> = [0x07, b'x'].to_vec(); // BEL terminator + trailing byte

        let (out1, events1) = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), &first);
        let (out2, events2) = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), &second);

        // Nothing is complete until the terminator arrives.
        assert!(out1.is_empty(), "no passthrough from the unfinished sequence");
        assert!(events1.is_empty(), "no completion event before the terminator");
        // The trailing non-OSC byte after the terminator still reaches the term.
        assert_eq!(out2, b"x".to_vec());

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "split sequence fires exactly one notification");
        assert!(calls[0].1.contains("hi"), "body carries the message: {}", calls[0].1);
        assert_eq!(events2.len(), 1, "split sequence surfaces exactly one completion event");
    }

    // --- macOS notifier script (v0.2 Phase 2 / #26) -------------------------

    // Issue #68 — which macOS path a process takes is decided by ONE pure
    // predicate on current_exe: a binary inside an .app bundle posts through
    // UNUserNotificationCenter (umux attribution), everything else (the
    // `tauri dev` binary under target/, cargo test itself) uses osascript.
    #[cfg(target_os = "macos")]
    #[test]
    fn bundled_app_detected_by_exe_path() {
        assert!(is_bundled_app(std::path::Path::new(
            "/Applications/umux.app/Contents/MacOS/umux"
        )));
        assert!(!is_bundled_app(std::path::Path::new(
            "/Users/dev/projects/umux/src-tauri/target/debug/umux"
        )));
        assert!(!is_bundled_app(std::path::Path::new("/usr/local/bin/umux")));
    }

    // T7 (plain text needs no escaping — the notification reaches AppleScript
    // verbatim):
    //   Input:  summary "umux", body "build done"
    //   Output: display notification "build done" with title "umux"
    #[cfg(target_os = "macos")]
    #[test]
    fn apple_script_plain_text_is_verbatim() {
        assert_eq!(
            apple_notification_script("umux", "build done"),
            "display notification \"build done\" with title \"umux\""
        );
    }

    // T8 (quotes and backslashes in the message must not break out of the
    // AppleScript string literal — a hostile body can't inject script code):
    //   Input:  summary `umux "done"`, body `task "x" finished \o/`
    //   Output: every `"` escaped as `\"`, every `\` doubled.
    #[cfg(target_os = "macos")]
    #[test]
    fn apple_script_escapes_quotes_and_backslashes() {
        assert_eq!(
            apple_notification_script("umux \"done\"", "task \"x\" finished \\o/"),
            "display notification \"task \\\"x\\\" finished \\\\o/\" with title \"umux \\\"done\\\"\""
        );
    }

    // --- Phase 14: notification mute wiring (#15) --------------------------

    // T6 (AC2 — a muted flag suppresses the notification on the live stream):
    //   The real app shares ONE Arc<AtomicBool> across every panel's service so
    //   a single toggle silences the whole app. This test mirrors that wiring:
    //   it builds the service from a shared flag, flips that flag (as the Tauri
    //   `set_notifications_muted` command would), then runs an OSC 9 chunk.
    //   Input:  a shared mute flag set to true, then a chunk with an OSC 9 seq.
    //   Output: zero show() calls — no desktop notification — AND the OSC bytes
    //           are still stripped from passthrough (the parser runs regardless;
    //           only delivery is muted, not parsing).
    #[test]
    fn muted_shared_flag_suppresses_notification_from_chunk() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let rec = RecordingNotifier::default();
        let flag = Arc::new(AtomicBool::new(false));
        let svc = NotificationService::with_mute(
            Box::new(rec.clone()),
            Some("umux".to_string()),
            flag.clone(),
        );
        let mut parser = OscParser::new();

        // Flip the shared flag — the command path does exactly this.
        flag.store(true, Ordering::SeqCst);

        let bytes: Vec<u8> = [0x1b, b']', b'9', b';', b'h', b'i', 0x07].to_vec();
        let (out, events) = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), &bytes);

        assert!(
            out.is_empty(),
            "OSC bytes still stripped from passthrough even when muted"
        );
        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "muted flag suppresses the notification on the live stream"
        );
        // v0.2 Phase 2 / #26: muting notifications must NOT mute the status
        // dot — the completion event still travels to the renderer, so the
        // panel flips to needs-attention even with notifications silenced.
        assert_eq!(events.len(), 1, "completion signal still routed while muted");
    }

    // The per-OS config-directory + migration tests moved with their code to
    // the store_core crate (#58): store_core/src/paths.rs.
}
