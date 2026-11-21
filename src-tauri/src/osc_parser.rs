// OscParser — re-export (v1.9.0, B5).
//
// The parser itself MOVED to `session_core::osc_parser` (v1.9.0 issue B5):
// the umux Terminal needs the same completion detection the desktop uses,
// and the plan's decision rule prefers a SHARED crate over a copy — one
// implementation, no drift. Every existing `crate::osc_parser::*` path in
// the app keeps working through this re-export; the module's own docs, the
// state machine and the fixtures live beside the session engine now.

pub use session_core::osc_parser::{
    NotificationEvent, OscParser, OscProtocol, PushResult,
};
