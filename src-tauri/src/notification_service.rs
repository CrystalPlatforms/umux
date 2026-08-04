// NotificationService — turns a parsed OSC completion event into a native
// desktop notification (Phase 13 / #14).
//
// Deep module: a tiny `notify(event, origin)` surface hiding everything about
// message composition and libnotify delivery. The system boundary (libnotify /
// notify-rust) sits behind the `Notifier` trait, so the behaviorally interesting
// part — how the message is built and what origin info it carries — is unit
// tested with a recording notifier; the real `LibnotifyNotifier` (in lib.rs) is
// a thin adapter on the OS boundary and is not unit-tested.
//
// Assumptions encoded by these tests:
//  - Input:  a NotificationEvent produced by OscParser (title may be empty for
//            OSC 9; body is the human message) + a PanelOrigin describing where
//            it came from (workspace/panel labels, either may be absent).
//  - Output: exactly one `Notifier::show(summary, body)` call per event.
//  - summary: the event's title when non-empty (Kitty/urxvt carry one),
//            otherwise the app label ("umux").
//  - body:    the event message; when an origin label is available it is
//            appended so the user can tell which workspace/panel finished.
//  - Tests assert on the PRESENCE of origin names in the body (substring), not
//            on exact formatting, so they survive cosmetic changes to the text.
//  - NOT tested here: debouncing/idempotency (deferred to a later issue),
//    libnotify delivery itself, the OscParser -> service wiring (lib.rs).

use crate::osc_parser::NotificationEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The system boundary: whatever actually shows a desktop notification.
/// Real impl (`LibnotifyNotifier`) lives in lib.rs; tests use a recording fake.
pub trait Notifier: Send {
    fn show(&self, summary: &str, body: &str);
}

/// Where a notification originated. Either field may be absent; when both are,
/// the notification carries no origin suffix.
#[derive(Clone, Default)]
pub struct PanelOrigin {
    pub workspace: Option<String>,
    pub panel: Option<String>,
}

impl PanelOrigin {
    /// True if any origin label is present and worth appending.
    fn has_any(&self) -> bool {
        self.workspace.is_some() || self.panel.is_some()
    }
}

pub struct NotificationService {
    notifier: Box<dyn Notifier + Send>,
    app_label: String,
    /// Shared mute flag. When true, notify() is a silent no-op. Held behind an
    /// Arc so lib.rs can share ONE flag across every panel's service — a single
    /// toggle mutes the whole app.
    muted: Arc<AtomicBool>,
}

