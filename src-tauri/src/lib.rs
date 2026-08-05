pub mod notification_service;
pub mod osc_parser;
pub mod pty_service;
pub mod ssh_manager;
pub mod workspace_store;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use notification_service::{NotificationService, Notifier, PanelOrigin};
use osc_parser::OscParser;
use pty_service::{PtyHandle, PtyService};
use ssh_manager::{parse_ssh_target, SshHandle, SshManager};
use workspace_store::{WorkspaceData, WorkspaceStore};

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

#[tauri::command]
fn pty_open(
    app: AppHandle,
    state: State<'_, Mutex<PtyService>>,
    mute: State<'_, MuteFlag>,
    shell: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    label: Option<String>,
) -> Result<u32, String> {
    // Default to the user's $SHELL, falling back to /bin/sh; an explicit
    // `shell` override (from a future WorkspaceStore config) wins.
    let shell = resolve_shell(shell.as_deref());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

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
            Box::new(LibnotifyNotifier),
            Some("umux".to_string()),
            mute_flag,
        );
        while let Ok(bytes) = rx.recv() {
            let passthrough = process_pty_chunk(&mut parser, &service, &origin, &bytes);
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
            Box::new(LibnotifyNotifier),
            Some("umux".to_string()),
            mute_flag,
        );
        while let Ok(bytes) = rx.recv() {
            let passthrough = process_pty_chunk(&mut parser, &service, &origin, &bytes);
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

/// Real `Notifier` backed by libnotify via the `notify-send` CLI. Thin adapter
/// on the OS boundary — not unit-tested; its behavior is verified manually by
/// Adam on Ubuntu/Wayland.
///
/// We shell out to `notify-send` (the libnotify command-line client) rather
/// than the `notify-rust` D-Bus API because GNOME silently drops banners from
/// `notify-rust` when the sending process has no associated `.desktop` file
/// (the case during `tauri dev`), while `notify-send` is always shown. A failed
/// spawn (no libnotify-bin installed) is logged but otherwise swallowed so a
/// missing notifier can never break the terminal stream.
struct LibnotifyNotifier;

impl Notifier for LibnotifyNotifier {
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

/// Where the workspace config file lives: `$XDG_CONFIG_HOME/umux/workspaces.json`
/// or `~/.config/umux/workspaces.json` when XDG is unset.
fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("umux").join("workspaces.json")
}

#[tauri::command]
fn load_workspaces(state: State<'_, WorkspaceStore>) -> WorkspaceData {
    state.load()
}

#[tauri::command]
fn save_workspaces(state: State<'_, WorkspaceStore>, data: WorkspaceData) -> Result<(), String> {
    state.save(&data).map_err(|e| e.to_string())
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
) -> Vec<u8> {
    let result = parser.push(bytes);
    for event in &result.events {
        service.notify(event, origin);
    }
    result.passthrough
}

/// Decide which shell binary to launch for a panel.
///
/// An explicit override (from a future WorkspaceStore config) wins; otherwise
/// we fall back to the user's `$SHELL`, then `/bin/sh` as a last resort.
pub fn resolve_shell(shell: Option<&str>) -> String {
    shell
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(PtyService::new()))
        .manage(Mutex::new(SshManager::new()))
        .manage(WorkspaceStore::new(config_path()))
        .manage(Arc::new(AtomicBool::new(false)) as MuteFlag)
        .invoke_handler(tauri::generate_handler![
            pty_open,
            pty_write,
            pty_resize,
            pty_close,
            ssh_open,
            ssh_write,
            ssh_resize,
            ssh_close,
            load_workspaces,
            save_workspaces,
            set_notifications_muted,
            notifications_muted,
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
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

        let out = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), bytes);

        assert_eq!(out, bytes.to_vec(), "plain bytes pass through unchanged");
        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "no notification for ordinary output"
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

        let out = process_pty_chunk(
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

        let out1 = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), &first);
        let out2 = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), &second);

        // Nothing is complete until the terminator arrives.
        assert!(out1.is_empty(), "no passthrough from the unfinished sequence");
        // The trailing non-OSC byte after the terminator still reaches the term.
        assert_eq!(out2, b"x".to_vec());

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "split sequence fires exactly one notification");
        assert!(calls[0].1.contains("hi"), "body carries the message: {}", calls[0].1);
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
        let out = process_pty_chunk(&mut parser, &svc, &PanelOrigin::default(), &bytes);

        assert!(
            out.is_empty(),
            "OSC bytes still stripped from passthrough even when muted"
        );
        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "muted flag suppresses the notification on the live stream"
        );
    }
}
