pub mod pty_service;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use pty_service::{PtyHandle, PtyService};

/// One chunk of PTY output, ferried to the renderer over the `pty_output` event.
/// `data` is raw bytes serialized as a JSON array of numbers; the frontend
/// rebuilds a `Uint8Array` from it.
#[derive(Serialize, Clone)]
struct PtyOutputPayload {
    id: u32,
    data: Vec<u8>,
}

#[tauri::command]
fn pty_open(app: AppHandle, state: State<'_, Mutex<PtyService>>) -> Result<u32, String> {
    // Default to the user's $SHELL, falling back to /bin/sh.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    let (handle, rx) = {
        let mut svc = state.lock().map_err(|e| e.to_string())?;
        svc.open(&shell, cwd, 80, 24).map_err(|e| e.to_string())?
    };
    let id = handle.id;

    // Drain the PTY's output channel on its own thread and forward each chunk
    // to the renderer. The channel disconnects (loop ends) when the PTY closes.
    let emit_app = app.clone();
    std::thread::spawn(move || {
        while let Ok(bytes) = rx.recv() {
            let _ = emit_app.emit("pty_output", PtyOutputPayload { id, data: bytes });
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(PtyService::new()))
        .invoke_handler(tauri::generate_handler![
            pty_open,
            pty_write,
            pty_resize,
            pty_close,
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
