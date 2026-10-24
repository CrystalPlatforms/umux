//! umux Core (Always-On device) — v1.7.5 phase 1 / issue #93.
//!
//! The daemon is the SINGLE holder of the sleep block: the Core flag is
//! daemon-owned state, persisted in the config dir and re-asserted on daemon
//! start with NO client call — the whole point is "every umux window closed".
//! Daemon stop (graceful or crash) releases the block by construction: on
//! macOS the assertion lives inside this process, and the kernel releases
//! every power assertion of a dying process. On Linux, closing the last
//! inhibitor fd releases the logind block, including on SIGKILL.
//!
//! - Persistence: `<config_dir>/storestation.core.json`, `{"enabled": bool}`.
//!   A missing or corrupt file reads as OFF (additive-safe, hand-editable).
//! - Backend (macOS): an IOKit power assertion —
//!   `PreventSystemSleep` named "umux Core", the `caffeinate -s` hold: no
//!   admin, the SCREEN may sleep/turn off, the machine may not — including
//!   lid close on AC power. macOS honors this assertion only on AC: on
//!   battery the block is not guaranteed, and the state carries the
//!   instruction string instead of pretending (story 142).
//! - Backend (Windows, phase 3 / issue #95): a dedicated daemon thread
//!   holding `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`
//!   — no admin, the SCREEN may sleep; the lid-close action is a power-plan
//!   setting the daemon cannot touch, so while ON the honest state carries
//!   the lid instruction (story 142).
//! - Backend (Linux, phase 4 / issue #96): a logind sleep inhibitor fd
//!   owned by the daemon; no display/idle or lid-switch inhibition. The
//!   lid action remains a system setting and is explained in status.
//!
//! umux Core Session (timed awake sessions, #111) slots into the SAME state
//! machine: the session is a bounded window (absolute `endsAt`, unix
//! seconds, max 24 h from `startedAt`) during which the daemon holds the
//! exact same platform assertion. One hold at a time — the session refuses
//! to start while Core is ON and `core.set(true)` is refused while a
//! session is live (the holds never stack, PRD decision); expiry drops the
//! existing `PowerAssertion` value, inheriting the crash-release guarantee
//! by construction. The ended record (`lastEndedAt`, `lastEndReason`) is
//! deliberately NOT persisted — it lives for the daemon's lifetime only.
//!
//! Unit tests below cover the pure parts (flag persistence, view shape);
//! the wire behavior is exercised in `storestation/tests/core.rs`.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The honest limit the status/Settings faces show when macOS runs on
/// battery: macOS honors the system-sleep block only on AC power, so on
/// battery the guarantee does not apply (the OS may refuse the assertion
/// outright, or hold it without honoring it — either way the user must
/// know, never silently).
const BATTERY_INSTRUCTION: &str = "Running on battery power: macOS honors the Always-On block only on AC power — plug the Mac in to keep it awake (including with the lid closed).";

/// Unsupported platforms refuse honestly instead of claiming a hold.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
const BACKEND_MISSING_INSTRUCTION: &str =
    "umux Core has no Always-On backend on this platform yet — support ships in a later v1.7.5 phase.";

/// The Windows lid caveat (story 142): the `SetThreadExecutionState` hold
/// stops IDLE sleep — but the lid-close action is a power-plan setting no
/// daemon can touch, so closing the lid may still sleep the machine per
/// the user's plan. While Core is ON this caveat IS the honest state (PO
/// decision 2026-10-04: always shown while ON; worded to stay true on
/// lidless desktops), never silence.
#[cfg(target_os = "windows")]
const LID_INSTRUCTION: &str = "umux Core is keeping this machine awake while idle. If this device has a closing lid: the lid-close action still follows your Windows power plan — to stay awake with the lid closed, set it to \"Do nothing\" in Power Options (Control Panel → \"Choose what closing the lid does\").";

/// The same caveat, worded for a live Core Session (the hold belongs to the
/// session until its end time, not to an indefinite Core).
#[cfg(target_os = "windows")]
const SESSION_LID_INSTRUCTION: &str = "umux Core Session is keeping this machine awake until the session's end time. If this device has a closing lid: the lid-close action still follows your Windows power plan — to stay awake with the lid closed, set it to \"Do nothing\" in Power Options (Control Panel → \"Choose what closing the lid does\").";

/// logind owns lid handling; a sleep inhibitor does not override the
/// lid-close policy (and umux never changes system configuration).
#[cfg(target_os = "linux")]
const LID_INSTRUCTION: &str = "umux Core is keeping this machine awake while idle. If this device has a closing lid: closing it may still suspend the machine under the system's logind lid policy. To stay awake with the lid closed, ask your administrator to configure that policy; umux does not change it.";

/// The same caveat, worded for a live Core Session.
#[cfg(target_os = "linux")]
const SESSION_LID_INSTRUCTION: &str = "umux Core Session is keeping this machine awake until the session's end time. If this device has a closing lid: closing it may still suspend the machine under the system's logind lid policy. To stay awake with the lid closed, ask your administrator to configure that policy; umux does not change it.";

/// The flag document: `<config_dir>/storestation.core.json`,
/// `{"enabled": bool, "session": {"startedAt": <unix s>, "endsAt": <unix s>} | null}`.
/// Daemon-owned state (the plan's durable decision) — deliberately NOT part
/// of the socket/pid cleanup, which only removes runtime leftovers: the
/// document must survive both a daemon restart AND a crash. Additive by
/// contract: a v1.7.5 reader takes only `enabled` from a new-format file; a
/// new reader sees "no session" in an old-format file; a corrupt file still
/// reads as OFF with no session.
fn flag_path(config_dir: &Path) -> PathBuf {
    config_dir.join("storestation.core.json")
}

/// A live session's absolute window (unix seconds — the durable decision:
/// never remaining time, so downtime degenerates honestly into "expired").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionWindow {
    pub started_at: u64,
    pub ends_at: u64,
}

/// The 24 h cap: a session's TOTAL span (`endsAt − startedAt`), start and
/// every extend alike (PRD technical pin).
pub const MAX_SESSION_SPAN_SECS: u64 = 24 * 60 * 60;

/// The pinned `lastEndReason` vocabulary (the status contract).
pub const END_REASON_EXPIRED: &str = "expired";
pub const END_REASON_STOPPED: &str = "stopped";

/// Why the daemon refused a Core Session op. The dispatch (server.rs) maps
/// these to the enumerated wire codes — one gate, no scattered refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRefusal {
    /// Start while Core is ON: Core already keeps the machine awake without
    /// a limit; the holds never stack (story 148).
    CoreOn,
    /// Start/extend would push the span past 24 h from `startedAt`.
    CapExceeded,
    /// Extend/stop with no live session.
    NoActiveSession,
    /// The flag document could not be persisted.
    Persist(String),
}

