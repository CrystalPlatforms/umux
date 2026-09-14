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

    /// Show a notification that carries an activation `payload` — an opaque
    /// string the backend echoes back when the user interacts with the banner
    /// (#76 follow-up: click-to-navigate). Default: this platform has no click
    /// handling — degrade to a plain show() and drop the payload.
    fn show_actionable(&self, summary: &str, body: &str, _payload: &str) {
        self.show(summary, body)
    }
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

    /// The origin suffix appended to a body: " (workspace · panel)" built from
    /// whichever labels are present. Only meaningful when has_any().
    fn suffix(&self) -> String {
        let parts: Vec<&str> = [self.workspace.as_deref(), self.panel.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        format!("\n({})", parts.join(" · "))
    }
}

/// The fixed body of a presence-based waiting ping (#76): a known AI CLI sits
/// in a panel's foreground, quiet, waiting for its human. No OSC event exists
/// on this path — the frontend's status machine detected the transition and
/// only reports WHERE via PanelOrigin.
const WAITING_BODY: &str = "Agent is waiting for your input";

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
    /// when available. Returns whether it was actually delivered (false when
    /// muted) so the caller can decide to announce the post.
    pub fn notify(&self, event: &NotificationEvent, origin: &PanelOrigin) -> bool {
        self.notify_with_payload(event, origin, None)
    }

    /// `notify` plus an opaque activation payload (#76 follow-up): with one,
    /// the banner goes out through the notifier's actionable path so a click
    /// can navigate back to the panel.
    pub fn notify_with_payload(
        &self,
        event: &NotificationEvent,
        origin: &PanelOrigin,
        payload: Option<&str>,
    ) -> bool {
        // Muted: drop the event silently. The notifier is never reached, so no
        // desktop notification fires — but terminal output is unaffected (the
        // OSC bytes were already stripped by the parser before we get here).
        if self.muted.load(Ordering::SeqCst) {
            return false;
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
            format!("{}{}", event.body, origin.suffix())
        } else {
            event.body.clone()
        };

        match payload {
            Some(p) => self.notifier.show_actionable(summary, &body, p),
            None => self.notifier.show(summary, &body),
        }
        true
    }

    /// Fire the presence-based waiting ping (#76): a known AI CLI is waiting
    /// for its first prompt in the panel described by `origin`. Same mute gate
    /// and origin-suffix composition as the completion path — the two paths
    /// differ only in WHERE their message text comes from (an OSC event there,
    /// the fixed WAITING_BODY here). Emission ONCE per transition is the
    /// caller's contract (the frontend fires on the state change, never per
    /// poll tick).
    pub fn notify_waiting(&self, origin: &PanelOrigin) -> bool {
        self.notify_waiting_with_payload(origin, None)
    }

    /// `notify_waiting` plus an activation payload — see notify_with_payload.
    pub fn notify_waiting_with_payload(&self, origin: &PanelOrigin, payload: Option<&str>) -> bool {
        if self.muted.load(Ordering::SeqCst) {
            return false;
        }
        let body = if origin.has_any() {
            format!("{}{}", WAITING_BODY, origin.suffix())
        } else {
            WAITING_BODY.to_string()
        };
        match payload {
            Some(p) => self.notifier.show_actionable(&self.app_label, &body, p),
            None => self.notifier.show(&self.app_label, &body),
        }
        true
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

    // --- Issue #76: presence-based needs-attention notifications -----------
    //
    // Assumptions encoded by these tests:
    //  - Input:  just a PanelOrigin (workspace/panel labels, either absent) —
    //            no OSC event exists on this path; the machine that detected
    //            the transition lives in the frontend and only reports WHERE.
    //  - Output: exactly one show() whose summary is the app label and whose
    //            body is the fixed waiting message, suffixed with the origin
    //            the same way completion notifications are (reuse, #76).
    //  - The app-wide mute flag gates this path too: muted -> zero show().
    //  - NOT tested here: WHICH command invokes this (lib.rs glue) or when the
    //    frontend calls it (transition-only debounce lives in the frontend).

    // T1 (tracer — the waiting ping carries the message and both labels):
    //   Input:  notify_waiting with origin workspace "main", panel "left".
    //   Output: one show(), summary "umux", body contains the waiting text
    //           AND both labels — the user can tell which panel waits.
    #[test]
    fn notify_waiting_with_origin_labels_body() {
        let (svc, rec) = service();
        let origin = PanelOrigin {
            workspace: Some("main".to_string()),
            panel: Some("left".to_string()),
        };

        svc.notify_waiting(&origin);

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "exactly one notification per transition");
        let (summary, body) = &calls[0];
        assert_eq!(summary, "umux", "summary is the app label");
        assert!(
            body.contains("waiting for your input"),
            "body names the waiting state: {body}"
        );
        assert!(body.contains("main"), "body carries the workspace: {body}");
        assert!(body.contains("left"), "body carries the panel: {body}");
    }

    // T2 (no origin must not leak an empty suffix — same contract as T2 of
    // the completion path): notify_waiting with both fields None fires a
    // clean body with no stray "()" or separator.
    #[test]
    fn notify_waiting_without_origin_omits_suffix() {
        let (svc, rec) = service();

        svc.notify_waiting(&PanelOrigin::default());

        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (_summary, body) = &calls[0];
        assert!(
            !body.contains('('),
            "no empty origin suffix on a label-less panel: {body}"
        );
    }

    // T3 (the bell mute gates waiting pings too — one mute is one mute):
    //   Input:  a muted service, then notify_waiting.
    //   Output: zero show() calls — muting the app mutes BOTH notification
    //           sources, never just one of them.
    #[test]
    fn muted_service_suppresses_waiting_notification() {
        let (svc, rec) = service();
        svc.set_muted(true);

        svc.notify_waiting(&PanelOrigin::default());

        assert!(
            rec.calls.lock().unwrap().is_empty(),
            "muted service must not fire the waiting ping"
        );
    }

    // --- Issue #76 follow-up (HITL 2026-09-14): click-to-navigate -----------
    //
    // A notification must be able to carry a NAVIGATION payload (where the
    // panel lives: workspace/tab/panel ids). The service stays a dumb postman:
    // it composes summary/body exactly as before and hands any payload string
    // to the notifier's actionable path; WHICH platform can act on a click is
    // the notifier's business (Linux notify-send actions; elsewhere the
    // frontend's focus heuristic).
    //
    // Assumptions encoded:
    //  - Input:  notify*/notify_waiting* with an optional payload (the JSON
    //            the backend will echo back on activation/posted events).
    //  - Output: with a payload, the notifier's show_actionable receives the
    //            composed summary/body AND the payload verbatim; without one,
    //            plain show() is enough (back-compat with every old call).
    //  - notify*/notify_waiting* return whether the notification was actually
    //            delivered (false when muted) so the CALLER can decide to
    //            emit its posted event — muted must never look posted.

    /// A fake Notifier that records BOTH surfaces separately.
    #[derive(Default, Clone)]
    struct ActionRecordingNotifier {
        plain: Arc<Mutex<Vec<(String, String)>>>,
        actionable: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    impl Notifier for ActionRecordingNotifier {
        fn show(&self, summary: &str, body: &str) {
            self.plain
                .lock()
                .unwrap()
                .push((summary.to_string(), body.to_string()));
        }
        fn show_actionable(&self, summary: &str, body: &str, payload: &str) {
            self.actionable
                .lock()
                .unwrap()
                .push((summary.to_string(), body.to_string(), payload.to_string()));
        }
    }

    // T4 (a payload rides to the notifier's actionable path verbatim):
    //   Input:  notify_with_payload with a navigation JSON, origin "main"/"left".
    //   Output: one show_actionable with the SAME composed body as the plain
    //           path and the payload unchanged — the service adds nothing.
    #[test]
    fn payload_rides_to_actionable_path_unchanged() {
        let rec = ActionRecordingNotifier::default();
        let svc = NotificationService::new(Box::new(rec.clone()), Some("umux".to_string()));
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "build done".to_string(),
        };
        let origin = PanelOrigin {
            workspace: Some("main".to_string()),
            panel: Some("left".to_string()),
        };

        let delivered = svc.notify_with_payload(&event, &origin, Some(r#"{"kind":"completion","ptyId":7}"#));

        assert!(delivered, "an unmuted notify reports delivery");
        let actionable = rec.actionable.lock().unwrap();
        assert_eq!(actionable.len(), 1, "exactly one actionable show");
        let (summary, body, payload) = &actionable[0];
        assert_eq!(summary, "umux");
        assert!(body.contains("build done"), "body composed as usual: {body}");
        assert!(body.contains("main"), "origin suffix still applied: {body}");
        assert_eq!(
            payload, r#"{"kind":"completion","ptyId":7}"#,
            "payload verbatim, untouched by composition"
        );
        assert!(rec.plain.lock().unwrap().is_empty(), "plain show not used");
    }

    // T5 (no payload keeps the plain path — every old call site unchanged):
    //   Input:  plain notify().
    //   Output: one plain show(), no actionable call, delivery reported.
    #[test]
    fn plain_notify_stays_on_plain_path() {
        let rec = ActionRecordingNotifier::default();
        let svc = NotificationService::new(Box::new(rec.clone()), Some("umux".to_string()));
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "done".to_string(),
        };

        let delivered = svc.notify(&event, &PanelOrigin::default());

        assert!(delivered);
        assert_eq!(rec.plain.lock().unwrap().len(), 1);
        assert!(rec.actionable.lock().unwrap().is_empty());
    }

    // T6 (muted reports NOT delivered — the caller must not claim a post):
    //   Input:  a muted service, then notify_with_payload.
    //   Output: false and zero notifier calls of either kind.
    #[test]
    fn muted_notify_reports_not_delivered() {
        let rec = ActionRecordingNotifier::default();
        let svc = NotificationService::new(Box::new(rec.clone()), Some("umux".to_string()));
        svc.set_muted(true);
        let event = NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: "done".to_string(),
        };

        let delivered = svc.notify_with_payload(&event, &PanelOrigin::default(), Some("{}"));

        assert!(!delivered, "muted delivery must read as not-posted");
        assert!(rec.plain.lock().unwrap().is_empty());
        assert!(rec.actionable.lock().unwrap().is_empty());
    }

    // T7 (the waiting ping carries the navigation payload too):
    //   Input:  notify_waiting_with_payload with the waiting target JSON.
    //   Output: one show_actionable: app-label summary, waiting body with the
    //           origin suffix, payload verbatim.
    #[test]
    fn waiting_ping_carries_payload() {
        let rec = ActionRecordingNotifier::default();
        let svc = NotificationService::new(Box::new(rec.clone()), Some("umux".to_string()));
        let origin = PanelOrigin {
            workspace: Some("api".to_string()),
            panel: Some("Tab 2".to_string()),
        };

        let delivered = svc.notify_waiting_with_payload(
            &origin,
            Some(r#"{"kind":"waiting","workspaceId":"ws-1","tabId":"tab-2","panelId":"p-3"}"#),
        );

        assert!(delivered);
        let actionable = rec.actionable.lock().unwrap();
        assert_eq!(actionable.len(), 1);
        let (summary, body, payload) = &actionable[0];
        assert_eq!(summary, "umux");
        assert!(body.contains("waiting for your input"), "body: {body}");
        assert!(body.contains("api") && body.contains("Tab 2"), "origin: {body}");
        assert!(payload.contains(r#""kind":"waiting""#));
    }
}