impl NotificationService {
    pub fn new(notifier: Box<dyn Notifier + Send>, app_label: Option<String>) -> Self {
        Self {
            notifier,
            app_label: app_label.unwrap_or_else(|| "umux".to_string()),
            muted: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Like `new`, but the service observes an externally owned mute flag
    /// instead of a private one. lib.rs creates ONE flag, shares it across
    /// every panel's service (so a single toggle mutes the whole app), and the
    /// `set_notifications_muted` command flips that same flag.
    pub fn with_mute(
        notifier: Box<dyn Notifier + Send>,
        app_label: Option<String>,
        muted: Arc<AtomicBool>,
    ) -> Self {
        Self {
            notifier,
            app_label: app_label.unwrap_or_else(|| "umux".to_string()),
            muted,
        }
    }

    /// Temporarily silence (true) or re-enable (false) notifications. The next
    /// notify() call observes the new state.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::SeqCst);
    }

    /// Whether notifications are currently silenced.
    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    /// Compose and fire one notification for `event`, labeling it with `origin`
    /// when available.
    pub fn notify(&self, event: &NotificationEvent, origin: &PanelOrigin) {
        // Muted: drop the event silently. The notifier is never reached, so no
        // desktop notification fires — but terminal output is unaffected (the
        // OSC bytes were already stripped by the parser before we get here).
        if self.muted.load(Ordering::SeqCst) {
            return;
        }
        // summary: prefer an explicit title (Kitty/urxvt carry one); otherwise
        // fall back to the app label.
        let summary = if event.title.is_empty() {
            self.app_label.as_str()
        } else {
            event.title.as_str()
        };

        // body: the message, with an origin suffix when any label is present.
        let body = if origin.has_any() {
            let parts: Vec<&str> = [origin.workspace.as_deref(), origin.panel.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            format!("{}\n({})", event.body, parts.join(" · "))
        } else {
            event.body.clone()
        };

        self.notifier.show(summary, &body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc_parser::OscProtocol;
    use std::sync::{Arc, Mutex};

    /// A fake Notifier that records every show() call. Arc<Mutex<..>> so it is
    /// Send+Sync (NotificationService requires Notifier: Send).
    #[derive(Default, Clone)]
    struct RecordingNotifier {
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl Notifier for RecordingNotifier {
        fn show(&self, summary: &str, body: &str) {
            self.calls
                .lock()
                .unwrap()
                .push((summary.to_string(), body.to_string()));
        }
    }

    fn service() -> (NotificationService, RecordingNotifier) {
        let rec = RecordingNotifier::default();
        let svc = NotificationService::new(Box::new(rec.clone()), Some("umux".to_string()));
        (svc, rec)
    }

    // T1 (AC3 — notification references the originating workspace/panel):
    //   Input:  an OSC 9 event with body "build done", origin workspace "main"
    //           and panel "left".
    //   Output: exactly one show() call whose body contains the message AND both
    //           origin labels — so the user can see which panel finished.
    #[test]
    fn notify_with_origin_labels_body() {
        let (svc, rec) = service();
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "build done".to_string(),
        };
        let origin = PanelOrigin {
            workspace: Some("main".to_string()),
            panel: Some("left".to_string()),
        };

        svc.notify(&event, &origin);

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "exactly one notification per event");
        let (_summary, body) = &calls[0];
        assert!(body.contains("build done"), "body carries the message: {body}");
        assert!(body.contains("main"), "body carries the workspace: {body}");
        assert!(body.contains("left"), "body carries the panel: {body}");
    }

    // T2 (AC3 — "when available": absent origin must not leak an empty label):
    //   Input:  an event with body "done", origin with both fields None.
    //   Output: one show() whose body is exactly the message — no stray "()" or
    //           suffix — so a notification with no known origin stays clean.
    #[test]
    fn notify_without_origin_omits_suffix() {
        let (svc, rec) = service();
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "done".to_string(),
        };

        svc.notify(&event, &PanelOrigin::default());

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (summary, body) = &calls[0];
        assert_eq!(body, "done", "body is just the message: {body}");
        assert_eq!(summary, "umux", "summary falls back to app label");
        assert!(!body.contains('('), "no empty origin suffix: {body}");
    }

    // --- Phase 14: notification mute (#15) ---------------------------------
    //
    // Assumptions encoded by these tests:
    //  - A NotificationService carries a mutable mute flag (defaults to unmuted).
    //  - set_muted(true) makes notify() a no-op: the Notifier's show() is NEVER
    //    called while muted, so no desktop notification fires (AC2).
    //  - set_muted(false) restores delivery; the same service can be toggled off
    //    and back on (AC1).
    //  - NOT tested here: the cross-thread shared flag wiring (lib.rs injects one
    //    Arc<AtomicBool> into every panel's service so a single toggle mutes the
    //    whole app) — that's lib.rs wiring, verified manually.

    // T1 (tracer — AC2: while muted, no notifications fire):
    //   Input:  a muted service, then an event that would normally notify.
    //   Output: zero show() calls — the notifier is never reached.
    #[test]
    fn muted_service_suppresses_notification() {
        let (svc, rec) = service();
        svc.set_muted(true);
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "build done".to_string(),
        };

        svc.notify(&event, &PanelOrigin::default());

        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "muted service must not fire any notification"
        );
    }

    // T2 (AC1 — mute is off by default, so notifications still fire):
    //   Input:  a freshly constructed service.
    //   Output: is_muted() is false, and a notify() delivers exactly one show().
    #[test]
    fn fresh_service_is_unmuted_and_notifies() {
        let (svc, rec) = service();
        assert!(!svc.is_muted(), "mute is off by default");
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "done".to_string(),
        };

        svc.notify(&event, &PanelOrigin::default());

        assert_eq!(
            rec.calls.lock().unwrap().len(),
            1,
            "unmuted service delivers the notification"
        );
    }

    // T3 (AC1 — toggle off and back on): mute, then unmute, then notify.
    //   Input:  set_muted(true) -> set_muted(false) -> notify.
    //   Output: is_muted() is false again, and exactly one show() fires — the
    //           mute is reversible, not a one-way trap.
    #[test]
    fn unmuting_restores_notification_delivery() {
        let (svc, rec) = service();
        svc.set_muted(true);
        svc.set_muted(false);
        assert!(!svc.is_muted(), "mute cleared after set_muted(false)");
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "done".to_string(),
        };

        svc.notify(&event, &PanelOrigin::default());

        assert_eq!(
            rec.calls.lock().unwrap().len(),
            1,
            "notification fires again after unmuting"
        );
    }
}
