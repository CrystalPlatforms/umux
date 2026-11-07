// gateway_service — the app side of the socket gateway (#124, v1.8.0).
//
// The ownership rule ("whoever is alive", PO 2026-10-27): one thread keeps
// the app on the socket whenever the daemon is NOT — probe, bind if free,
// serve until released, clean up, probe again. With Storestation ON the
// loop never binds (the daemon owns the socket; the app stays a client).
// The Storestation toggle releases the bind BEFORE spawning the daemon
// (suspend_for_daemon) and lets the loop back after a disable (resume) —
// that is the whole v1.8.0 handover, nothing more.
//
// The gateway is READ-ONLY (#124 scope): it serves `storestation.status`
// (app variant) and `workspaces.list`. Live state comes FROM THE FRONTEND —
// this module is the bridge: a `workspaces.list` request becomes one
// `gateway_live_state_request` event, the UI answers through the
// `gateway_live_state_respond` invoke, and the gateway forwards the result.
// The UI stays the single owner of live state; the gateway never caches it
// and never touches the store files (the saved section rides along from the
// read-only store load in the respond command).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use umux_storestation::gateway::{self, GatewayBridge};
use umux_storestation::protocol::{codes, ErrorObj};

/// How often the ownership loop re-probes while it is not serving.
const IDLE_TICK: Duration = Duration::from_millis(500);
/// How long a probe-refused loop waits before probing again (something
/// else serves — usually the daemon; re-checking twice a second would be
/// busy-work).
const NOT_SERVING_TICK: Duration = Duration::from_millis(2000);
/// The frontend answer budget — well inside the protocol's 10 s request
/// budget, so the CLI's caller never sees the transport timeout first.
const FRONTEND_ANSWER_BUDGET: Duration = Duration::from_secs(5);

/// The app-wide gateway state. A global (OnceLock) because its callers span
/// both worlds the managed-state pattern serves awkwardly: free functions
/// like `spawn_daemon_if_absent` (no AppHandle) and the exit hook. The
/// service carries no store or connection state — only three flags — so a
/// global cannot leak state across instances the way a connection would.
pub struct GatewayService {
    /// Mirror of `settings.storestation.daemon_enabled` (the RouterDriver
    /// flag's twin, kept in step at the same call sites): while the
    /// Storestation toggle is ON the loop must NEVER grab the socket from
    /// the daemon.
    daemon_enabled: AtomicBool,
    /// The enable window (and the app-exit path): release the bind NOW and
    /// stay off the socket until [`GatewayService::resume`].
    suspended: AtomicBool,
    /// One-shot signal that unblocks a RUNNING serve loop — `serve` polls
    /// it between accepts and returns (its exit path cleans socket/pid).
    release_requested: AtomicBool,
}

static SERVICE: OnceLock<GatewayService> = OnceLock::new();