/// Why `core.set(true)` was refused: a live session owns the hold until it
/// ends (the exact mirror of the start refusal — one hold at a time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreSetError {
    SessionActive,
    Persist(String),
}

/// Read the persisted document. Tolerant by design: no file (first run, old
/// daemon's dir) and unparseable content (a hand-mangled file) both read as
/// OFF with no session — a broken byte can never turn the machine's sleep
/// block on. An old-format file (v1.7.5, `{"enabled": …}` only) reads with
/// no session; a new-format file read by a v1.7.5 daemon yields `enabled`
/// only (that reader ignores unknown keys by construction).
fn load_doc(config_dir: &Path) -> (bool, Option<SessionWindow>) {
    let Ok(text) = std::fs::read_to_string(flag_path(config_dir)) else {
        return (false, None);
    };
    let Some(value) = serde_json::from_str::<Value>(&text).ok() else {
        return (false, None);
    };
    let enabled = value.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let session = value.get("session").and_then(|s| {
        let started_at = s.get("startedAt").and_then(Value::as_u64)?;
        let ends_at = s.get("endsAt").and_then(Value::as_u64)?;
        Some(SessionWindow { started_at, ends_at })
    });
    (enabled, session)
}

fn save_doc(
    config_dir: &Path,
    enabled: bool,
    session: Option<SessionWindow>,
) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(&json!({
        "enabled": enabled,
        "session": session.map(|s| json!({
            "startedAt": s.started_at,
            "endsAt": s.ends_at,
        })),
    }))
    .expect("the flag document is always serializable");
    std::fs::write(flag_path(config_dir), body)
}

/// Wall-clock unix seconds. The only clock read in this module's runtime
/// paths — every state transition takes `now` as a parameter so tests (and
/// the restore path) drive time explicitly.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_secs()
}

/// The daemon-side Core state: the user's choice (`enabled`, persisted) and
/// the platform's live truth (`held`, best effort) plus, when the guarantee
/// is limited or refused, the instruction string — never silent (story 142).
/// The session (when live) owns the SAME single assertion slot: one hold at
/// a time, by construction.
pub struct CoreState {
    enabled: bool,
    assertion: Option<power::PowerAssertion>,
    instruction: Option<String>,
    session: Option<SessionWindow>,
    // The ended record — in-memory only, daemon-lifetime (the plan pins it
    // non-persisted; a fresh daemon starts with a clean end state).
    last_ended_at: Option<u64>,
    last_end_reason: Option<&'static str>,
}

impl CoreState {
    /// Daemon-start restore: read the document and re-assert whatever the
    /// user left running — Core ON, a live session, or nothing — BEFORE the
    /// serve loop starts, with NO client call. A session past its deadline
    /// is expired, never an infinite hold: the hold is not taken and the
    /// expiry is recorded (story 146).
    pub fn restore(config_dir: &Path) -> CoreState {
        CoreState::restore_with_clock(config_dir, unix_now())
    }

    /// [`restore`](Self::restore) with the clock driven by the caller —
    /// every state transition in this module takes `now` explicitly, and
    /// restore is no exception (the unit tests pin the expiry arithmetic).
    pub fn restore_with_clock(config_dir: &Path, now: u64) -> CoreState {
        let (enabled, session) = load_doc(config_dir);
        // A document claiming BOTH holds at once is unreachable through the
        // daemon (each side refuses the other); if a hand-edit produced it,
        // Core — the primary v1.7.5 state — wins and the session record is
        // discarded without an end record (it never ran).
        let session = if enabled { None } else { session };
        let mut state = CoreState {
            enabled: false,
            assertion: None,
            instruction: None,
            session: None,
            last_ended_at: None,
            last_end_reason: None,
        };
        if let Some(window) = session {
            if window.ends_at > now {
                state.session = Some(window);
                state.take_hold();
                if state.held() {
                    eprintln!(
                        "umux-storestation: Core Session resumed — system sleep is prevented for the remaining {}s.",
                        window.ends_at - now
                    );
                } else {
                    eprintln!(
                        "umux-storestation: Core Session resumed but the sleep block is NOT held: {}",
                        state.instruction.as_deref().unwrap_or("unknown reason")
                    );
                }
            } else {
                // Past deadline: expired during downtime. Record the end at
                // the deadline itself (deterministic across restarts) and
                // settle the document so the next reader sees it too.
                state.last_ended_at = Some(window.ends_at);
                state.last_end_reason = Some(END_REASON_EXPIRED);
                if let Err(e) = save_doc(config_dir, enabled, None) {
                    eprintln!("umux-storestation: could not persist the expired session: {e}");
                }
                eprintln!(
                    "umux-storestation: Core Session had already ended (deadline passed) — nothing asserted."
                );
            }
        }
        if enabled {
            state.apply(true);
            if state.held() {
                eprintln!(
                    "umux-storestation: umux Core resumed — system sleep is prevented."
                );
            } else {
                eprintln!(
                    "umux-storestation: umux Core resumed but the sleep block is NOT held: {}",
                    state.instruction.as_deref().unwrap_or("unknown reason")
                );
            }
        }
        state
    }

    /// Flip Core. The user's choice persists FIRST (an assertion hiccup must
    /// not undo it — the next daemon start retries), then the assertion is
    /// taken or dropped. Refused while a session is live (the mirror of the
    /// session-start refusal): the flip would otherwise clobber the
    /// session's assertion slot. `on = false` is allowed everywhere and —
    /// while a session is live — touches ONLY the Core choice, never the
    /// session's hold.
    pub fn set_enabled(&mut self, config_dir: &Path, on: bool) -> Result<(), CoreSetError> {
        if on && self.session.is_some() {
            return Err(CoreSetError::SessionActive);
        }
        save_doc(config_dir, on, self.session)
            .map_err(|e| CoreSetError::Persist(e.to_string()))?;
        self.apply(on);
        Ok(())
    }

    /// (Re)apply the choice. With a live session the hold is NOT touched:
    /// the session owns it until its own end (see [`Self::set_enabled`]).
    fn apply(&mut self, on: bool) {
        self.enabled = on;
        if self.session.is_some() {
            return;
        }
        if !on {
            self.assertion = None; // Drop releases the previous assertion
            self.instruction = None;
            return;
        }
        self.take_hold();
    }

