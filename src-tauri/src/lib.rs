pub mod analytics;
pub mod cmux_import;
pub mod git_branch;
pub mod listening_ports;
pub mod notification_service;
pub mod osc_parser;
pub mod pty_debug;
pub mod pty_service;
pub mod session_driver;
pub mod shell_probe;
pub mod ssh_manager;
pub mod updater_probe;

// The workspace/settings store (model, WorkspaceStore, SettingsStore, config
// paths) is shared library code since #58: StoreCore (`store_core` crate)
// owns it so the upcoming `umux` CLI can write the same files through the
// same implementation. The app just consumes it here.
use store_core::paths::{config_dir, config_path, legacy_config_dir, migrate_legacy_config, reset_store_files, settings_path};
use store_core::settings_store::{settings_fallback_warning, Settings, SettingsStore};
use store_core::workspace_store::{fallback_warning, Group, Workspace, WorkspaceData, WorkspaceStore};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use notification_service::{NotificationService, Notifier, PanelOrigin};
use osc_parser::OscParser;
use session_driver::{RouterDriver, SessionCore};
use ssh_manager::parse_ssh_target;

// The SessionCore seam (#85, v1.7.0 phase 3): every session-touching
// command routes through the driver trait; the engine lives only behind
// session_driver (the in-process impl today). The pty_service module stays
// as the re-export path the rest of the crate uses for its pure helpers
// (process_name below); the ENGINE types are not touched outside the driver.
use pty_service::process_name;

/// The app-wide notification mute flag. One instance is created in `run()` and
/// shared (via Arc) with every panel's NotificationService, so a single toggle
/// silences notifications across the whole app. `false` = audible (default).
type MuteFlag = Arc<AtomicBool>;

/// One chunk of PTY output, ferried to the renderer over the `pty_output` event.
/// `data` is base64-encoded raw bytes; the frontend decodes it into a
/// `Uint8Array`. Base64 replaced the original JSON number array (perf audit
/// 2026-09-05): a 4 KB chunk is ~5.4 KB of base64 vs ~15 KB of
/// `[12,34,...]` text, and decoding is far cheaper than parsing a huge array —
/// this path runs for EVERY PTY chunk on EVERY open panel.
#[derive(Serialize, Clone)]
struct PtyOutputPayload {
    id: u32,
    data: String,
}