impl GatewayService {
    pub fn global() -> &'static GatewayService {
        SERVICE.get_or_init(|| GatewayService {
            daemon_enabled: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
            release_requested: AtomicBool::new(false),
        })
    }

    /// Keep the mirror in step with the Storestation toggle (same call
    /// sites as `RouterDriver::set_daemon_enabled`).
    pub fn set_daemon_enabled(&self, on: bool) {
        self.daemon_enabled.store(on, Ordering::SeqCst);
    }

    /// The Storestation-enable window (and app exit): stop serving now and
    /// do not re-bind until [`GatewayService::resume`]. Unblocks a running
    /// serve loop within one accept tick.
    pub fn suspend_for_daemon(&self) {
        self.suspended.store(true, Ordering::SeqCst);
        self.release_requested.store(true, Ordering::SeqCst);
    }

    /// Lift the suspension — the loop may bind again (the daemon is gone:
    /// a disable, or the enable flow failed and the app owes the user its
    /// live reads).
    pub fn resume(&self) {
        self.suspended.store(false, Ordering::SeqCst);
    }

    /// The clean-exit path: release the socket AND keep the loop from
    /// re-binding while the process winds down.
    pub fn stop_for_exit(&self) {
        self.suspend_for_daemon();
    }

    /// Spawn the ownership loop — ONCE, at app setup. Every bind/serve
    /// cycle runs on this thread; a bind is a daemon-absent moment, and a
    /// serve ends only through release_requested or listener death.
    pub fn spawn(app: AppHandle) {
        let service = Self::global();
        // The executor role (#126) reaches the window through this handle.
        let _ = APP_HANDLE.set(app.clone());
        let spawned = std::thread::Builder::new()
            .name("umux-gateway".into())
            .spawn(move || {
                let dir = store_core::paths::config_dir();
                let bridge: Arc<dyn GatewayBridge> = Arc::new(FrontendBridge {
                    app: app.clone(),
                });
                loop {
                    // The daemon owns the socket's job while Storestation is
                    // ON, and nobody binds during the suspend window — the
                    // loop just waits.
                    if service.daemon_enabled.load(Ordering::SeqCst)
                        || service.suspended.load(Ordering::SeqCst)
                    {
                        // Daemon crash while ON (#126): the app detects the
                        // dead client connection and takes the socket over
                        // (in-process session fallback is the driver's own
                        // dead-connection rule). NO automatic hand-back —
                        // the daemon returns only through the explicit ON
                        // flow, so clearing the MIRROR (never the setting)
                        // is the whole takeover.
                        if service.daemon_enabled.load(Ordering::SeqCst)
                            && !service.suspended.load(Ordering::SeqCst)
                            && daemon_client_dead(&app)
                        {
                            log::warn!(
                                "umux gateway: the daemon connection died while Storestation is ON — taking the socket over"
                            );
                            service.set_daemon_enabled(false);
                            continue; // bind on this same tick
                        }
                        std::thread::sleep(IDLE_TICK);
                        continue;
                    }
                    match gateway::prepare(
                        &dir,
                        Arc::clone(&bridge),
                        env!("CARGO_PKG_VERSION"),
                    ) {
                        Ok(prepared) => {
                            log::info!(
                                "umux gateway: serving the local socket (no daemon answered)"
                            );
                            gateway::serve(prepared, || {
                                service.release_requested.load(Ordering::SeqCst)
                            });
                            service.release_requested.store(false, Ordering::SeqCst);
                            log::info!("umux gateway: socket released");
                        }
                        Err(gateway::GatewayPrepareError::AlreadyRunning { server }) => {
                            // The daemon (or another app instance) owns the
                            // socket — stay a client, re-check periodically.
                            let _ = server;
                            std::thread::sleep(NOT_SERVING_TICK);
                        }
                        Err(gateway::GatewayPrepareError::Io(e)) => {
                            log::warn!("umux gateway: could not bind: {e}");
                            std::thread::sleep(NOT_SERVING_TICK);
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("umux gateway: could not spawn the ownership loop: {e}");
        }
    }
}

// --- the live-state bridge (#124) ---------------------------------------------
//
// request → the UI answers. One pending slot per request id; the respond
// command below resolves it. The UI holds the only live state — this bridge
// caches nothing and answers nothing on its own.

fn pending() -> &'static Mutex<HashMap<u64, mpsc::Sender<Value>>> {
    static PENDING: OnceLock<Mutex<HashMap<u64, mpsc::Sender<Value>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// The app handle, parked once at spawn so the EXECUTOR role (#126) can
/// reach the window from anywhere (the daemon driver's lazy connect has no
/// AppHandle in scope). None before setup finishes — the honest "the window
/// is not up yet".
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

struct FrontendBridge {
    app: AppHandle,
}

impl GatewayBridge for FrontendBridge {
    fn workspaces_list(&self) -> Result<Value, ErrorObj> {
        let doc = frontend_round_trip(
            &self.app,
            "gateway_live_state_request",
            json!({}),
            "live-state read",
        )?;
        // The respond command assembles {live, saved}; nothing to unwrap.
        Ok(doc)
    }

    fn call(&self, op: &str, params: &Value) -> Result<Value, ErrorObj> {
        let payload = frontend_round_trip(
            &self.app,
            "gateway_op_request",
            json!({ "op": op, "params": params }),
            op,
        )?;
        unwrap_op_answer(op, payload)
    }
}

/// The executor-side op execution (#126): the same bridge the app gateway
/// serves, driven by the DAEMON's forwarded requests. Uses the parked app
/// handle — an app connected to the daemon executes app-domain ops for the
/// whole catalog this way.
pub fn execute_frontend_op(op: &str, params: &Value) -> Result<Value, ErrorObj> {
    match APP_HANDLE.get() {
        Some(app) => {
            let payload = frontend_round_trip(
                app,
                "gateway_op_request",
                json!({ "op": op, "params": params }),
                op,
            )?;
            unwrap_op_answer(op, payload)
        }
        None => Err(ErrorObj::new(
            codes::IO_ERROR,
            "the umux window is not up yet to execute the command",
            vec!["try again once the umux window has finished loading".into()],
        )),
    }
}

/// The app-side executor callback handed to the daemon driver (#126).
pub fn executor_handler() -> umux_storestation::client::ExecutorHandler {
    Arc::new(|op: &str, params: &Value| execute_frontend_op(op, params))
}

/// The frontend answers `{ok:true, result}|{ok:false, error}`; the error
/// object is forwarded verbatim to the requesting CLI client.
fn unwrap_op_answer(op: &str, payload: Value) -> Result<Value, ErrorObj> {
    match payload.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(payload.get("result").cloned().unwrap_or(Value::Null)),
        _ => {
            let default = ErrorObj::new(
                codes::IO_ERROR,
                format!("the umux window answered \"{op}\" without a result"),
                vec![],
            );
            Err(payload
                .get("error")
                .and_then(|e| serde_json::from_value(e.clone()).ok())
                .unwrap_or(default))
        }
    }
}

/// One request → the UI answers → response. The shared shape of both bridge
/// directions (#124 read, #125 mutate, #126 executor): mint a request id,
/// park its channel in the pending map, emit ONE event, wait inside the
/// budget, clean up on any failure. `what` names the op in the timeout
/// error so a wedged window tells the agent WHAT did not answer.
fn frontend_round_trip(
    app: &AppHandle,
    event: &str,
    extra: Value,
    what: &str,
) -> Result<Value, ErrorObj> {
    let id = NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel::<Value>();
    pending().lock().expect("gateway pending lock").insert(id, tx);
    let mut payload = json!({ "id": id });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            payload[k] = v.clone();
        }
    }
    let emit = app.emit(event, payload);
    if let Err(e) = emit {
        pending()
            .lock()
            .expect("gateway pending lock")
            .remove(&id);
        return Err(ErrorObj::new(
            codes::IO_ERROR,
            format!("could not ask the umux window for {what}: {e}"),
            vec![],
        ));
    }
    match rx.recv_timeout(FRONTEND_ANSWER_BUDGET) {
        Ok(doc) => Ok(doc),
        Err(_) => {
            pending()
                .lock()
                .expect("gateway pending lock")
                .remove(&id);
            Err(ErrorObj::new(
                codes::IO_ERROR,
                format!("the umux window did not answer {what} within 5 s"),
                vec!["try again once the umux window has finished loading".into()],
            ))
        }
    }
}