    /// Take the platform hold for whoever owns it right now (Core or a
    /// live session). Drops any previous assertion first, then takes a
    /// fresh one and evaluates the honest caveats.
    fn take_hold(&mut self) {
        self.assertion = None;
        self.instruction = None;
        match power::assert_system_sleep() {
            Ok(assertion) => {
                self.assertion = Some(assertion);
                // The OS created the assertion, but macOS honors the
                // system-sleep block only on AC — on battery the caveat is
                // the honest state, never silence.
                if power_source_is_battery() == Some(true) {
                    self.instruction = Some(BATTERY_INSTRUCTION.to_string());
                }
                // Windows/Linux: idle sleep is held, but the lid-close
                // policy remains outside the daemon. Always show its caveat,
                // worded for the hold's owner.
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                {
                    self.instruction = Some(if self.session.is_some() {
                        SESSION_LID_INSTRUCTION.to_string()
                    } else {
                        LID_INSTRUCTION.to_string()
                    });
                }
            }
            Err(message) => {
                // The OS refused the assertion (on battery it may refuse
                // outright). Never silent: the state reports WHY the block
                // is not held.
                self.instruction = Some(if power_source_is_battery() == Some(true) {
                    BATTERY_INSTRUCTION.to_string()
                } else {
                    message
                });
            }
        }
    }

    /// Explicit release on the daemon's clean exit path. The OS releases
    /// anyway when the process dies — this is hygiene, not the guarantee.
    pub fn release(&mut self) {
        self.assertion = None;
    }

    /// The persisted user choice (vs [`held`](Self::held), the live truth).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The live truth: is a system-sleep assertion held right now?
    pub fn held(&self) -> bool {
        self.assertion.is_some()
    }

    /// Is a Core Session live right now? (Not whether its hold succeeded —
    /// the held/instruction truth carries that.)
    pub fn session_active(&self) -> bool {
        self.session.is_some()
    }

    /// Start a timed session: persist the absolute window FIRST (an
    /// assertion hiccup must not undo it — the next daemon start retries,
    /// exactly like the Core flag), then take the hold. `now` is the stamp
    /// `startedAt` gets; the deadline is `now + duration_secs`.
    pub fn start_session(
        &mut self,
        config_dir: &Path,
        duration_secs: u64,
        now: u64,
    ) -> Result<(), SessionRefusal> {
        if self.enabled {
            return Err(SessionRefusal::CoreOn);
        }
        if duration_secs > MAX_SESSION_SPAN_SECS {
            return Err(SessionRefusal::CapExceeded);
        }
        let window = SessionWindow {
            started_at: now,
            ends_at: now.saturating_add(duration_secs),
        };
        save_doc(config_dir, self.enabled, Some(window))
            .map_err(|e| SessionRefusal::Persist(e.to_string()))?;
        // A start over a live session RESTARTS the window: the new window's
        // cap runs from the new startedAt, and the superseded window leaves
        // no end record — the vocabulary records ends the user can see
        // (expiry/stop of THE visible session), and a superseded window
        // never shows up as ended. The hold is continuous: an
        // already-held assertion is kept, never dropped and retaken.
        self.session = Some(window);
        self.last_ended_at = None;
        self.last_end_reason = None;
        if self.assertion.is_none() {
            self.take_hold();
        }
        Ok(())
    }

    /// Extend the live session. The cap is the TOTAL span from `startedAt`
    /// (the deadline moves, the start never does); a refusal changes
    /// nothing. If the OS dropped the hold mid-session (a backend hiccup),
    /// extending re-asserts — never silent.
    pub fn extend_session(
        &mut self,
        config_dir: &Path,
        duration_secs: u64,
    ) -> Result<(), SessionRefusal> {
        let Some(window) = self.session else {
            return Err(SessionRefusal::NoActiveSession);
        };
        let ends_at = window.ends_at.saturating_add(duration_secs);
        if ends_at - window.started_at > MAX_SESSION_SPAN_SECS {
            return Err(SessionRefusal::CapExceeded);
        }
        // The deadline is absolute, so the extension is pure arithmetic on
        // the persisted window — `now` plays no part (dispatch has already
        // settled a due expiry before calling).
        let extended = SessionWindow {
            started_at: window.started_at,
            ends_at,
        };
        save_doc(config_dir, self.enabled, Some(extended))
            .map_err(|e| SessionRefusal::Persist(e.to_string()))?;
        self.session = Some(extended);
        if self.assertion.is_none() {
            self.take_hold();
        }
        Ok(())
    }

    /// End the live session early: drop the hold, record the end at `now`,
    /// persist the settled document.
    pub fn stop_session(&mut self, config_dir: &Path, now: u64) -> Result<(), SessionRefusal> {
        if self.session.is_none() {
            return Err(SessionRefusal::NoActiveSession);
        }
        self.end_session(config_dir, now, END_REASON_STOPPED)
            .map_err(SessionRefusal::Persist)
    }

    /// The deadline tick: when the live session's `endsAt` has passed, drop
    /// the hold with no client call and record the expiry at the deadline
    /// itself. Returns whether it expired.
    pub fn expire_if_due(&mut self, config_dir: &Path, now: u64) -> bool {
        let Some(window) = self.session else {
            return false;
        };
        if window.ends_at > now {
            return false;
        }
        match self.end_session(config_dir, window.ends_at, END_REASON_EXPIRED) {
            Ok(()) => true,
            Err(e) => {
                // The tick has no caller to answer: the state IS settled
                // (session gone, hold dropped) — only the document could
                // not be written. Say so on the daemon's stderr.
                eprintln!("umux-storestation: could not persist the session expiry: {e}");
                true
            }
        }
    }

    /// The shared ending: drop the hold, record the end, settle the
    /// document. Dropping the `PowerAssertion` value releases it on every
    /// platform (macOS Drop → IOPMAssertionRelease; Windows unblocks the
    /// hold thread; Linux closes the inhibitor fd) — the crash-release
    /// guarantee is inherited, never re-implemented.
    fn end_session(
        &mut self,
        config_dir: &Path,
        ended_at: u64,
        reason: &'static str,
    ) -> Result<(), String> {
        self.session = None;
        self.assertion = None;
        self.instruction = None;
        self.last_ended_at = Some(ended_at);
        self.last_end_reason = Some(reason);
        save_doc(config_dir, self.enabled, None).map_err(|e| e.to_string())
    }

    /// The status-view shape shared by `core.set`/`core.session.*` results
    /// and the `storestation.status` `core` object:
    /// `{enabled, held, instruction, session}` — the first three keys are
    /// the unchanged v1.7.5 contract, `session` joins additively (#111).
    pub fn view(&self) -> Value {
        json!({
            "enabled": self.enabled,
            "held": self.held(),
            "instruction": self.instruction,
            "session": self.session_view(),
        })
    }