/// Encode raw PTY bytes for the wire (see PtyOutputPayload).
fn encode_payload(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
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

/// One ConEmu-style cwd report (`OSC 9;9;<path>`) surfaced by the OSC parser
/// from a panel's output stream (quickupdate 2026-09-13, the Windows
/// live-folder fix): the shell's prompt hook announces the shell's current
/// directory on every prompt render, and the frontend folds it into the
/// panel's workingDirectory — instant sidebar folder lines / branch labels
/// without waiting for the next periodic snapshot. Emitted for local AND SSH
/// panels; the frontend ignores reports from SSH panels (a remote path is not
/// a local workingDirectory).
#[derive(Serialize, Clone)]
struct PtyCwdPayload {
    id: u32,
    cwd: String,
}

#[tauri::command]
fn pty_open(
    app: AppHandle,
    driver: State<'_, RouterDriver>,
    mute: State<'_, MuteFlag>,
    shell: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    label: Option<String>,
    // Phase-4 rebind ids (issue #86): which workspace/tab/panel this
    // surface belongs to, so a Storestation-owned session can be found
    // again after a relaunch. Absent on older callers — always Optional.
    workspace_id: Option<String>,
    tab_id: Option<String>,
    panel_id: Option<String>,
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

    let params = session_driver::PtyOpenParams {
        shell,
        cwd,
        cols,
        rows,
        workspace_id,
        tab_id,
        panel_id,
    };
    let (id, rx) = driver.pty_open(&params).map_err(|e| e.to_string())?;

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
    // Opt-in pipeline diagnostics (issue #75 hunt): when the flag file exists,
    // every passthrough chunk leaves one numbers-only line (byte count, running
    // total, hex prefix — never terminal content) in pty-debug.log. The gate is
    // re-read per chunk, so dropping the file stops the dump immediately.
    std::thread::spawn(move || {
        let mut parser = OscParser::new();
        let service = NotificationService::with_mute(
            platform_notifier(&emit_app),
            Some("umux".to_string()),
            mute_flag,
        );
        let mut debug_total: u64 = 0;
        while let Ok(bytes) = rx.recv() {
            let result = process_pty_chunk(&mut parser, &service, &origin, id, &bytes);
            let debug_dir = config_dir();
            if pty_debug::enabled(&debug_dir) {
                debug_total += result.passthrough.len() as u64;
                pty_debug::append(
                    &debug_dir,
                    &pty_debug::chunk_line(id, debug_total, result.passthrough.len(), &result.passthrough),
                    pty_debug::max_log_bytes(),
                );
            }
            if let Some(cwd) = &result.cwd_report {
                // Shell-announced cwd (quickupdate 2026-09-13): instant sidebar
                // folder updates, ahead of the periodic snapshot.
                let _ = emit_app.emit("pty_cwd", PtyCwdPayload { id, cwd: cwd.clone() });
            }
            if !result.events.is_empty() {
                // Completion signal first, then the surviving output bytes: the
                // frontend status machine's grace window expects a TUI's
                // trailing redraw to land right AFTER its completion signal.
                let _ = emit_app.emit("pty_completion", PanelSignalPayload { id });
                // Announce the post for click-to-navigate (#76 follow-up): the
                // frontend's focus heuristic may only ever consume REAL
                // banners — a muted ping must not look posted.
                if !service.is_muted() {
                    let _ = emit_app.emit(
                        "notification_posted",
                        serde_json::json!({ "kind": "completion", "ptyId": id }).to_string(),
                    );
                }
            }
            if !result.passthrough.is_empty() {
                let _ = emit_app.emit(
                    "pty_output",
                    PtyOutputPayload { id, data: encode_payload(&result.passthrough) },
                );
            }
        }
    });

    Ok(id)
}

#[tauri::command]
fn pty_write(driver: State<'_, RouterDriver>, id: u32, data: String) -> Result<(), String> {
    let debug_dir = config_dir();
    if pty_debug::enabled(&debug_dir) {
        pty_debug::append(&debug_dir, &pty_debug::input_line(id, data.len()), pty_debug::max_log_bytes());
    }
    driver
        .pty_write(id, data.as_bytes())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn pty_resize(driver: State<'_, RouterDriver>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    driver.pty_resize(id, cols, rows).map_err(|e| e.to_string())
}

#[tauri::command]
fn pty_close(driver: State<'_, RouterDriver>, id: u32) -> Result<(), String> {
    driver.pty_close(id);
    Ok(())
}

/// Opt-in pipeline diagnostics (issue #75 hunt): the frontend periodically
/// reports how many characters a panel's DOM currently holds. Numbers only —
/// no terminal content ever crosses this boundary. Logged only while the
/// `debug-pty.flag` file exists, so a normal install does zero work here
/// (the frontend calls this unconditionally; the backend is the gate).
#[tauri::command]
fn pty_debug_paint(id: u32, chars: usize, visible: usize) -> Result<(), String> {
    let dir = config_dir();
    if pty_debug::enabled(&dir) {
        pty_debug::append(&dir, &pty_debug::paint_line(id, chars, visible), pty_debug::max_log_bytes());
    }
    Ok(())
}

/// v0.2 Phase 4 / #28 — is a live process (not the idle shell) running in
/// this local panel? Every close path (X button, Ctrl+Shift+W, workspace
/// close) asks this BEFORE tearing a panel down; `true` means the frontend
/// must confirm with the user first. SSH panels have no equivalent: the local
/// `ssh` client is always the foreground group while connected, and the
/// remote side is opaque (OSC-only, no polling), so they close without asking.
#[tauri::command]
fn pty_is_busy(driver: State<'_, RouterDriver>, id: u32) -> Result<bool, String> {
    Ok(driver.pty_is_busy(id))
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

// PERF (audit 2026-09-05): async — cwd reads are OS calls (on macOS a whole
// `lsof` spawn) and must never run on the UI thread. State is fetched from the
// AppHandle inside the worker thread: `State<'_, T>` cannot cross into a
// `'static` closure, but `app.state::<T>()` works from any thread.
#[tauri::command]
async fn panel_cwds(
    app: AppHandle,
    panels: Vec<CwdQuery>,
) -> Result<Vec<CwdAnswer>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let driver = app.state::<RouterDriver>();
        Ok(panels
            .into_iter()
            .map(|q| {
                let cwd = driver
                    .pty_cwd(q.pty_id)
                    .map(|p| p.to_string_lossy().into_owned());
                CwdAnswer {
                    panel_id: q.panel_id,
                    cwd,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
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

// PERF (audit 2026-09-05): this runs every ~2 s for every local panel. It used
// to be a SYNC command — Tauri runs those on the UI thread — that held the
// global PTY mutex while naming each pid, and naming on Windows spawned
// `tasklist.exe` (100–500 ms per call!). Net effect: the whole UI froze every
// two seconds and keystrokes stalled behind the scan. Now: pids resolve under
// a short lock, naming happens off-thread via the native API (pty_service::
// process_name), and the command is async so the UI thread is never involved.
#[tauri::command]
async fn panel_processes(
    driver: State<'_, RouterDriver>,
    panels: Vec<CwdQuery>,
) -> Result<Vec<PanelProcessAnswer>, String> {
    // Under the lock: only the cheap part — which pid owns each panel's fg.
    let resolved: Vec<(String, Option<u32>)> = panels
        .into_iter()
        .map(|q| {
            let pid = driver.pty_foreground_pid(q.pty_id);
            (q.panel_id, pid)
        })
        .collect();
    // Off the lock (and off the UI thread): name each pid.
    tauri::async_runtime::spawn_blocking(move || {
        Ok(resolved
            .into_iter()
            .map(|(panel_id, pid)| PanelProcessAnswer {
                panel_id,
                process: pid.and_then(process_name),
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

// --- Splashscreen handoff (quickupdate 2026-09-13) ---------------------------
//
// The splash window is visible from process start (it is a static page, so it
// paints immediately); the MAIN window starts hidden so the app never shows a
// half-booted UI. The frontend invokes this once its boot work is done (workspaces
// + settings loaded, session restore applied), which closes the splash and
// reveals the main window. Idempotent: a missing window label is fine, and a
// second call just re-shows the (already visible) main window.

#[tauri::command]
fn close_splashscreen(app: AppHandle) {
    // The splash is the gate: a call with the splash already gone (a dev-HMR
    // reload re-runs the frontend boot effect) must not steal focus back.
    if let Some(splash) = app.get_webview_window("splashscreen") {
        let _ = splash.close();
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.show();
            let _ = main.set_focus();
        }
    }
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

// PERF (audit 2026-09-05): async — each dir means `.git` file reads (slow
// under real-time antivirus); the UI thread must never wait on disk.
#[tauri::command]
async fn git_branches(dirs: Vec<String>) -> Result<Vec<GitBranchAnswer>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(dirs
            .into_iter()
            .map(|dir| GitBranchAnswer {
                branch: git_branch::resolve_branch(std::path::Path::new(&dir)),
                dir,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
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

// PERF (audit 2026-09-05): the socket tables come from `netstat -ano` on
// Windows (hundreds of ms) and the old sync version held the global PTY mutex
// WHILE running it — every keystroke stalled behind a tab hover. Now the lock
// only collects child pids; the scan runs off-thread.
#[tauri::command]
async fn tab_ports(
    driver: State<'_, RouterDriver>,
    tabs: Vec<TabPortsQuery>,
) -> Result<Vec<TabPortsAnswer>, String> {
    // Under the lock: just the pid roots per tab (cheap).
    let rooted: Vec<(String, Vec<u32>)> = tabs
        .into_iter()
        .map(|tab| {
            let roots: Vec<u32> = tab
                .pty_ids
                .iter()
                .filter_map(|id| driver.pty_child_pid(*id))
                .collect();
            (tab.tab_id, roots)
        })
        .collect();
    // Off the lock: the actual socket-table scan.
    tauri::async_runtime::spawn_blocking(move || {
        let listeners = listening_ports::listening_sockets();
        let edges = listening_ports::parent_edges();
        Ok(rooted
            .into_iter()
            .map(|(tab_id, roots)| TabPortsAnswer {
                ports: listening_ports::aggregate_ports(&listeners, &edges, &roots),
                tab_id,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
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
    driver: State<'_, RouterDriver>,
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

    let (id, rx) = driver
        .ssh_open(&parsed, cwd, cols, rows)
        .map_err(|e| e.to_string())?;

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
            let result = process_pty_chunk(&mut parser, &service, &origin, id, &bytes);
            if let Some(cwd) = &result.cwd_report {
                // A remote shell announcing ITS cwd. The frontend ignores cwd
                // reports for SSH panels (a remote path is not a local
                // workingDirectory) — this emission is future-proofing for the
                // SSH View's remote control, not live data today.
                let _ = emit_app.emit("ssh_cwd", PtyCwdPayload { id, cwd: cwd.clone() });
            }
            if !result.events.is_empty() {
                // Same per-panel completion signal as local panels (see the
                // pty_open thread) — remote status parity (#26).
                let _ = emit_app.emit("ssh_completion", PanelSignalPayload { id });
                // Same post announcement as local panels (click-to-navigate).
                if !service.is_muted() {
                    let _ = emit_app.emit(
                        "notification_posted",
                        serde_json::json!({ "kind": "completion", "ptyId": id }).to_string(),
                    );
                }
            }
            if !result.passthrough.is_empty() {
                let _ = emit_app.emit(
                    "ssh_output",
                    PtyOutputPayload { id, data: encode_payload(&result.passthrough) },
                );
            }
        }

        // The output stream ended (ssh exited). Poll its exit code and, if it's
        // a connection failure (255 / signal), emit a `ssh_exit` event carrying
        // a clear message so the frontend can show a diagnostic instead of a
        // dead, blank panel. A clean or remote-command exit yields error=None.
        let error = poll_ssh_exit(&emit_app, id, &host);
        let _ = emit_app.emit("ssh_exit", SshExitPayload { id, error });
    });

    Ok(id)
}

#[tauri::command]
fn ssh_write(driver: State<'_, RouterDriver>, id: u32, data: String) -> Result<(), String> {
    driver.ssh_write(id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
fn ssh_resize(driver: State<'_, RouterDriver>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    driver.ssh_resize(id, cols, rows).map_err(|e| e.to_string())
}

#[tauri::command]
fn ssh_close(driver: State<'_, RouterDriver>, id: u32) -> Result<(), String> {
    driver.ssh_close(id);
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
struct NativeNotifier {
    /// Needed to emit `notification_activated` when the user clicks the
    /// banner's action (click-to-navigate, #76 follow-up).
    app: AppHandle,
}

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

    /// The real click path (#76 follow-up): the banner carries an "Open"
    /// action button; `--wait` keeps the notify-send process alive until the
    /// banner closes and prints the ACTIVATED action's id on stdout — "open"
    /// means the user clicked it, anything else (timeout, dismissed) means
    /// they didn't. The wait runs on a DETACHED thread: a banner can sit for
    /// many seconds and the caller (a PTY reader thread or an invoke) must
    /// never block on its lifetime.
    fn show_actionable(&self, summary: &str, body: &str, payload: &str) {
        let app = self.app.clone();
        let summary = summary.to_string();
        let body = body.to_string();
        let payload = payload.to_string();
        std::thread::spawn(move || {
            let result = std::process::Command::new("notify-send")
                .arg("--app-name=umux")
                .arg("--wait")
                .arg("--action=open=Open")
                .arg(&summary)
                .arg(&body)
                .output();
            let activated = match &result {
                Ok(out) => {
                    out.status.success()
                        && String::from_utf8_lossy(&out.stdout).trim() == "open"
                }
                Err(_) => false,
            };
            if activated {
                log::info!("[notify] action activated: payload={payload:?}");
                let _ = app.emit("notification_activated", payload);
            }
        });
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
        // CREATE_NO_WINDOW (perf/UX audit 2026-09-05): without it every toast
        // flashed a console window on the desktop; the toast itself is
        // rendered by the OS notification service and is unaffected.
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let result = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
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
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        Box::new(NativeNotifier)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Box::new(NativeNotifier { app: app.clone() })
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

// PERF (audit 2026-09-05): async — config reads/writes hit the disk (slow
// under real-time antivirus); the UI thread must never wait on them.
#[tauri::command]
async fn load_workspaces(app: AppHandle) -> Result<WorkspaceData, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<WorkspaceStore>();
        let (data, status) = state.load_with_status();
        // AC3: a corrupted config must not be a silent downgrade. Surface a clear
        // message both in the backend log and as an event the frontend can render.
        // Missing is a normal first run -> silent; Ok is success.
        if let Some(message) = fallback_warning(status) {
            log::warn!("[config] fallback: {message}");
            let _ = app.emit("config_fallback", ConfigFallbackPayload { message });
        }
        Ok(data)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_workspaces(
    app: AppHandle,
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
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<WorkspaceStore>();
        state
            .save(&WorkspaceData {
                workspaces,
                groups,
                order,
            })
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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

/// Where a waiting ping's panel lives — display labels for the banner body
/// plus the ids a click needs to navigate back (#76 follow-up). One struct so
/// the invoke boundary is one key, not five.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitingPingTarget {
    workspace: Option<String>,
    panel: Option<String>,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    panel_id: Option<String>,
}

/// Fire the presence-based waiting ping for one panel (issue #76). The
/// frontend's agent-status machine detected the transition INTO
/// needs-attention via CLI PRESENCE (panel_processes polling — never terminal
/// content) and reports WHERE via `target`. The service here applies the same
/// app-wide mute flag the bell button and the OSC completion path use — one
/// mute is one mute — and composes the fixed waiting message with the origin
/// suffix, exactly like a completion ping. The banner carries the navigation
/// payload, and the post is announced (`notification_posted`) so the
/// frontend's click/focus routing only ever consumes REAL banners. Emission
/// ONCE per transition is the caller's contract: the frontend fires on the
/// state change only, never per poll tick.
// PERF (audit 2026-09-05): async — a desktop notification is an external
// roundtrip (D-Bus / osascript / UNUserNotificationCenter); the UI thread
// must never wait on it (same reasoning as load_workspaces).
#[tauri::command]
async fn notify_panel_needs_input(
    app: AppHandle,
    target: WaitingPingTarget,
    mute: State<'_, MuteFlag>,
) -> Result<(), String> {
    let mute = Arc::clone(&*mute);
    let origin = PanelOrigin {
        workspace: target.workspace,
        panel: target.panel,
    };
    let payload = serde_json::json!({
        "kind": "waiting",
        "workspaceId": target.workspace_id,
        "tabId": target.tab_id,
        "panelId": target.panel_id,
    })
    .to_string();
    let emit_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let service = NotificationService::with_mute(
            platform_notifier(&app),
            Some("umux".to_string()),
            mute,
        );
        let delivered = service.notify_waiting_with_payload(&origin, Some(&payload));
        if delivered {
            let _ = emit_app.emit("notification_posted", payload);
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Load the persisted feature toggles (v0.2 Phase 3 / #27). A corrupted file
/// falls back to defaults AND emits the same `config_fallback` warning the
/// workspace config uses, so the downgrade is surfaced, not silent.
// PERF (audit 2026-09-05): async — same disk-wait reasoning as load_workspaces.
#[tauri::command]
async fn load_settings(app: AppHandle) -> Result<Settings, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SettingsStore>();
        let (settings, status) = state.load_with_status();
        if let Some(message) = settings_fallback_warning(status) {
            log::warn!("[config] settings fallback: {message}");
            let _ = app.emit("config_fallback", ConfigFallbackPayload { message });
        }
        Ok(settings)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Persist the feature toggles. The param is named `settings` to match the
/// invoke key the frontend sends (`invoke('save_settings', { settings })`) —
/// Tauri maps arguments by name (see the save_workspaces comment).
#[tauri::command]
async fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SettingsStore>();
        state.save(&settings).map_err(|e| e.to_string())?;
        // The Storestation toggle's live image rides the same write (#86):
        // the router reads an AtomicBool, never the disk.
        app.state::<RouterDriver>()
            .set_daemon_enabled(settings.storestation.daemon_enabled);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

// --- umux Storestation (v1.7.0 phase 4 / #86) ---------------------------------
//
// The Settings section's backend: a live status probe (offline is a state)
// and the toggle. Enable makes sure a daemon is running — probing first,
// spawning the bundled binary when absent (dev: it is the workspace binary
// sitting beside the app in the cargo target dir) — then installs the
// daemon-client driver. Disable runs the daemon's own graceful shutdown
// (every owned shell dies cleanly, story 110).

/// Spawn `umux-storestation run` when no daemon answers. `Ok(false)` = one
/// was already running; `Ok(true)` = we spawned it and it answered. The
/// binary is looked up beside the app executable first (installer layout +
/// the cargo target dir in dev), then PATH.
fn spawn_daemon_if_absent() -> Result<bool, String> {
    let dir = config_dir();
    let probe = |dir: &PathBuf| {
        umux_storestation::client::Client::connect(dir, "desktop", env!("CARGO_PKG_VERSION"))
            .is_ok()
    };
    if probe(&dir) {
        return Ok(false); // somebody else's daemon — use it, spawn nothing
    }
    let bin = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|dir| dir.join(format!("umux-storestation{}", std::env::consts::EXE_SUFFIX)))
        })
        .filter(|path| path.is_file());
    let mut command = match bin {
        Some(path) => std::process::Command::new(path),
        None => std::process::Command::new("umux-storestation"),
    };
    // Fully detached: the daemon outlives the app by design (THE DEMO).
    command
        .arg("run")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .spawn()
        .map_err(|e| {
            format!(
                "could not start umux-storestation: {e} — in dev, build it once with \
                 `cd src-tauri && cargo build` (it lands beside the app binary)"
            )
        })?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if probe(&dir) {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                "umux-storestation did not answer within 5 s of being started".into(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[tauri::command]
async fn storestation_status(app: AppHandle) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let driver = app.state::<RouterDriver>();
        driver.storestation_status()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn storestation_set_enabled(
    app: AppHandle,
    enable: bool,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let driver = app.state::<RouterDriver>();
        if enable {
            spawn_daemon_if_absent()?;
            driver.connect_daemon()?;
        } else {
            driver.disable_daemon()?;
        }
        Ok(driver.storestation_status())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Factory reset (#74): remove EVERY umux state file — workspaces.json (the
/// workspace/layout/tree store) and settings.json — so the next launch boots
/// first-run clean. The frontend restarts the app afterwards (plugin-process
/// relaunch), so the fresh state is what the user actually sees.
#[tauri::command]
fn reset_all(app: AppHandle) -> Result<(), String> {
    reset_store_files(&config_dir()).map_err(|e| format!("reset failed: {e}"))?;
    // quickupdate 2026-09-18: the geometry plugin keeps .window-state.json in
    // the identifier config dir (a directory of ours with nothing else in it),
    // so the reset clears it too — "Reset" forgets the window size as well.
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::remove_file(dir.join(tauri_plugin_window_state::DEFAULT_FILENAME));
    }
    Ok(())
}

/// Raw installed-shell probes for the Settings "Default shell" picker (#77):
/// PATH scan everywhere, /etc/shells + the login shell on Unix, registry App
/// Paths on Windows. RAW results only — ranking, dedup, and display names
/// live in the pure TS ShellDetector (src/shellDetector.ts); nothing here
/// assumes any specific shell exists, so an empty list is a valid answer.
// PERF (2026-09-11): async — the probe scans every PATH dir × known shells
// (hundreds of stat calls) and on Windows spawns up to eight `reg query`
// children (hundreds of ms warm, seconds under antivirus). This used to be a
// SYNC command — Tauri runs those on the UI thread — and the frontend calls it
// at every boot (the #78 arrow needs the shell count) and on every Settings
// open, so the whole window froze right at startup. Same fix as panel_cwds /
// panel_processes (audit 2026-09-05): the probe runs on a worker thread and
// the UI thread is never involved. A failed join degrades to the empty list —
// the same answer the frontend already treats as "no shells detected".
#[tauri::command]
async fn list_shells() -> Vec<shell_probe::ShellProbe> {
    tauri::async_runtime::spawn_blocking(shell_probe::probe_shells)
        .await
        .unwrap_or_default()
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
/// `friendly_ssh_exit` translator with one bounded poll through the driver.
fn poll_ssh_exit(app: &AppHandle, id: u32, host: &str) -> Option<String> {
    let driver = app.state::<RouterDriver>();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let code = loop {
        match driver.ssh_exit_code(id) {
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
/// completion events it surfaces. Returns the parser's full view of the chunk:
/// the bytes the terminal should still see (everything that wasn't a
/// recognized notification sequence — byte-identical to the input for non-OSC
/// bytes, satisfying "normal terminal output is unaffected by the parser being
/// active"), the notification events, and any cwd report.
///
/// Each notification goes out with a navigation payload (#76 follow-up: which
/// pty fired — a click on the banner returns to this panel), so the caller
/// should echo `notification_posted` for the frontend's click/focus routing.
///
/// The parser is held by the caller (one per PTY) so sequences split across
/// chunk boundaries are recognized across calls. This is the testable core of
/// the PTY-output wiring; `pty_open` plugs a real `Notifier` into it.
fn process_pty_chunk(
    parser: &mut OscParser,
    service: &NotificationService,
    origin: &PanelOrigin,
    pty_id: u32,
    bytes: &[u8],
) -> osc_parser::PushResult {
    let result = parser.push(bytes);
    let payload = serde_json::json!({ "kind": "completion", "ptyId": pty_id }).to_string();
    for event in &result.events {
        service.notify_with_payload(event, origin, Some(&payload));
    }
    // The whole PushResult travels back so the reader thread can ALSO emit the
    // per-panel completion signal (`pty_completion` / `ssh_completion`) and any
    // cwd report (`pty_cwd`). The notify above (desktop notification) is
    // unchanged — v0.1 behavior.
    result
}

/// The user's real login shell from the OS user database (getpwuid), or None.
///
/// A GUI-launched app (Finder/Dock on macOS, a desktop launcher on Linux)
/// inherits NO `$SHELL` at all, so when `$SHELL` is missing this is the truth
/// about which shell the user actually runs. Without it the old fallback
/// launched `/bin/sh` for a zsh user: none of their config loaded — no prompt
/// integration (git missing from the prompt), no aliases (macOS report
/// 2026-09-07).
#[cfg(unix)]
pub(crate) fn passwd_shell() -> Option<String> {
    // SAFETY: getpwuid returns a pointer into libc's static per-user storage;
    // the shell path is copied out immediately and nothing else is retained.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr((*pw).pw_shell).to_string_lossy().into_owned();
        if shell.is_empty() {
            None
        } else {
            Some(shell)
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn passwd_shell() -> Option<String> {
    None
}

/// Decide which shell binary to launch for a panel.
///
/// An explicit override (from a future WorkspaceStore config) wins; otherwise
/// the default is per-OS (v1.0 Phase 9 / #33): Windows PowerShell on Windows
/// (the in-box Windows PowerShell 5.1 — always present on Windows 10+, which
/// pwsh is not), then the user's `$SHELL`, then their passwd login shell (a
/// GUI launch has no `$SHELL` env var to read), then `/bin/sh`.
pub fn resolve_shell(shell: Option<&str>) -> String {
    resolve_shell_from(
        shell,
        std::env::var("SHELL").ok().filter(|s| !s.is_empty()),
        passwd_shell(),
    )
}

/// Pure decision core of `resolve_shell` — the env and passwd lookups are the
/// only OS-bound parts, injected here so the whole chain is testable.
fn resolve_shell_from(
    shell: Option<&str>,
    env_shell: Option<String>,
    passwd: Option<String>,
) -> String {
    if let Some(s) = shell {
        return s.to_string();
    }
    if cfg!(windows) {
        return "powershell.exe".to_string();
    }
    env_shell
        .or(passwd)
        .unwrap_or_else(|| "/bin/sh".to_string())
}

/// The user's home directory ($HOME on unix, $USERPROFILE on Windows), or
/// None when unset or not an existing directory.
///
/// Panels must open SOMEWHERE the user can actually use. The old fallback was
/// umux's own current dir — arbitrary for a GUI launch, and after an in-app
/// updater relaunch it can even be a DELETED directory (macOS 2026-09-07: the
/// shell opened "in a dot", `ls` answered "Operation not permitted", lsof
/// reported the cwd as "."). The home dir is what every native terminal does.
fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    match std::env::var(var) {
        Ok(dir) if !dir.is_empty() && std::path::Path::new(&dir).is_dir() => {
            Some(PathBuf::from(dir))
        }
        _ => None,
    }
}

/// Decide the working directory a new shell starts in (v0.2 Phase 5 / #29).
/// A cwd saved in the session snapshot wins when it still exists, is a
/// directory, and is ABSOLUTE — a relative value (e.g. the "." lsof once
/// reported for a shell in a dead directory, macOS 2026-09-07) would resolve
/// against umux's own — arbitrary, GUI-launch — cwd and must not pose as a
/// saved location. Anything else falls back to the user's home dir — a GUI
/// launch's own current dir is an arbitrary system location and can be dead
/// after an updater relaunch (see `home_dir`) — then the app's current dir,
/// then `/`.
pub fn resolve_cwd(cwd: Option<&str>) -> PathBuf {
    match cwd {
        Some(dir)
            if !dir.is_empty()
                && std::path::Path::new(dir).is_absolute()
                && std::path::Path::new(dir).is_dir() =>
        {
            PathBuf::from(dir)
        }
        _ => home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))),
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
    // v0.2 Phase 6 / #30, quickupdate 2026-09-12 (Adam): analytics is ALWAYS
    // on — the old analyticsEnabled kill switch is gone from the schema (the
    // CLI's config get/set entries and the settings field no longer exist),
    // so the Aptabase plugin below registers unconditionally and nothing can
    // turn it off.

    // The SessionCore router (#85 seam + #86 daemon face): seeded with the
    // persisted Storestation toggle. Startup with the daemon ON connects
    // lazily — the first panel open (or the Settings status row) makes the
    // socket connection, so a daemon that is not up yet never blocks boot.
    let router = RouterDriver::new(initial_settings.storestation.daemon_enabled);

    let builder = tauri::Builder::default()
        // The SessionCore seam (#85): the ONE managed state the session
        // commands go through. The in-process driver is today's face
        // (Storestation OFF — byte-identical to v1.6.x); with the daemon ON
        // the same trait routes to the socket driver.
        .manage(router)
        .manage(WorkspaceStore::new(config_path()))
        .manage(settings_store)
        .manage(mute)
        .invoke_handler(tauri::generate_handler![
            pty_open,
            pty_write,
            pty_resize,
            pty_close,
            pty_is_busy,
            pty_debug_paint,
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
            notify_panel_needs_input,
            load_settings,
            save_settings,
            storestation_status,
            storestation_set_enabled,
            reset_all,
            list_shells,
            open_settings_file,
            updater_status,
            close_splashscreen,
            cmux_import::read_cmux_import_sources,
        ]);

    // #30 AC2, quickupdate 2026-09-12: the plugin is the only place the SDK
    // comes to life, and it registers on EVERY startup — analytics has no
    // off state anymore. (The plugin flushes its queue on app exit by
    // itself, so no exit hook is needed here.)
    let builder = builder.plugin(analytics::aptabase_plugin());

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
        .plugin(tauri_plugin_notification::init())
        // Issue #72: clicking a listening port opens http://localhost:{port}
        // in the system browser (open-url); copy stays on the frontend.
        .plugin(tauri_plugin_opener::init())
        // Window geometry persistence (quickupdate 2026-09-18): size, position,
        // maximized and fullscreen are saved when the app exits and restored at
        // window creation — while the splash is still up and the main window is
        // hidden — so the close_splashscreen reveal already shows the restored
        // geometry. VISIBLE is deliberately NOT tracked: visibility is the
        // splash handoff's job (the main window must stay hidden until boot
        // completes), and DECORATIONS is not user-adjustable in umux. The
        // splash keeps its fixed centered look via the denylist, which also
        // stops the plugin from saving its 320x220 box.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .with_denylist(&["splashscreen"])
                .build(),
        );

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
            // users. Unconditional: analytics is always on (quickupdate
            // 2026-09-12), matching the plugin registration above.
            {
                use tauri_plugin_aptabase::EventTracker;
                if let Err(e) = app.handle().track_event(analytics::APP_OPEN_EVENT, None) {
                    log::warn!("[analytics] track_event failed: {e}");
                }
            }
            // HITL fix 2026-09-10: WebView2's BROWSER accelerator keys (F5,
            // F12, Ctrl+Shift+C/I, print/zoom chords) are consumed by the
            // webview before the page ever sees them — Ctrl+Shift+C opened
            // DevTools instead of letting the terminal copy its selection
            // (clipboardShortcut never ran). A terminal owns its own
            // keyboard: turn the browser accelerators off entirely. Failures
            // log and continue — a missing settings interface must never
            // keep the window from opening.
            #[cfg(windows)]
            if let Some(win) = app.get_webview_window("main") {
                use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings6;
                let result = win.with_webview(move |webview| unsafe {
                    use windows::core::Interface;
                    let controller = webview.controller();
                    let Ok(core) = controller.CoreWebView2() else {
                        log::warn!("[accelerators] CoreWebView2 unavailable");
                        return;
                    };
                    let Ok(settings) = core.Settings() else {
                        log::warn!("[accelerators] WebView2 settings unavailable");
                        return;
                    };
                    match settings.cast::<ICoreWebView2Settings6>() {
                        Ok(s6) => {
                            if let Err(e) = s6.SetAreBrowserAcceleratorKeysEnabled(false) {
                                log::warn!("[accelerators] disable failed: {e}");
                            }
                        }
                        Err(e) => log::warn!("[accelerators] settings6 cast failed: {e}"),
                    }
                });
                if let Err(e) = result {
                    log::warn!("[accelerators] with_webview failed: {e}");
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

    // T3 (macOS report 2026-09-07 — GUI launch has no $SHELL): with the env
    // var missing, the passwd entry decides. A zsh user launched from
    // Finder/Dock must get zsh, not /bin/sh — sh reads none of their config,
    // so the prompt lost its git integration and aliases. (Unix-only: on
    // Windows resolve_shell_from answers "powershell.exe" before the chain.)
    #[cfg(not(windows))]
    #[test]
    fn resolve_shell_without_env_uses_passwd_shell() {
        assert_eq!(
            resolve_shell_from(None, None, Some("/bin/zsh".to_string())),
            "/bin/zsh"
        );
    }

    // T4: $SHELL (a dev run from a terminal) still outranks the passwd entry.
    #[cfg(not(windows))]
    #[test]
    fn resolve_shell_env_outranks_passwd() {
        assert_eq!(
            resolve_shell_from(None, Some("/bin/bash".to_string()), Some("/bin/zsh".to_string())),
            "/bin/bash"
        );
    }

    // T5: no override, no $SHELL, no passwd entry (unknown user) — the old
    // /bin/sh last resort still applies.
    #[cfg(not(windows))]
    #[test]
    fn resolve_shell_nothing_known_falls_back_to_sh() {
        assert_eq!(resolve_shell_from(None, None, None), "/bin/sh");
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

    // T-C2 (AC3 — a stale snapshot value falls back to a usable directory):
    //   Input:  Some(<a path that does not exist>)
    //   Output: the user's HOME dir (macOS 2026-09-07: the old app-cwd
    //           fallback put GUI-launched panels in an arbitrary — possibly
    //           deleted — directory, "ls: Operation not permitted"). Only
    //           when HOME is unusable does the app cwd apply.
    #[test]
    fn resolve_cwd_missing_directory_falls_back() {
        let expected = home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
        assert_eq!(resolve_cwd(Some("/definitely/not/a/real/dir/umux-test")), expected);
    }

    // T-C2b (macOS 2026-09-07 — the stored dot): a RELATIVE saved cwd exists
    // on disk (it resolves against the app's cwd), but it is not a real
    // saved location — it must fall back to home, not silently reopen the
    // panel wherever umux itself happens to run.
    #[test]
    fn resolve_cwd_relative_saved_value_falls_back() {
        let expected = home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
        assert_eq!(resolve_cwd(Some(".")), expected);
    }

    // T-C3 (no saved cwd — panels created fresh this session):
    //   Input:  None
    //   Output: the user's HOME dir, like every native terminal (macOS
    //           2026-09-07: the app-cwd fallback opened GUI-launched panels
    //           in an arbitrary — sometimes dead — directory).
    #[test]
    fn resolve_cwd_none_falls_back() {
        let expected = home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
        assert_eq!(resolve_cwd(None), expected);
    }

    // T-C4 (a hostile/accidental value naming a FILE, not a directory):
    //   Input:  Some(<path of an existing regular file>)
    //   Output: home dir — spawning a shell "in" a file is nonsense, and
    //           is_dir() (not just exists()) is what prevents it.
    #[test]
    fn resolve_cwd_file_path_falls_back() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let expected = home_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
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

        let result = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), 0, bytes);

        assert_eq!(result.passthrough, bytes.to_vec(), "plain bytes pass through unchanged");
        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "no notification for ordinary output"
        );
        assert!(
            result.events.is_empty(),
            "no completion signal for ordinary output"
        );
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

        let result = process_pty_chunk(
            &mut parser,
            &svc,
            &PanelOrigin {
                workspace: Some("main".to_string()),
                panel: None,
            },
            0,
            &bytes,
        );

        // The terminal still sees the surrounding text, but NOT the OSC bytes.
        assert_eq!(
            result.passthrough,
            b"befaft".to_vec(),
            "OSC bytes stripped from passthrough"
        );

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "exactly one notification");
        assert!(calls[0].1.contains("build done"), "body carries the message");

        // v0.2 Phase 2 / #26: the same chunk also surfaces the parsed event so
        // the reader thread can emit the per-panel completion signal. Both
        // channels (desktop notification + status signal) fire from one chunk.
        assert_eq!(result.events.len(), 1, "exactly one completion event surfaced");
        assert_eq!(result.events[0].protocol, osc_parser::OscProtocol::Nine);
        assert!(result.events[0].body.contains("build done"));
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

        let first = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), 0, &first);
        let second = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), 0, &second);

        // Nothing is complete until the terminator arrives.
        assert!(first.passthrough.is_empty(), "no passthrough from the unfinished sequence");
        assert!(first.events.is_empty(), "no completion event before the terminator");
        // The trailing non-OSC byte after the terminator still reaches the term.
        assert_eq!(second.passthrough, b"x".to_vec());

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "split sequence fires exactly one notification");
        assert!(calls[0].1.contains("hi"), "body carries the message: {}", calls[0].1);
        assert_eq!(
            second.events.len(),
            1,
            "split sequence surfaces exactly one completion event"
        );
    }

    // T6 (quickupdate 2026-09-13 — a cwd report rides the same wiring): the
    // shell's `9;9;<cwd>` surfaces as cwd_report for the reader thread's
    // `pty_cwd` emission, is NOT a notification, and the sequence still passes
    // through untouched (the terminal never loses bytes).
    #[test]
    fn process_chunk_with_cwd_report_surfaces_path() {
        let (svc, rec) = wiring_service();
        let mut parser = OscParser::new();
        let bytes = b"\x1b]9;9;C:\\proj\x07ok".to_vec();

        let result = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), 0, &bytes);

        assert_eq!(result.cwd_report.as_deref(), Some("C:\\proj"));
        assert_eq!(result.passthrough, bytes, "cwd report passes through byte-identical");
        assert!(result.events.is_empty(), "a cwd report is not a completion");
        assert!(rec.calls.lock().unwrap().is_empty(), "no desktop notification either");
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
        let result = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), 0, &bytes);

        assert!(
            result.passthrough.is_empty(),
            "OSC bytes still stripped from passthrough even when muted"
        );
        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "muted flag suppresses the notification on the live stream"
        );
        // v0.2 Phase 2 / #26: muting notifications must NOT mute the status
        // dot — the completion event still travels to the renderer, so the
        // panel flips to needs-attention even with notifications silenced.
        assert_eq!(
            result.events.len(),
            1,
            "completion signal still routed while muted"
        );
    }

    // The per-OS config-directory + migration tests moved with their code to
    // the store_core crate (#58): store_core/src/paths.rs.
}
