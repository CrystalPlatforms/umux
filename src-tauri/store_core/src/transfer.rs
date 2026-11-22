//! transfer — the Desktop↔Terminal workspace-state move (B7, v1.9.0 / #79–#80).
//!
//! Desktop and Terminal keep SEPARATE saved states (`config_path()` vs
//! `term_config_path()`); this module is the pure merge one store's state
//! into the other's when the user explicitly moves it (the CLI's
//! `workspaces import desk|term`, and later F1's UI buttons through the same
//! commands). Settings are deliberately NOT transferable — per-surface app
//! config, the exchange module's documented rule.
//!
//! PURE, like everything beside the stores: both states arrive by value, no
//! I/O, a fresh-id supplier is injected. Merge semantics mirror the cmux
//! importer's additive contract (nothing is ever overwritten):
//!   - name collisions suffixed `X from <label>` → `X from <label> 2` …;
//!   - every incoming entity gets a FRESH id (the two stores' id spaces can
//!     overlap — a term store may have been exported from the desk store);
//!   - groups keep their nesting, members re-file under their mapped group;
//!   - the incoming ORDER is appended (remapped), nested groups excluded —
//!     they hang off their parent, never the sidebar root;
//!   - the source state is never mutated: the result is a NEW state.

use std::collections::{HashMap, HashSet};

use crate::workspace_store::{Group, LayoutNode, Workspace, WorkspaceData};

/// A name that survives collision with EVERYTHING already in `existing`:
/// `X`, then `X from <label>`, then `X from <label> 2`, … Nothing is ever
/// overwritten. (`label` is "desk" or "term" — WHOSE store the entity came
/// from.) The cmux importer's `unique_name`, generalized over the label.
/// The set holds OWNED names — it must outlive each generated candidate.
fn unique_name(existing: &HashSet<String>, base: &str, label: &str) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{base} from {label}")
        } else {
            format!("{base} from {label} {n}")
        };
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Rebuild a layout tree with FRESH ids for every node (split AND leaf):
/// the incoming store's panel ids may equal ids already in the target, and
/// layout ids are pane identity — they must never collide. Ratios, shape and
/// order are preserved.
fn remap_layout(node: &LayoutNode, gen_id: &mut dyn FnMut() -> String) -> LayoutNode {
    match node {
        LayoutNode::Leaf { .. } => LayoutNode::Leaf { id: gen_id() },
        LayoutNode::Split {
            orientation,
            ratio,
            first,
            second,
            ..
        } => LayoutNode::Split {
            id: gen_id(),
            orientation: *orientation,
            ratio: *ratio,
            first: Box::new(remap_layout(first, gen_id)),
            second: Box::new(remap_layout(second, gen_id)),
        },
    }
}

/// Merge `incoming` (the OTHER store's state) into `live`, returning the NEW
/// target state. `label` names the source store ("desk" / "term") and drives
/// the collision suffix; `gen_id` supplies the target-side ids. Pure.
pub fn merge_states(
    live: &WorkspaceData,
    incoming: &WorkspaceData,
    label: &str,
    gen_id: &mut dyn FnMut() -> String,
) -> WorkspaceData {
    if incoming.workspaces.is_empty() && incoming.groups.is_empty() {
        return live.clone();
    }
    let mut next = live.clone();

    // --- groups first (workspaces reference them) ---------------------------
    // Pass 1: every incoming group gets a fresh id + collision-resolved name.
    let mut group_id_map: HashMap<String, String> = HashMap::new();
    let mut taken_group_names: HashSet<String> =
        next.groups.iter().map(|g| g.name.clone()).collect();
    let mut fresh_groups: Vec<(Group, Option<String>)> = Vec::new();
    for g in &incoming.groups {
        let name = unique_name(&taken_group_names, &g.name, label);
        taken_group_names.insert(name.clone());
        let new_id = gen_id();
        // parent_id resolved in pass 2 (the parent may come later in the vec).
        fresh_groups.push((
            Group {
                id: new_id.clone(),
                name,
                collapsed: g.collapsed,
                pinned: g.pinned,
                parent_id: None,
                color: g.color.clone(),
            },
            g.parent_id.clone(),
        ));
        group_id_map.insert(g.id.clone(), new_id);
    }
    // Pass 2: repoint parents through the map; a parent that does not exist
    // (dangling reference) drops to top level — never a dangling id.
    for (group, old_parent) in &mut fresh_groups {
        if let Some(old) = old_parent.clone() {
            group.parent_id = group_id_map.get(&old).cloned();
        }
    }
    for (group, _) in &fresh_groups {
        next.groups.push(group.clone());
    }

    // --- workspaces ----------------------------------------------------------
    let mut taken_ws_names: HashSet<String> = next
        .workspaces
        .iter()
        .map(|w| w.name.clone())
        .collect();
    let mut ws_id_map: HashMap<String, String> = HashMap::new();
    for w in &incoming.workspaces {
        let name = unique_name(&taken_ws_names, &w.name, label);
        taken_ws_names.insert(name.clone());
        let new_id = gen_id();
        ws_id_map.insert(w.id.clone(), new_id.clone());
        next.workspaces.push(Workspace {
            id: new_id,
            name,
            panels: w.panels.clone(),
            layout: w.layout.as_ref().map(|l| remap_layout(l, gen_id)),
            tabs: w
                .tabs
                .iter()
                .map(|t| {
                    let mut tab = t.clone();
                    tab.id = gen_id();
                    tab.layout = t.layout.as_ref().map(|l| remap_layout(l, gen_id));
                    tab
                })
                .collect(),
            pinned: w.pinned,
            group_id: w.group_id.as_ref().and_then(|g| group_id_map.get(g).cloned()),
            color: w.color.clone(),
        });
    }

    // --- order ---------------------------------------------------------------
    // Append the incoming display order, remapped; a workspace whose group
    // was dangling (or an id that referenced nothing) is dropped rather than
    // corrupting the sidebar. Nested groups never join the root order.
    let top_level_groups: HashSet<&str> = incoming
        .groups
        .iter()
        .filter(|g| g.parent_id.is_some())
        .map(|g| g.id.as_str())
        .collect();
    for id in &incoming.order {
        if top_level_groups.contains(id.as_str()) {
            continue;
        }
        let mapped = if let Some(g) = group_id_map.get(id) {
            Some(g.clone())
        } else {
            ws_id_map.get(id).cloned()
        };
        if let Some(mapped) = mapped {
            next.order.push(mapped);
        }
    }

    next
}

