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
}

impl NotificationService {
    pub fn new(notifier: Box<dyn Notifier + Send>, app_label: Option<String>) -> Self {
        Self {
            notifier,
            app_label: app_label.unwrap_or_else(|| "umux".to_string()),
        }
    }

    /// Compose and fire one notification for `event`, labeling it with `origin`
    /// when available.
    pub fn notify(&self, event: &NotificationEvent, origin: &PanelOrigin) {
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
}
