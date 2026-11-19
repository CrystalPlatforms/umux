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
