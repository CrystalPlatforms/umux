//! The neutral umux exchange format (#61, story 71).
//!
//! One self-describing JSON envelope carries a store's state between umux
//! surfaces: `umux export` writes it, the CLI import (#63) and the v1.7.0
//! Desktop↔Terminal transfer read it. It lives in StoreCore — next to the
//! model it wraps — so every producer and consumer shares ONE definition,
//! the same reason the stores themselves are here.
//!
//! Wire shape (pretty-printed on export):
//!
//! ```json
//! {
//!   "format": "umux-exchange",
//!   "version": 1,
//!   "kind": "workspaces",
//!   "data": { "workspaces": [...], "groups": [...], "order": [...] }
//! }
//! ```
//!
//! - `format` identifies the envelope; anything else is not ours to parse.
//! - `version` is the format version. A reader that sees a version it does
//!   not know must refuse the file with a clear message, never guess — the
//!   rule is documented in the README's Exchange format section.
//! - `kind` names what `data` holds (`workspaces` today). Settings are
//!   deliberately NOT exchange data: they are per-surface app config, not
//!   state a user moves between machines or surfaces.
//! - `data` is the store's own serialized shape (the exact JSON
//!   `WorkspaceStore` saves), so export→import round-trips without a
//!   second translation layer.

use serde::{Deserialize, Serialize};

use crate::workspace_store::WorkspaceData;

/// Wire value of the envelope's `format` field.
pub const EXCHANGE_FORMAT: &str = "umux-exchange";

/// Wire value of the envelope's `version` field. Bump ONLY for a breaking
/// change to the shape below; readers refuse versions they don't know.
pub const EXCHANGE_VERSION: u32 = 1;

/// What the envelope's `data` field holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExchangeKind {
    /// The workspace state of one store (workspaces, groups, order).
    Workspaces,
}

impl ExchangeKind {
    /// The wire string for this kind.
    pub fn wire(self) -> &'static str {
        match self {
            ExchangeKind::Workspaces => "workspaces",
        }
    }
}

/// The exchange envelope as it serializes. `data` borrows the store state so
/// building a document never copies it.
#[derive(Serialize)]
pub struct ExchangeDocument<'a> {
    pub format: &'static str,
    pub version: u32,
    pub kind: &'static str,
    pub data: &'a WorkspaceData,
}

/// Serialize a store's workspace state as an exchange document (pretty JSON,
/// no trailing newline — the CLI decides between stdout and a file).
pub fn to_exchange(kind: ExchangeKind, data: &WorkspaceData) -> String {
    let document = ExchangeDocument {
        format: EXCHANGE_FORMAT,
        version: EXCHANGE_VERSION,
        kind: kind.wire(),
        data,
    };
    serde_json::to_string_pretty(&document).expect("ExchangeDocument is always serializable")
}

/// The envelope as it parses back (owned — the reader takes the data with it).
#[derive(Deserialize)]
struct ExchangeEnvelope {
    format: String,
    version: u32,
    kind: String,
    data: WorkspaceData,
}