    /// The pinned `core.session` shape: `{active, endsAt, remainingSecs,
    /// lastEndedAt, lastEndReason}`. Times are unix seconds; when no
    /// session is live `endsAt`/`remainingSecs` are null while the ended
    /// record (this daemon's lifetime) still travels.
    fn session_view(&self) -> Value {
        let (ends_at, remaining_secs) = match self.session {
            Some(window) => (
                json!(window.ends_at),
                json!(window.ends_at.saturating_sub(unix_now())),
            ),
            None => (Value::Null, Value::Null),
        };
        json!({
            "active": self.session.is_some(),
            "endsAt": ends_at,
            "remainingSecs": remaining_secs,
            "lastEndedAt": self.last_ended_at,
            "lastEndReason": self.last_end_reason,
        })
    }
}

/// Is the Mac drawing from a battery right now? Best effort: parse
/// `pmset -g ps` ("Now drawing from 'AC Power'" / "'Battery Power'").
/// `None` = cannot tell (command missing, output unknown) — no instruction
/// is claimed, because inventing a limit would be its own lie.
fn power_source_is_battery() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("pmset").args(["-g", "ps"]).output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout).to_lowercase();
        if text.contains("battery power") {
            Some(true)
        } else if text.contains("ac power") {
            Some(false)
        } else {
            None
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The per-platform assertion backend. macOS (this phase) talks IOKit
/// directly — two C functions and CFString construction, no new dependency.
#[cfg(target_os = "macos")]
mod power {
    use std::ffi::{c_char, c_void, CString};

    type CFStringRef = *const c_void;
    type IOPMAssertionID = u32;
    type IOReturn = i32;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            assertion_level: u32,
            assertion_name: CFStringRef,
            assertion_id: *mut IOPMAssertionID,
        ) -> IOReturn;
        fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: CFStringRef);
    }

    // kCFStringEncodingUTF8, kIOPMAssertionLevelOn — stable IOKit/CF constants.
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;
    // `caffeinate -s` semantics: the SYSTEM may not sleep — idle or lid
    // close on AC power. macOS honors this type only on AC; on battery it
    // refuses it (or ignores it), which the honest state reports.
    const ASSERTION_TYPE: &str = "PreventSystemSleep";
    const ASSERTION_NAME: &str = "umux Core";

    fn cf_string(value: &str) -> CFStringRef {
        let c = CString::new(value).expect("assertion type/name never contain interior NULs");
        // SAFETY: c is a valid NUL-terminated buffer; null alloc = the
        // default allocator. The returned CF object is released by the caller.
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8) }
    }

    /// A held system-sleep assertion. The kernel releases it when this
    /// value drops (explicit [`Drop`] on macOS) AND when the whole process
    /// dies — daemon stop releases the block by construction.
    pub struct PowerAssertion {
        id: u32,
    }

    impl Drop for PowerAssertion {
        fn drop(&mut self) {
            // SAFETY: self.id is an assertion id THIS process created and
            // has not released; a nonzero IOReturn here means the kernel
            // already released it — nothing to report, the guarantee holds.
            unsafe {
                IOPMAssertionRelease(self.id);
            }
        }
    }

    /// Take the system-sleep assertion (named "umux Core" — that name is
    /// what `pmset -g assertions` shows, the acceptance gate).
    pub fn assert_system_sleep() -> Result<PowerAssertion, String> {
        let assertion_type = cf_string(ASSERTION_TYPE);
        let assertion_name = cf_string(ASSERTION_NAME);
        let mut id: IOPMAssertionID = 0;
        // SAFETY: both CFStrings are live objects, the out-pointer is valid.
        let ret = unsafe {
            IOPMAssertionCreateWithName(
                assertion_type,
                K_IOPM_ASSERTION_LEVEL_ON,
                assertion_name,
                &mut id,
            )
        };
        // SAFETY: both were created by CFStringCreateWithCString above.
        unsafe {
            CFRelease(assertion_type);
            CFRelease(assertion_name);
        }
        if ret != 0 {
            return Err(format!("macOS refused the power assertion (IOReturn {ret})."));
        }
        Ok(PowerAssertion { id })
    }
}

/// Windows (v1.7.5 phase 3 / issue #95): a dedicated daemon thread holding
/// `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` — the hold
/// `powercfg /requests` shows: no admin, the SCREEN may sleep, the machine
/// may not drift into idle sleep. The hold is THREAD-LOCAL on Windows: it
/// lives until the thread clears it (the explicit clear below is hygiene)
/// or DIES — which is why the assertion owns its own thread whose lifetime
/// is the daemon's: daemon stop/crash kills the thread and Windows drops a
/// dead thread's execution state by itself.
#[cfg(target_os = "windows")]
mod power {
    use std::sync::mpsc;

    type ExecutionState = u32;