/// The UI's answer to a live-state read. Assembles the gateway's
/// `workspaces.list` document: `live` from the UI's current state (the
/// workspaces/groups/order it persists, plus the per-panel agent states)
/// and `saved` from the read-only store load — the same store_core read the
/// CLI does offline, so the two sections always share one shape.
#[tauri::command]
pub fn gateway_live_state_respond(id: u64, state: Value, agent_states: Value) {
    let saved = {
        let store = store_core::workspace_store::WorkspaceStore::new(
            store_core::paths::config_path(),
        );
        let text = store_core::workspace_store::serialize_config(&store.load());
        serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)
    };
    let live = json!({
        "workspaces": state.get("workspaces").cloned().unwrap_or(Value::Null),
        "groups": state.get("groups").cloned().unwrap_or(Value::Null),
        "order": state.get("order").cloned().unwrap_or(Value::Null),
        "agentStates": agent_states,
    });
    let doc = json!({ "live": live, "saved": saved });
    let route = pending()
        .lock()
        .expect("gateway pending lock")
        .remove(&id);
    if let Some(tx) = route {
        let _ = tx.send(doc);
    }
}

/// The UI's answer to a live OP request (#125). `payload` is the whole
/// answer document — `{ok:true, result:…}` or `{ok:false, error:{…}}` —
/// forwarded verbatim; the bridge unwraps it (and converts a malformed
/// answer into the honest ioError instead of guessing).
#[tauri::command]
pub fn gateway_op_respond(id: u64, payload: Value) {
    let route = pending()
        .lock()
        .expect("gateway pending lock")
        .remove(&id);
    if let Some(tx) = route {
        let _ = tx.send(payload);
    }
}

/// Whether the app's daemon client connection is known-dead (#126 crash
/// takeover). Only a LIVE-THEN-DIED connection counts: no driver yet is the
/// lazy connect's own state (it owns its retries), not a crash.
fn daemon_client_dead(app: &AppHandle) -> bool {
    use tauri::Manager;
    app.try_state::<crate::session_driver::RouterDriver>()
        .map(|driver| driver.daemon_connection_dead())
        .unwrap_or(false)
}
