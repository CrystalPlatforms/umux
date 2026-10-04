//! umux Core (Always-On device) — v1.7.5 phase 1 / issue #93.
//!
//! The daemon is the SINGLE holder of the sleep block: the Core flag is
//! daemon-owned state, persisted in the config dir and re-asserted on daemon
//! start with NO client call — the whole point is "every umux window closed".
//! Daemon stop (graceful or crash) releases the block by construction: on
//! macOS the assertion lives inside this process, and the kernel releases
//! every power assertion of a dying process.
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
//! - Linux (plan phase 4): honest `held: false` + instruction, never a
//!   silent lie.
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

/// The instruction for platforms whose backend ships in a later phase
/// (Linux session inhibit) — the op answers truthfully instead of
/// pretending. Only the Linux-bound stub reaches for it (macOS and
/// Windows carry real backends).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
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

/// The flag file: `<config_dir>/storestation.core.json`. Daemon-owned state
/// (the plan's durable decision) — deliberately NOT part of the socket/pid
/// cleanup, which only removes runtime leftovers: the flag must survive both
/// a daemon restart AND a crash.
fn flag_path(config_dir: &Path) -> PathBuf {
    config_dir.join("storestation.core.json")
}

/// Read the persisted flag. Tolerant by design: no file (first run, old
/// daemon's dir) and unparseable content (a hand-mangled file) both read as
/// OFF — a broken byte can never turn the machine's sleep block on.
fn load_flag(config_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(flag_path(config_dir)) else {
        return false;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("enabled").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn save_flag(config_dir: &Path, enabled: bool) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(&json!({ "enabled": enabled }))
        .expect("the flag document is always serializable");
    std::fs::write(flag_path(config_dir), body)
}

/// The daemon-side Core state: the user's choice (`enabled`, persisted) and
/// the platform's live truth (`held`, best effort) plus, when the guarantee
/// is limited or refused, the instruction string — never silent (story 142).
pub struct CoreState {
    enabled: bool,
    assertion: Option<power::PowerAssertion>,
    instruction: Option<String>,
}

impl CoreState {
    /// Daemon-start restore: read the flag and, when Core is ON, take the
    /// assertion BEFORE the serve loop starts — a fresh daemon (login
    /// autostart included) re-asserts with no client call (AC: Core ON
    /// survives a daemon restart).
    pub fn restore(config_dir: &Path) -> CoreState {
        let mut state = CoreState {
            enabled: false,
            assertion: None,
            instruction: None,
        };
        if load_flag(config_dir) {
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
    /// taken or dropped. The returned error (persistence failure) is the
    /// only case the op answers as a failure; the held truth travels in the
    /// view, never as a silent success.
    pub fn set_enabled(&mut self, config_dir: &Path, on: bool) -> Result<(), String> {
        save_flag(config_dir, on).map_err(|e| format!("could not persist the Core flag: {e}"))?;
        self.apply(on);
        Ok(())
    }

    /// (Re)apply the choice: drop whatever assertion is held, then — when
    /// ON — take a fresh one and evaluate the battery caveat.
    fn apply(&mut self, on: bool) {
        self.enabled = on;
        self.assertion = None; // Drop releases the previous assertion
        self.instruction = None;
        if !on {
            return;
        }
        match power::assert_system_sleep() {
            Ok(assertion) => {
                self.assertion = Some(assertion);
                // The OS created the assertion, but macOS honors the
                // system-sleep block only on AC — on battery the caveat is
                // the honest state, never silence.
                if power_source_is_battery() == Some(true) {
                    self.instruction = Some(BATTERY_INSTRUCTION.to_string());
                }
                // Windows: the hold works (idle sleep, battery included),
                // but the lid-close action is a power-plan setting this
                // call cannot touch — while ON the lid caveat IS the
                // honest state (PO decision 2026-10-04), never silence.
                #[cfg(target_os = "windows")]
                {
                    self.instruction = Some(LID_INSTRUCTION.to_string());
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

    /// The status-view shape shared by `core.set` results and the
    /// `storestation.status` `core` object: `{enabled, held, instruction}`.
    pub fn view(&self) -> Value {
        json!({
            "enabled": self.enabled,
            "held": self.held(),
            "instruction": self.instruction,
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
/// Everywhere else: a stub that refuses honestly.
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

/// Platforms without a backend yet (Linux, phase 4): refuse honestly —
/// `held: false` + the instruction string, never a fake OK.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
        save_flag(dir.path(), false).unwrap();
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
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_restore_re_asserts_from_the_persisted_flag() {
        let dir = tempfile::tempdir().unwrap();
        save_flag(dir.path(), true).unwrap();
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