    // Stable Win32 constants (winuser.h): continuous mode + "the system is
    // required" — idle sleep is stopped, the display is NOT held awake.
    const ES_CONTINUOUS: ExecutionState = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: ExecutionState = 0x0000_0001;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(es_flags: ExecutionState) -> ExecutionState;
    }

    /// A held system-execution request. Dropping it ends the hold: the
    /// parked thread unblocks, clears its state and exits (the join in
    /// [`Drop`] is what a stuck hold thread would trip over). If the whole
    /// process dies first, the thread dies with it and Windows releases
    /// the hold anyway — the explicit clear is hygiene, not the guarantee.
    pub struct PowerAssertion {
        release: Option<mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    /// Take the hold on a dedicated thread and CONFIRM it before reporting
    /// success — a zero return is the documented failure value, so `held`
    /// is the OS's answer, never an assumption.
    pub fn assert_system_sleep() -> Result<PowerAssertion, String> {
        let (ack_tx, ack_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("umux-core-always-on".to_string())
            .spawn(move || {
                // SAFETY: SetThreadExecutionState takes no pointers and is
                // documented thread-local; the hold dies with this thread,
                // whichever way the thread ends.
                let previous =
                    unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
                if previous == 0 {
                    let _ = ack_tx.send(Err(
                        "Windows refused the Always-On hold (SetThreadExecutionState failed)."
                            .to_string(),
                    ));
                    return;
                }
                let _ = ack_tx.send(Ok(()));
                // Park until the assertion is dropped (the sender goes away,
                // recv errors) — or until the process dies, which Windows
                // treats the same for the hold.
                let _ = release_rx.recv();
                // SAFETY: as above — clears THIS thread's continuous state.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            })
            .map_err(|e| format!("could not spawn the Core hold thread: {e}"))?;
        match ack_rx.recv() {
            Ok(Ok(())) => Ok(PowerAssertion {
                release: Some(release_tx),
                thread: Some(thread),
            }),
            Ok(Err(message)) => Err(message),
            Err(_) => Err("the Core hold thread died before confirming the hold.".to_string()),
        }
    }

    impl Drop for PowerAssertion {
        fn drop(&mut self) {
            self.release = None; // unblocks the parked hold thread
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Linux (issue #96): logind's Inhibit returns a Unix fd. Its lifetime,
/// not the D-Bus connection's, owns the hold: OFF, graceful shutdown and
/// process death (including SIGKILL) close it. No helper process can outlive
/// the daemon and retain the inhibitor.
///
/// Only "sleep" is blocked: the display may blank/lock, and logind's lid
/// policy remains untouched. "idle" would affect the system idle action;
/// "handle-lid-switch" would take over the lid, both outside this backend.
/// See https://www.freedesktop.org/wiki/Software/systemd/inhibit/
#[cfg(target_os = "linux")]
mod power {
    use dbus::blocking::Connection;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::time::Duration;

    pub struct PowerAssertion {
        // OwnedFd's Drop closes the inhibitor; the kernel does so on crash.
        _inhibitor: OwnedFd,
    }

    pub fn assert_system_sleep() -> Result<PowerAssertion, String> {
        let connection = Connection::new_system().map_err(|e| {
            format!("Cannot connect to the Linux system bus: {e}. Run umux in your desktop user session with logind available.")
        })?;
        let proxy = connection.with_proxy(
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            Duration::from_secs(2),
        );
        let (inhibitor,): (OwnedFd,) = proxy.method_call(
            "org.freedesktop.login1.Manager",
            "Inhibit",
            ("sleep", "umux Core", "Always-On device", "block"),
        ).map_err(|e| {
            format!("logind refused the Always-On sleep inhibitor: {e}. Check that your desktop session permits sleep inhibition; the system lid-close policy remains unchanged.")
        })?;
        // The daemon launches terminal children. Never let an exec inherit
        // this fd and keep the block alive after the daemon dies.
        // SAFETY: inhibitor owns a live fd throughout both fcntl calls.
        let flags = unsafe { libc::fcntl(inhibitor.as_raw_fd(), libc::F_GETFD) };
        if flags < 0
            || unsafe {
                libc::fcntl(
                    inhibitor.as_raw_fd(),
                    libc::F_SETFD,
                    flags | libc::FD_CLOEXEC,
                )
            } < 0
        {
            return Err(format!(
                "Cannot protect the logind inhibitor fd from inheritance: {}.",
                std::io::Error::last_os_error()
            ));
        }
        Ok(PowerAssertion {
            _inhibitor: inhibitor,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Read;
        use std::os::unix::net::UnixStream;

        #[test]
        fn explicit_release_closes_the_fd_while_the_daemon_state_still_lives() {
            let (mut observer, inhibitor) = UnixStream::pair().unwrap();
            let mut state = super::super::CoreState {
                enabled: true,
                assertion: Some(PowerAssertion {
                    _inhibitor: inhibitor.into(),
                }),
                instruction: Some(super::super::LID_INSTRUCTION.to_owned()),
                session: None,
                last_ended_at: None,
                last_end_reason: None,
            };
            observer.set_nonblocking(true).unwrap();
            assert_eq!(
                observer.read(&mut [0]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            state.release();
            assert_eq!(observer.read(&mut [0]).unwrap(), 0, "release closed the fd");
            assert!(!state.held());
            assert!(state.enabled(), "shutdown must preserve the user's choice");
        }

        // Core Session (#112): the SAME state machine over the SAME fd
        // lifetime. A Core flip during a live session must NOT close the
        // inhibitor; the session's own end (stop or expiry) closes it.
        #[test]
        fn session_end_closes_the_fd_and_a_core_flip_does_not() {
            let (mut observer, inhibitor) = UnixStream::pair().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let now = super::super::unix_now();
            let mut state = super::super::CoreState {
                enabled: false,
                assertion: Some(PowerAssertion {
                    _inhibitor: inhibitor.into(),
                }),
                instruction: Some(super::super::SESSION_LID_INSTRUCTION.to_owned()),
                session: Some(super::super::SessionWindow {
                    started_at: now,
                    ends_at: now + 3600,
                }),
                last_ended_at: None,
                last_end_reason: None,
            };
            observer.set_nonblocking(true).unwrap();
            assert_eq!(
                observer.read(&mut [0]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "the session's inhibitor is live"
            );

            // The mirror refusal keeps the hold: Core ON is refused, the
            // fd stays open.
            assert_eq!(
                state.set_enabled(dir.path(), true),
                Err(super::super::CoreSetError::SessionActive)
            );
            assert_eq!(
                observer.read(&mut [0]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "the refused Core flip did not touch the session's fd"
            );

            // Core OFF during a session is allowed — and still must not
            // close the session's inhibitor.
            state.set_enabled(dir.path(), false).unwrap();
            assert_eq!(
                observer.read(&mut [0]).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "the Core flip did not touch the session's fd"
            );

            // The session's own end closes the fd — by dropping the SAME
            // PowerAssertion value (the crash-release guarantee inherited).
            state.stop_session(dir.path(), now + 10).unwrap();
            assert_eq!(observer.read(&mut [0]).unwrap(), 0, "stop closed the fd");
            assert_eq!(state.last_end_reason, Some(super::super::END_REASON_STOPPED));
        }

        // Expiry over a due session closes the fd with no client call.
        #[test]
        fn expiry_closes_the_fd() {
            let (mut observer, inhibitor) = UnixStream::pair().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let now = super::super::unix_now();
            let mut state = super::super::CoreState {
                enabled: false,
                assertion: Some(PowerAssertion {
                    _inhibitor: inhibitor.into(),
                }),
                instruction: Some(super::super::SESSION_LID_INSTRUCTION.to_owned()),
                session: Some(super::super::SessionWindow {
                    started_at: now,
                    ends_at: now.saturating_sub(1),
                }),
                last_ended_at: None,
                last_end_reason: None,
            };
            observer.set_nonblocking(true).unwrap();
            assert!(state.expire_if_due(dir.path(), now));
            assert_eq!(observer.read(&mut [0]).unwrap(), 0, "expiry closed the fd");
            assert_eq!(state.last_end_reason, Some(super::super::END_REASON_EXPIRED));
        }
    }
}

/// Platforms without a backend: refuse honestly —
/// `held: false` + the instruction string, never a fake OK.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod power {
    pub struct PowerAssertion {
        _private: (),
    }

    pub fn assert_system_sleep() -> Result<PowerAssertion, String> {
        Err(crate::core::BACKEND_MISSING_INSTRUCTION.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The flag file's name and location are an on-disk contract: the
    // daemon's own dir, NOT a subdirectory, alongside sock/pid/pids.
    #[test]
    fn flag_lives_in_the_config_dir() {
        assert_eq!(
            flag_path(Path::new("/tmp/umux")),
            PathBuf::from("/tmp/umux/storestation.core.json")
        );
    }

    // Default OFF: a config dir without the flag file (first run, a pre-Core
    // daemon's dir, a factory reset) restores enabled=false, held=false.
    #[test]
    fn missing_flag_reads_as_off() {
        let dir = tempfile::tempdir().unwrap();
        let state = CoreState::restore(dir.path());
        assert!(!state.enabled(), "no flag = Core off");
        assert!(!state.held(), "nothing asserted");
        let view = state.view();
        assert_eq!(view["enabled"], false);
        assert_eq!(view["held"], false);
        assert_eq!(view["instruction"], Value::Null, "nothing to instruct about");
    }

    // A corrupt flag file is OFF, never ON — a mangled byte must not be
    // able to turn the machine's sleep block on.
    #[test]
    fn corrupt_flag_reads_as_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("storestation.core.json"), b"{not json").unwrap();
        let state = CoreState::restore(dir.path());
        assert!(!state.enabled());
        assert!(!state.held());
    }

    // An explicit false persists as false across a restore.
    #[test]
    fn explicit_off_persists() {
        let dir = tempfile::tempdir().unwrap();
        save_doc(dir.path(), false, None).unwrap();
        let state = CoreState::restore(dir.path());
        assert!(!state.enabled());
    }

    // The view always carries all three contract keys (the wire shape the
    // status op and the core.set result share).
    #[test]
    fn view_carries_the_full_contract_shape() {
        let dir = tempfile::tempdir().unwrap();
        let state = CoreState::restore(dir.path());
        let view = state.view();
        for key in ["enabled", "held", "instruction"] {
            assert!(view.get(key).is_some(), "missing {key}: {view}");
        }
    }

    // macOS: the real backend — take the assertion, see it held, release it.
    // The `pmset -g assertions | grep -i umux` visibility itself is the
    // acceptance check run OUTSIDE the test process (HITL/AFK script), this
    // only proves the FFI path answers and the drop releases cleanly.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_assertion_is_taken_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore(dir.path());
        state.apply(true);
        // macOS honors the system-sleep block only on AC: on AC the
        // assertion is held with nothing to instruct; on battery the OS
        // may refuse it outright (held=false) — but NEVER silently.
        if power_source_is_battery() == Some(true) {
            assert!(
                state.view()["instruction"].as_str().is_some(),
                "battery limit is surfaced, never silent: {}",
                state.view()
            );
        } else {
            assert!(
                state.held(),
                "on AC the assertion is held: {:?}",
                state.instruction
            );
            assert_eq!(state.view()["instruction"], Value::Null);
        }
        state.release();
        assert!(!state.held(), "release drops the assertion");
    }

    // macOS: a restore over a LIVE session window re-asserts with no
    // client call — the backend-level half of the restart AC (#111/#146);
    // the wire level runs in tests/core_session.rs.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_restore_re_asserts_a_live_session_window() {
        let dir = tempfile::tempdir().unwrap();
        let now = unix_now();
        save_doc(
            dir.path(),
            false,
            Some(SessionWindow { started_at: now, ends_at: now + 3600 }),
        )
        .unwrap();
        let state = CoreState::restore_with_clock(dir.path(), now);
        assert!(state.session_active(), "the window is live: {:?}", state.last_end_reason);
        if power_source_is_battery() == Some(true) {
            assert!(
                state.view()["instruction"].as_str().is_some(),
                "the battery limit is surfaced, never silent: {:?}",
                state.instruction
            );
        } else {
            assert!(state.held(), "on AC the fresh daemon re-asserted: {:?}", state.instruction);
        }
    }

    // OFF clears a held state: no assertion, no stale instruction. (On
    // battery the ON side may already be refused — OFF must still land at
    // the same clean state.)
    #[cfg(target_os = "macos")]
    #[test]
    fn toggling_off_releases_the_assertion() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore(dir.path());
        state.apply(true);
        state.apply(false);
        assert!(!state.held());
        assert_eq!(state.view()["instruction"], Value::Null);
    }

    // ---- Windows (v1.7.5 phase 3 / issue #95) ----
    //
    // State-before-RED assumptions (#95):
    // - `SetThreadExecutionState` is THREAD-LOCAL on Windows: the hold
    //   lives until the thread clears it or dies, so the backend runs a
    //   dedicated daemon thread; the thread dying with the process
    //   releases the hold ("the assertion thread's lifetime is the
    //   daemon's" — daemon stop/crash releases by construction).
    // - A zero return = the OS refused the hold; nonzero = the previous
    //   state (success). `held` becomes true only AFTER that confirmation
    //   — the honest state, never an assumption.
    // - Windows honors the hold on battery too (it only drains the
    //   battery); the PRD bans battery intelligence, so the battery caveat
    //   stays macOS-only.
    // - While Core is ON the LID caveat is the honest Windows state (PO
    //   decision 2026-10-04: always shown when ON — the lid-close action
    //   is a power-plan setting no daemon can touch), so the instruction
    //   is non-empty even when held — never silent (story 142).
    // - Tests assert the contract (non-empty instruction), not the exact
    //   UX wording. The wire shape is platform-agnostic (`CoreState::view`);
    //   the unix-gated wire tests cover it on macOS, these cover the
    //   Windows backend itself.

    // The real backend, live: Core ON holds the idle-sleep block (no
    // admin) and the lid caveat travels with it. A hang in `release()`
    // (the join in Drop never unblocking) fails this test by timing out —
    // which is the observable for "the release path completes".
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_core_on_holds_sleep_and_carries_the_lid_caveat() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore(dir.path());
        state.apply(true);
        assert!(
            state.held(),
            "Windows holds the idle-sleep block: {:?}",
            state.instruction
        );
        let view = state.view();
        assert_eq!(view["held"], true);
        let instruction = view["instruction"]
            .as_str()
            .expect("the lid caveat is the honest Windows state, never silence");
        assert!(!instruction.is_empty(), "an instruction is never empty");
        state.release();
        assert!(!state.held(), "release drops the hold");
        // Re-assert after a release: the previous hold's thread really
        // finished (a leaked or stuck thread would show up here).
        state.apply(true);
        assert!(state.held(), "re-asserting after a release works");
        state.release();
    }

    // OFF lands clean: hold dropped, no stale instruction (story 143 —
    // and the guard against a leaked hold thread outliving the toggle).
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_toggling_off_releases_and_clears_the_instruction() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore(dir.path());
        state.apply(true);
        state.apply(false);
        assert!(!state.held());
        assert_eq!(state.view()["instruction"], Value::Null);
    }

    // Daemon-start re-assert on Windows: a persisted ON flag comes back
    // held with NO client call — the "every umux window closed" contract,
    // live on the Windows backend.

    // ---- umux Core Session (#111) — unit tests over the pure machine ----
    //
    // Assumptions (state-before-RED, #111): times are DRIVEN (explicit
    // `now`), the doc is the only side effect, and the hold paths run the
    // real backend where one exists (harmless in a test process — the
    // existing Core tests do the same). The held/instruction side is
    // asserted only in the platform-gated tests below.

    fn doc_of(dir: &Path) -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(flag_path(dir)).expect("the flag doc exists"),
        )
        .expect("the flag doc is JSON")
    }

    fn state_with_session(started_at: u64, ends_at: u64) -> CoreState {
        CoreState {
            enabled: false,
            assertion: None,
            instruction: None,
            session: Some(SessionWindow { started_at, ends_at }),
            last_ended_at: None,
            last_end_reason: None,
        }
    }

    // The doc round-trips both fields; an OLD-format doc (v1.7.5) reads as
    // "no session"; the v1.7.5 reader expression applied to a NEW-format
    // doc still yields `enabled` only (the additive contract, both ways).
    #[test]
    fn the_session_doc_round_trips_and_degrades_honestly_both_ways() {
        let dir = tempfile::tempdir().unwrap();
        let window = SessionWindow { started_at: 1000, ends_at: 4600 };
        save_doc(dir.path(), false, Some(window)).unwrap();
        assert_eq!(load_doc(dir.path()), (false, Some(window)));

        // Old daemon wrote only `enabled`: the new reader sees no session.
        std::fs::write(flag_path(dir.path()), b"{\"enabled\": true}").unwrap();
        assert_eq!(load_doc(dir.path()), (true, None));

        // The v1.7.5 reader expression over a NEW-format doc: `enabled`
        // only — the session key is ignored by construction.
        save_doc(dir.path(), true, Some(window)).unwrap();
        let text = std::fs::read_to_string(flag_path(dir.path())).unwrap();
        let enabled = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("enabled").and_then(Value::as_bool))
            .unwrap_or(false);
        assert!(enabled, "the old reader still finds enabled: {text}");
    }

    // A corrupt doc reads OFF with NO session — a broken byte can never
    // turn a hold on (the v1.7.5 rule, extended to the session).
    #[test]
    fn a_corrupt_doc_reads_off_with_no_session() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(flag_path(dir.path()), b"{\"enabled\": true, \"session\":").unwrap();
        assert_eq!(load_doc(dir.path()), (false, None));
    }

    // story 148: start while Core is ON is refused before anything is
    // touched — no window, no doc write.
    #[test]
    fn start_refuses_while_core_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore(dir.path());
        save_doc(dir.path(), true, None).unwrap();
        state.enabled = true;
        assert_eq!(
            state.start_session(dir.path(), 60, 1000),
            Err(SessionRefusal::CoreOn)
        );
        assert!(!state.session_active());
        assert_eq!(load_doc(dir.path()), (true, None), "the doc is untouched");
    }

    // A successful start persists the absolute window FIRST and clears a
    // stale end record (the end state belongs to the session the user
    // sees; story 145).
    #[test]
    fn start_persists_the_window_and_clears_stale_end_records() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore(dir.path());
        state.last_ended_at = Some(500);
        state.last_end_reason = Some(END_REASON_STOPPED);
        state.start_session(dir.path(), 60, 1000).unwrap();
        assert_eq!(
            state.session,
            Some(SessionWindow { started_at: 1000, ends_at: 1060 })
        );
        assert_eq!(state.last_ended_at, None, "the stale end record is gone");
        assert_eq!(state.last_end_reason, None);
        assert_eq!(
            load_doc(dir.path()).1,
            Some(SessionWindow { started_at: 1000, ends_at: 1060 })
        );
    }

    // The 24 h cap is the TOTAL span from startedAt: a refused extend (or
    // start) changes nothing — neither the state nor the doc.
    #[test]
    fn extend_refuses_past_the_cap_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_with_session(0, 3600);
        save_doc(dir.path(), false, state.session).unwrap();
        assert_eq!(
            state.extend_session(dir.path(), 86400),
            Err(SessionRefusal::CapExceeded)
        );
        assert_eq!(
            state.session,
            Some(SessionWindow { started_at: 0, ends_at: 3600 }),
            "the window is untouched"
        );
        assert_eq!(
            load_doc(dir.path()).1,
            Some(SessionWindow { started_at: 0, ends_at: 3600 }),
            "the doc is untouched"
        );

        // Exactly 24 h is the cap, inclusive.
        let mut state = state_with_session(0, 3600);
        state.extend_session(dir.path(), 86400 - 3600).unwrap();
        assert_eq!(
            state.session,
            Some(SessionWindow { started_at: 0, ends_at: 86400 })
        );
    }

    // Extend moves ONLY the deadline; startedAt is the cap's anchor.
    #[test]
    fn extend_moves_only_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_with_session(100, 3600);
        state.extend_session(dir.path(), 1800).unwrap();
        assert_eq!(
            state.session,
            Some(SessionWindow { started_at: 100, ends_at: 5400 })
        );
    }

    // Stop records the end at the stop moment and settles the doc.
    #[test]
    fn stop_records_the_end_and_settles_the_doc() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_with_session(1000, 4600);
        state.stop_session(dir.path(), 2000).unwrap();
        assert!(!state.session_active());
        assert!(!state.held(), "the hold is dropped");
        assert_eq!(state.last_ended_at, Some(2000));
        assert_eq!(state.last_end_reason, Some(END_REASON_STOPPED));
        assert_eq!(load_doc(dir.path()), (false, None), "the doc is settled");
    }

    // Extend/stop with nothing live are refused, not silently ignored.
    #[test]
    fn extend_and_stop_without_a_session_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore(dir.path());
        assert_eq!(
            state.extend_session(dir.path(), 60),
            Err(SessionRefusal::NoActiveSession)
        );
        assert_eq!(
            state.stop_session(dir.path(), 1000),
            Err(SessionRefusal::NoActiveSession)
        );
    }

    // The deadline tick expires AT the deadline (deterministic across
    // restarts), and a settled session never expires twice.
    #[test]
    fn expire_records_at_the_deadline_and_never_twice() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_with_session(0, 900);
        assert!(!state.expire_if_due(dir.path(), 899), "not yet due");
        assert!(state.session_active());
        assert!(state.expire_if_due(dir.path(), 900), "due at the deadline");
        assert_eq!(state.last_ended_at, Some(900), "recorded at the deadline");
        assert_eq!(state.last_end_reason, Some(END_REASON_EXPIRED));
        assert_eq!(load_doc(dir.path()), (false, None));
        assert!(!state.expire_if_due(dir.path(), 901), "nothing left to expire");
    }

    // Restore with a past deadline: expired at the deadline, doc settled,
    // nothing held — never an infinite hold (story 146).
    #[test]
    fn restore_records_an_expired_deadline_and_settles_the_doc() {
        let dir = tempfile::tempdir().unwrap();
        save_doc(dir.path(), false, Some(SessionWindow { started_at: 100, ends_at: 200 }))
            .unwrap();
        let state = CoreState::restore_with_clock(dir.path(), 300);
        assert!(!state.session_active());
        assert!(!state.held());
        assert_eq!(state.last_ended_at, Some(200));
        assert_eq!(state.last_end_reason, Some(END_REASON_EXPIRED));
        assert_eq!(load_doc(dir.path()), (false, None), "the doc is settled");
    }

    // A doc claiming BOTH holds is unreachable through the daemon; if a
    // hand-edit produces one, Core wins and the session record is dropped
    // without an end record (it never ran).
    #[test]
    fn restore_prefers_core_when_the_doc_claims_both_holds() {
        let dir = tempfile::tempdir().unwrap();
        save_doc(dir.path(), true, Some(SessionWindow { started_at: 100, ends_at: 100_000 }))
            .unwrap();
        let state = CoreState::restore_with_clock(dir.path(), 200);
        assert!(state.enabled(), "Core wins");
        assert!(!state.session_active(), "the session record is discarded");
        assert_eq!(state.last_ended_at, None, "no end record for a session that never ran");
        assert_eq!(state.last_end_reason, None);
    }

    // The mirror refusal: core.set(true) while a session is live is
    // refused before anything is persisted; core.set(false) during a
    // session flips ONLY the choice — the session's window survives in
    // both the state and the doc.
    #[test]
    fn core_set_refusals_and_off_during_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_with_session(1000, 100_000);
        save_doc(dir.path(), false, state.session).unwrap();
        assert_eq!(
            state.set_enabled(dir.path(), true),
            Err(CoreSetError::SessionActive)
        );
        assert!(!state.enabled(), "the choice is unchanged");
        assert!(state.session_active(), "the session is unchanged");
        assert_eq!(load_doc(dir.path()).1.is_some(), true, "the doc is unchanged");

        state.set_enabled(dir.path(), false).unwrap();
        assert!(!state.enabled());
        assert!(state.session_active(), "the session survived the Core off");
        // (The hold itself is untouched by construction — apply returns
        // early with a live session; the fd/assertion proof is pinned by
        // the platform-gated backend tests, which can hold real values.)
        assert_eq!(load_doc(dir.path()).1.is_some(), true, "the window persisted");
    }

    // The status view: the pinned session key set rides beside the
    // unchanged v1.7.5 core contract keys, with the right null semantics.
    #[test]
    fn the_view_carries_the_pinned_session_shape() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore_with_clock(dir.path(), 1000);
        let inactive = state.view()["session"].clone();
        assert_eq!(inactive["active"], false);
        assert_eq!(inactive["endsAt"], Value::Null);
        assert_eq!(inactive["remainingSecs"], Value::Null);
        assert_eq!(inactive["lastEndedAt"], Value::Null);
        assert_eq!(inactive["lastEndReason"], Value::Null);

        let now = unix_now();
        state.session = Some(SessionWindow { started_at: now, ends_at: now + 600 });
        let active = state.view()["session"].clone();
        assert_eq!(active["active"], true);
        assert_eq!(active["endsAt"], now + 600);
        assert_eq!(active["remainingSecs"], 600, "counting down from the clock");
    }


    // ---- Windows session backend (#112): the SAME state machine, the
    // SetThreadExecutionState hold. The session owns WHEN the untouched
    // v1.7.5 backend is taken and dropped — nothing else.
    //
    // State-before-RED assumptions (#112): the hold is thread-local, the
    // session's stop/expiry ends the hold thread cleanly (a stuck join
    // would hang this test), and the honest caveat is worded for the
    // session ("Core Session"), not for Core.

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_session_holds_stops_and_carries_the_session_lid_caveat() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = CoreState::restore_with_clock(dir.path(), unix_now());
        state.start_session(dir.path(), 3600, unix_now()).unwrap();
        assert!(
            state.held(),
            "the session holds the idle-sleep block: {:?}",
            state.instruction
        );
        let caveat = state
            .view()["instruction"]
            .as_str()
            .expect("the session caveat is the honest Windows state, never silence")
            .to_string();
        assert!(!caveat.is_empty());
        assert!(
            caveat.contains("Core Session"),
            "the caveat is worded for the session: {caveat}"
        );

        state.stop_session(dir.path(), unix_now()).unwrap();
        assert!(!state.held(), "stop releases the hold");
        assert_eq!(state.view()["instruction"], Value::Null);

        // The previous hold's thread really finished — re-asserting works.
        state.start_session(dir.path(), 60, unix_now()).unwrap();
        assert!(state.held(), "a session after a stop holds again");
        state.stop_session(dir.path(), unix_now()).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_session_expiry_releases_the_hold() {
        let dir = tempfile::tempdir().unwrap();
        let now = unix_now();
        let mut state = state_with_session(now, now.saturating_sub(1)); // already due
        state.take_hold();
        assert!(state.held(), "the hold is taken: {:?}", state.instruction);
        assert!(state.expire_if_due(dir.path(), now), "the tick expires it");
        assert!(!state.held(), "expiry released the hold");
        assert_eq!(state.last_end_reason, Some(END_REASON_EXPIRED));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_restore_re_asserts_from_the_persisted_flag() {
        let dir = tempfile::tempdir().unwrap();
        save_doc(dir.path(), true, None).unwrap();
        let state = CoreState::restore(dir.path());
        assert!(state.enabled());
        assert!(
            state.held(),
            "the fresh daemon re-asserted by itself: {:?}",
            state.instruction
        );
        assert!(
            state.view()["instruction"].as_str().is_some(),
            "the lid caveat still travels while ON: {}",
            state.view()
        );
    }
}