/// One preview row of the Desktop↔Terminal wizard (#80 rework, 2026-11-22):
/// a workspace the planned merge ADDS over the target, with the FINAL
/// collision-suffixed name, its resolved group name (None = top level) and
/// its tab count. Serialized camelCase — the rows ride the preview command's
/// JSON straight into the frontend's tree builder.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedWorkspaceRow {
    pub name: String,
    pub group_name: Option<String>,
    pub tab_count: usize,
}

/// A group node the planned merge ADDS over the target, with how many of the
/// planned workspaces file into it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedGroupRow {
    pub name: String,
    pub child_count: usize,
}

/// Diff a planned merge result against its target (PURE): every workspace
/// and group whose id the target does not have (mergeStates mints FRESH ids
/// for the incoming side, so id absence is exactly "added") becomes a row —
/// the wizard's preview list. Names are the FINAL ones (suffixes resolved),
/// so what the user previews is what lands. The cmux wizard's
/// `buildImportPreview`, expressed over whole store states.
pub fn added_rows(
    live: &WorkspaceData,
    planned: &WorkspaceData,
) -> (Vec<AddedWorkspaceRow>, Vec<AddedGroupRow>) {
    let live_ws_ids: HashSet<&str> = live.workspaces.iter().map(|w| w.id.as_str()).collect();
    let live_group_ids: HashSet<&str> = live.groups.iter().map(|g| g.id.as_str()).collect();
    let rows = planned
        .workspaces
        .iter()
        .filter(|w| !live_ws_ids.contains(w.id.as_str()))
        .map(|w| AddedWorkspaceRow {
            name: w.name.clone(),
            group_name: w
                .group_id
                .as_ref()
                .and_then(|gid| planned.groups.iter().find(|g| &g.id == gid))
                .map(|g| g.name.clone()),
            tab_count: w.tabs.len(),
        })
        .collect();
    let group_rows = planned
        .groups
        .iter()
        .filter(|g| !live_group_ids.contains(g.id.as_str()))
        .map(|g| AddedGroupRow {
            name: g.name.clone(),
            child_count: planned
                .workspaces
                .iter()
                .filter(|w| w.group_id.as_deref() == Some(g.id.as_str()))
                .count(),
        })
        .collect();
    (rows, group_rows)
}

#[cfg(test)]
mod tests {
    //! The merge contract's pure core: collisions never overwrite, ids are
    //! freshly minted on the target side, groups keep their nesting, the
    //! incoming order appends remapped, and the source state is never
    //! mutated (the result is a NEW state).

    use super::*;
    use crate::workspace_store::{Orientation, Tab};