/// Parse an exchange document into its kind and data. The documented reader
/// rule, enforced: invalid JSON, a missing/wrong envelope, an unknown FORMAT,
/// an unknown VERSION, or an unknown KIND each refuse with a clear message —
/// a reader never guesses (README, Exchange format).
pub fn from_exchange(text: &str) -> Result<(ExchangeKind, WorkspaceData), String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| "the document is not valid JSON".to_string())?;
    let envelope: ExchangeEnvelope = serde_json::from_value(value)
        .map_err(|_| "the document is not an umux exchange document".to_string())?;
    if envelope.format != EXCHANGE_FORMAT {
        return Err(format!(
            "not an umux exchange document (format \"{}\")",
            envelope.format
        ));
    }
    if envelope.version != EXCHANGE_VERSION {
        return Err(format!(
            "unsupported exchange version {} — this umux reads version {}",
            envelope.version, EXCHANGE_VERSION
        ));
    }
    let kind = match envelope.kind.as_str() {
        "workspaces" => ExchangeKind::Workspaces,
        other => {
            return Err(format!("unsupported exchange kind \"{other}\""));
        }
    };
    Ok((kind, envelope.data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_store::{Group, LayoutNode, Orientation, Tab, Workspace};

    /// A store state exercising every exchange-relevant shape: a grouped
    /// workspace with a split tab, plus the interleaved sidebar order.
    fn sample_data() -> WorkspaceData {
        WorkspaceData {
            workspaces: vec![
                Workspace {
                    id: "ws-1".into(),
                    name: "alpha".into(),
                    panels: vec![],
                    layout: None,
                    pinned: None,
                    group_id: Some("g-1".into()),
                    color: None,
                    tabs: vec![Tab {
                        id: "tab-1".into(),
                        layout: Some(LayoutNode::Split {
                            id: "s-1".into(),
                            orientation: Orientation::Horizontal,
                            ratio: 0.5,
                            first: Box::new(LayoutNode::Leaf { id: "p-1".into() }),
                            second: Box::new(LayoutNode::Leaf { id: "p-2".into() }),
                        }),
                        name: Some("Tab 1".into()),
                        pinned: None,
                        color: None,
                    }],
                },
                Workspace {
                    id: "ws-2".into(),
                    name: "beta".into(),
                    ..Default::default()
                },
            ],
            groups: vec![Group {
                id: "g-1".into(),
                name: "projekty".into(),
                collapsed: None,
                pinned: None,
                parent_id: None,
                color: None,
            }],
            order: vec!["ws-2".into(), "g-1".into(), "ws-1".into()],
        }
    }

    // T-X1 (the envelope is self-describing — the documented wire contract):
    //   Input:  an exchange document for a workspace state.
    //   Output: format "umux-exchange", version 1, kind "workspaces" —
    //           exactly the README's Exchange format section.
    #[test]
    fn envelope_carries_format_version_and_kind() {
        let doc: serde_json::Value =
            serde_json::from_str(&to_exchange(ExchangeKind::Workspaces, &WorkspaceData::default()))
                .unwrap();

        assert_eq!(doc["format"], "umux-exchange");
        assert_eq!(doc["version"], 1);
        assert_eq!(doc["kind"], "workspaces");
    }

    // T-X2 (AC — content equals the store state; `data` IS the store's own
    // serialized shape, so it parses back into the identical model):
    //   Input:  the sample state (grouped workspace, split tab, order).
    //   Output: doc["data"] deserializes into a WorkspaceData equal to the
    //           input — the round-trip property the import (#63) relies on.
    #[test]
    fn data_field_round_trips_to_the_store_state() {
        let data = sample_data();

        let doc: serde_json::Value =
            serde_json::from_str(&to_exchange(ExchangeKind::Workspaces, &data)).unwrap();
        let back: WorkspaceData = serde_json::from_value(doc["data"].clone()).unwrap();

        assert_eq!(back, data);
    }

    // T-X3 (a first-run store exports a valid, well-formed document too):
    //   Input:  an empty WorkspaceData.
    //   Output: valid JSON whose data parses back empty — export never
    //           special-cases "nothing to export".
    #[test]
    fn empty_store_exports_a_valid_document() {
        let text = to_exchange(ExchangeKind::Workspaces, &WorkspaceData::default());

        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        let back: WorkspaceData = serde_json::from_value(doc["data"].clone()).unwrap();
        assert_eq!(back, WorkspaceData::default());
    }

    // T-X4 (kind names the payload — a second kind would only ever come with
    // a documented reader; the wire string is part of the format contract):
    #[test]
    fn kind_wire_names_are_stable() {
        assert_eq!(ExchangeKind::Workspaces.wire(), "workspaces");
    }

    // T-X5 (the reader round-trips the writer — the property `umux import
    // umux` and the export→import round-trip hang on):
    #[test]
    fn from_exchange_round_trips_to_exchange() {
        let data = sample_data();

        let (kind, back) =
            from_exchange(&to_exchange(ExchangeKind::Workspaces, &data)).unwrap();

        assert_eq!(kind, ExchangeKind::Workspaces);
        assert_eq!(back, data);
    }

    // T-X6 (the documented reader rule — every alien document refuses with a
    // clear message instead of guessing):
    //   invalid JSON / wrong shape / unknown format / unknown version /
    //   unknown kind → Err naming the problem.
    #[test]
    fn from_exchange_refuses_alien_documents() {
        let err = from_exchange("{ not json").unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");

        let err = from_exchange(r#"{"a": 1}"#).unwrap_err();
        assert!(err.contains("not an umux exchange document"), "{err}");

        let alien = |from: &str, to: &str| {
            r#"{"format":"umux-exchange","version":1,"kind":"workspaces","data":{}}"#
                .replacen(from, to, 1)
        };
        let err = from_exchange(&alien(
            r#""format":"umux-exchange""#,
            r#""format":"someone-else""#,
        ))
        .unwrap_err();
        assert!(err.contains("not an umux exchange document"), "{err}");

        let err = from_exchange(&alien(r#""version":1"#, r#""version":99"#)).unwrap_err();
        assert!(
            err.contains("unsupported exchange version 99"),
            "unknown versions refuse: {err}"
        );

        let err = from_exchange(&alien(
            r#""kind":"workspaces""#,
            r#""kind":"something-else""#,
        ))
        .unwrap_err();
        assert!(err.contains("unsupported exchange kind"), "{err}");
    }
}