    fn ws(id: &str, name: &str) -> Workspace {
        Workspace {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    fn group(id: &str, name: &str, parent: Option<&str>) -> Group {
        Group {
            id: id.into(),
            name: name.into(),
            collapsed: None,
            pinned: None,
            parent_id: parent.map(|p| p.to_string()),
            color: None,
        }
    }

    fn sequential_ids() -> impl FnMut() -> String {
        let mut n = 0u64;
        move || {
            n += 1;
            format!("new-{n}")
        }
    }

    fn tab(id: &str, leaf: &str) -> Tab {
        Tab {
            id: id.into(),
            layout: Some(LayoutNode::Leaf { id: leaf.into() }),
            name: None,
            pinned: None,
            color: None,
            shell: None,
        }
    }

    #[test]
    fn empty_incoming_state_leaves_the_target_alone() {
        let live = WorkspaceData {
            workspaces: vec![ws("w1", "proj")],
            ..Default::default()
        };
        let next = merge_states(&live, &WorkspaceData::default(), "desk", &mut sequential_ids());
        assert_eq!(next, live);
    }

    // --- added_rows: the wizard's preview rows --------------------------------
    //
    // The Desktop↔Terminal wizard (#80 rework, 2026-11-22) previews the merge
    // through THESE rows: every entity the planned state adds over the target
    // (fresh ids — nothing else can differ), with the FINAL collision-suffixed
    // names, the resolved group name and the tab count. The cmux wizard's
    // buildImportPreview, expressed over whole store states.

    #[test]
    fn added_rows_list_only_the_planned_additions_with_final_names() {
        let live = WorkspaceData {
            workspaces: vec![ws("w1", "proj")],
            ..Default::default()
        };
        let incoming = WorkspaceData {
            workspaces: vec![ws("w2", "proj"), {
                let mut w = ws("w3", "solo");
                w.tabs = vec![tab("t1", "leaf-1"), tab("t2", "leaf-2")];
                w
            }],
            groups: vec![group("g1", "client", None)],
            ..Default::default()
        };
        let planned = merge_states(&live, &incoming, "term", &mut sequential_ids());
        let (rows, group_rows) = added_rows(&live, &planned);
        assert_eq!(rows.len(), 2, "the target's own row never lists");
        assert_eq!(rows[0].name, "proj from term");
        assert_eq!(rows[0].group_name, None);
        assert_eq!(rows[0].tab_count, 0);
        assert_eq!(rows[1].name, "solo");
        assert_eq!(rows[1].tab_count, 2);
        assert_eq!(group_rows.len(), 1);
        assert_eq!(group_rows[0].name, "client");
    }

    #[test]
    fn added_rows_resolve_the_group_name_and_count_its_planned_members() {
        let live = WorkspaceData::default();
        let incoming = WorkspaceData {
            workspaces: vec![{
                let mut w = ws("w1", "inner");
                w.group_id = Some("g1".into());
                w
            }],
            groups: vec![group("g1", "client", None)],
            ..Default::default()
        };
        let planned = merge_states(&live, &incoming, "desk", &mut sequential_ids());
        let (rows, group_rows) = added_rows(&live, &planned);
        assert_eq!(rows[0].group_name.as_deref(), Some("client"));
        assert_eq!(group_rows[0].child_count, 1);
    }

    #[test]
    fn added_rows_of_an_empty_merge_are_empty() {
        let live = WorkspaceData {
            workspaces: vec![ws("w1", "proj")],
            ..Default::default()
        };
        let planned = merge_states(&live, &WorkspaceData::default(), "desk", &mut sequential_ids());
        let (rows, group_rows) = added_rows(&live, &planned);
        assert!(rows.is_empty());
        assert!(group_rows.is_empty());
    }

    #[test]
    fn colliding_names_get_the_from_label_suffix_then_numbers() {
        let live = WorkspaceData {
            workspaces: vec![ws("w1", "proj")],
            ..Default::default()
        };
        let incoming = WorkspaceData {
            workspaces: vec![ws("w1", "proj"), ws("w2", "proj")],
            ..Default::default()
        };
        let next = merge_states(&live, &incoming, "desk", &mut sequential_ids());
        let names: Vec<&str> = next.workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, vec!["proj", "proj from desk", "proj from desk 2"]);
    }

    #[test]
    fn incoming_entities_get_fresh_ids_even_without_name_clashes() {
        let live = WorkspaceData {
            workspaces: vec![ws("w1", "alpha")],
            ..Default::default()
        };
        let incoming = WorkspaceData {
            workspaces: vec![ws("w1", "beta")],
            ..Default::default()
        };
        let next = merge_states(&live, &incoming, "term", &mut sequential_ids());
        assert_eq!(next.workspaces.len(), 2);
        assert_eq!(next.workspaces[0].id, "w1", "the target's own ids stand");
        assert_eq!(
            next.workspaces[1].id, "new-1",
            "the incoming id space is remapped"
        );
    }

    #[test]
    fn groups_keep_nesting_and_workspaces_re_file_under_mapped_groups() {
        let live = WorkspaceData::default();
        let incoming = WorkspaceData {
            workspaces: vec![Workspace {
                group_id: Some("g-child".into()),
                ..ws("w1", "inner")
            }],
            groups: vec![group("g-root", "Root", None), group("g-child", "Child", Some("g-root"))],
            order: vec!["g-root".into(), "w1".into()],
            ..Default::default()
        };
        let next = merge_states(&live, &incoming, "desk", &mut sequential_ids());
        // Fresh group ids, the parent link remapped through them.
        let child = next.groups.iter().find(|g| g.name == "Child").unwrap();
        let root = next.groups.iter().find(|g| g.name == "Root").unwrap();
        assert_ne!(child.id, "g-child");
        assert_ne!(root.id, "g-root");
        assert_eq!(child.parent_id.as_deref(), Some(root.id.as_str()));
        // The workspace hangs off the mapped group.
        assert_eq!(next.workspaces[0].group_id.as_deref(), Some(child.id.as_str()));
        // The nested group never joins the root order — only the root group
        // and the workspace do, in the incoming order.
        assert_eq!(
            next.order,
            vec![root.id.clone(), next.workspaces[0].id.clone()]
        );
    }

    #[test]
    fn a_dangling_group_reference_drops_to_top_level() {
        let live = WorkspaceData::default();
        let incoming = WorkspaceData {
            workspaces: vec![Workspace {
                group_id: Some("g-ghost".into()),
                ..ws("w1", "lost")
            }],
            ..Default::default()
        };
        let next = merge_states(&live, &incoming, "term", &mut sequential_ids());
        assert_eq!(next.workspaces[0].group_id, None);
    }

    #[test]
    fn order_entries_that_reference_nothing_are_dropped() {
        let live = WorkspaceData {
            workspaces: vec![ws("w1", "live")],
            order: vec!["w1".into()],
            ..Default::default()
        };
        let incoming = WorkspaceData {
            workspaces: vec![ws("w9", "new")],
            order: vec!["w9".into(), "ghost".into()],
            ..Default::default()
        };
        let next = merge_states(&live, &incoming, "desk", &mut sequential_ids());
        assert_eq!(next.order, vec!["w1".to_string(), "new-1".to_string()]);
    }

    #[test]
    fn tab_layouts_arrive_with_fresh_node_ids_and_kept_shape() {
        let incoming_layout = LayoutNode::Split {
            id: "s1".into(),
            orientation: Orientation::Horizontal,
            ratio: 0.7,
            first: Box::new(LayoutNode::Leaf { id: "p1".into() }),
            second: Box::new(LayoutNode::Leaf { id: "p2".into() }),
        };
        let live = WorkspaceData::default();
        let incoming = WorkspaceData {
            workspaces: vec![Workspace {
                tabs: vec![tab("t1", "p1")],
                layout: Some(incoming_layout.clone()),
                ..ws("w1", "split")
            }],
            ..Default::default()
        };
        let next = merge_states(&live, &incoming, "desk", &mut sequential_ids());
        let moved = &next.workspaces[0];
        assert_ne!(moved.tabs[0].id, "t1", "tab ids are remapped");
        match &moved.tabs[0].layout {
            Some(LayoutNode::Leaf { id }) => assert_ne!(id, "p1", "leaf ids are remapped"),
            _ => panic!("expected a leaf"),
        }
        match &moved.layout {
            Some(LayoutNode::Split { id, ratio, first, second, .. }) => {
                assert_ne!(id, "s1");
                assert!((ratio - 0.7).abs() < 1e-9, "ratios are preserved");
                let leaf_id = |n: &LayoutNode| match n {
                    LayoutNode::Leaf { id } => id.clone(),
                    _ => panic!("expected leaves"),
                };
                assert_ne!(leaf_id(first), "p1");
                assert_ne!(leaf_id(second), "p2");
                assert_ne!(leaf_id(first), leaf_id(second), "every node gets its own id");
            }
            _ => panic!("expected the split to survive"),
        }
    }

    #[test]
    fn the_source_state_is_never_mutated() {
        let incoming = WorkspaceData {
            workspaces: vec![ws("w1", "proj")],
            groups: vec![group("g1", "Grp", None)],
            order: vec!["w1".into(), "g1".into()],
            ..Default::default()
        };
        let before = incoming.clone();
        let live = WorkspaceData::default();
        let _ = merge_states(&live, &incoming, "desk", &mut sequential_ids());
        assert_eq!(incoming, before, "merge must not touch its input");
    }
}
