//! cmux_import — the SECOND cmux importer, in Rust (#63).
//!
//! The FIRST importer is the TypeScript one (`src/cmuxImport.ts`, the wizard's
//! engine). Per the PO decision of 2026-08-30 it stays untouched — this module
//! is a second implementation of the same pipeline (parse → plan → apply),
//! kept at parity by the shared-fixture tests: `store_core/tests/cmux_parity.rs`
//! on this side, `src/cmuxParity.test.ts` on the TS side, both comparing
//! against the committed golden (`src/fixtures/cmux-plan-golden.json`). Drift
//! between the two importers breaks one of the suites BEFORE a release.
//!
//! PURE, like its TS twin: both source files arrive as STRINGS (`None` =
//! absent → a flat import from whatever exists), nothing here touches I/O,
//! and a malformed PRESENT file yields `Err` with a message naming the file —
//! before any state exists to touch. Per-entry damage is tolerated (a
//! non-workspace action, a workspace without a title, a group without an id —
//! skipped), per-file damage is not.
//!
//! Semantics mirrored one-to-one from cmuxImport.ts (read it for the full
//! story): the session store is PRIMARY (workspaces, order, panels→tabs,
//! groups, membership); cmux.json `actions` are SECONDARY (matching titles
//! skipped, the rest as extra FLAT workspaces, ids `action-N` in document
//! order); group-anchor workspaces are the hidden group headers and are
//! skipped; stale groupIds import flat. Apply is additive: `X` collides →
//! `X from cmux` → `X from cmux 2`…; members file into their groups; one umux
//! tab per cmux surface, named from the surface (positional "Tab N" when
//! untitled), the surface's directory onto its panel (workspace cwd as the
//! first tab's fallback).
//!
//! Document order matters: cmux.json `actions` iterate in DOCUMENT order (the
//! TS `Object.values` contract) — serde_json's `preserve_order` feature (set
//! in store_core's Cargo.toml) is what makes the `action-N` ids and the merged
//! plan order match the reference.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workspace_store::{Group, LayoutNode, Panel, Tab, Workspace, WorkspaceData};

// --- plan model (serialized with the TS field names — the parity contract) --

/// One cmux surface → ONE umux tab: an optional surface title (absent =
/// umux's positional "Tab N") and the surface's directory.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct CmuxImportTab {
    pub name: Option<String>,
    pub directory: Option<String>,
}

/// One workspace to import. `id` is the SOURCE id (referencing group
/// membership) — umux assigns its own ids at apply time. `color` (#73) is
/// the cmux workspace's `customColor` ALREADY mapped to the nearest umux
/// palette hex at parse time; None = cmux had no (readable) color.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct CmuxImportWorkspace {
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub tabs: Vec<CmuxImportTab>,
    pub color: Option<String>,
}

/// One group to import, with its members' source ids. Flat, like cmux's.
/// `color` (#73): the group's cmux accent color mapped to the umux palette,
/// same as the workspace field.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CmuxImportGroup {
    pub id: String,
    pub name: String,
    pub collapsed: bool,
    pub pinned: bool,
    pub member_ids: Vec<String>,
    pub color: Option<String>,
}

/// The umux fixed palette (#69) — the ONLY colors the app offers, mirrored
/// from the TS `COLOR_PALETTE` in workspaces.ts (order matters for the
/// nearest-match tie-break).
const IMPORT_PALETTE: [&str; 8] = [
    "#4ade80", // light green
    "#16a34a", // dark green
    "#60a5fa", // light blue
    "#2563eb", // dark blue
    "#eab308", // yellow
    "#ef4444", // red
    "#ec4899", // pink
    "#a855f7", // purple
];

/// Map a cmux accent color (#73) onto the nearest umux palette hex: cmux
/// lets the user pick ANY color, umux offers the fixed eight, so an imported
/// color lands on its closest palette entry (squared Euclidean RGB distance;
/// a tie picks the earlier palette entry — deterministic, same contract as
/// the TS `nearestPaletteColor`). Only strict `#RRGGBB` (case-insensitive)
/// is accepted; anything else imports as NO color.
fn map_cmux_color(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let bytes = raw.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return None;
    }
    if !bytes[1..].iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let int = u32::from_str_radix(&raw[1..], 16).ok()?;
    // Signed channels — nearest-RGB differences go negative all the time.
    let r = ((int >> 16) & 0xff) as i32;
    let g = ((int >> 8) & 0xff) as i32;
    let b = (int & 0xff) as i32;
    let mut best: Option<(i32, &str)> = None;
    for hex in IMPORT_PALETTE {
        let pi = u32::from_str_radix(&hex[1..], 16).expect("palette hexes are strict");
        let dr = r - ((pi >> 16) & 0xff) as i32;
        let dg = g - ((pi >> 8) & 0xff) as i32;
        let db = b - (pi & 0xff) as i32;
        let dist = dr * dr + dg * dg + db * db;
        if best.map_or(true, |(bd, _)| dist < bd) {
            best = Some((dist, hex));
        }
    }
    best.map(|(_, hex)| hex.to_string())
}

/// The parse output: everything the sources describe, in source order.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct CmuxImportPlan {
    pub workspaces: Vec<CmuxImportWorkspace>,
    pub groups: Vec<CmuxImportGroup>,
}

// --- parse -------------------------------------------------------------------

/// Parse the two source FILES (as strings; `None` = file absent) into an
/// import plan. Errors name the offending file, mirroring the TS messages —
/// per-FILE damage only; damaged entries are skipped.
pub fn parse_cmux_sources(
    config_json: Option<&str>,
    session_json: Option<&str>,
) -> Result<CmuxImportPlan, String> {
    let config = parse_json_file(config_json, "cmux.json")?;
    let session = parse_json_file(session_json, "cmux session store")?;

    // --- cmux.json: workspace actions (secondary source) ---------------------
    let mut action_workspaces: Vec<CmuxImportWorkspace> = Vec::new();
    if let Some(config) = config {
        let obj = config
            .as_object()
            .ok_or("cmux.json is malformed: expected an object at the top level")?;
        if let Some(actions) = obj.get("actions") {
            // Absent and null both mean "no actions" (the TS `!= null` guard;
            // note `typeof null === "object"`, so null must not error).
            if !actions.is_null() {
                let actions_obj = actions.as_object().ok_or(
                    "cmux.json is malformed: \"actions\" must be an object",
                )?;
                // Document order (preserve_order) — the `action-N` ids and the
                // merged plan order hang on it.
                let mut extra = 0usize;
                for entry in actions_obj.values() {
                    let Some(entry) = entry.as_object() else { continue };
                    if entry.get("type").and_then(Value::as_str) != Some("workspace") {
                        continue;
                    }
                    let Some(ws) = entry.get("workspace").and_then(Value::as_object) else {
                        continue;
                    };
                    let Some(name) = non_empty(ws.get("name").and_then(Value::as_str)) else {
                        continue;
                    };
                    // Each declared surface becomes its own TAB, named from
                    // the surface; no `surfaces` array = one unnamed surface.
                    // Every action tab carries the workspace's cwd.
                    let pane = ws
                        .get("layout")
                        .and_then(Value::as_object)
                        .and_then(|l| l.get("pane").and_then(Value::as_object));
                    let surfaces = pane.and_then(|p| p.get("surfaces")).and_then(Value::as_array);
                    let cwd = ws.get("cwd").and_then(Value::as_str).map(String::from);
                    let tabs: Vec<CmuxImportTab> = match surfaces {
                        Some(list) => list
                            .iter()
                            .map(|s| CmuxImportTab {
                                name: s
                                    .as_object()
                                    .and_then(|s| s.get("name").and_then(Value::as_str))
                                    .and_then(trimmed),
                                directory: cwd.clone(),
                            })
                            .collect(),
                        None => vec![CmuxImportTab { name: None, directory: cwd.clone() }],
                    };
                    action_workspaces.push(CmuxImportWorkspace {
                        id: format!("action-{extra}"),
                        title: name,
                        cwd,
                        tabs,
                        // cmux.json actions carry no colors — the session
                        // store is the only color source (#73).
                        color: None,
                    });
                    extra += 1;
                }
            }
        }
    }

    // --- session store: workspaces + flat groups (primary source) ------------
    let mut workspaces: Vec<CmuxImportWorkspace> = Vec::new();
    let mut groups: Vec<CmuxImportGroup> = Vec::new();
    let mut group_ids: std::collections::HashSet<String> = Default::default();
    let mut session_titles: std::collections::HashSet<String> = Default::default();
    // cmux backs every group's header row with a hidden ANCHOR workspace —
    // the group's `anchorWorkspaceId` names it; those entries are SKIPPED.
    let mut anchor_ids: std::collections::HashSet<String> = Default::default();
    if let Some(session) = session {
        let obj = session.as_object().ok_or(
            "cmux session store is malformed: expected an object at the top level",
        )?;
        let windows = obj.get("windows");
        if let Some(w) = windows {
            if !w.is_null() && !w.is_array() {
                return Err("cmux session store is malformed: \"windows\" must be an array".into());
            }
        }
        // `windows[0]?.tabManager ?? null` — a non-object window or a missing
        // key reads as "no tab manager", not an error (the TS `?.` chain).
        let tm = windows
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|first| first.get("tabManager"))
            .cloned();
        if let Some(tm) = tm.filter(|tm| !tm.is_null()) {
            let tm = tm
                .as_object()
                .ok_or("cmux session store is malformed: \"tabManager\" must be an object")?;
            let raw_workspaces = tm.get("workspaces").and_then(Value::as_array);
            let raw_groups = tm.get("workspaceGroups").and_then(Value::as_array);

            for raw in raw_groups.into_iter().flatten() {
                let Some(g) = raw.as_object() else { continue };
                let Some(id) = g.get("id").and_then(Value::as_str) else { continue };
                let Some(name) = non_empty(g.get("name").and_then(Value::as_str)) else {
                    continue;
                };
                group_ids.insert(id.to_string());
                if let Some(anchor) = g.get("anchorWorkspaceId").and_then(Value::as_str) {
                    anchor_ids.insert(anchor.to_string());
                }
                groups.push(CmuxImportGroup {
                    id: id.to_string(),
                    name,
                    collapsed: g.get("isCollapsed") == Some(&Value::Bool(true)),
                    pinned: g.get("isPinned") == Some(&Value::Bool(true)),
                    member_ids: Vec::new(),
                    // #73: the group's cmux accent color → nearest palette hex.
                    color: map_cmux_color(g.get("customColor").and_then(Value::as_str)),
                });
            }

            for (i, raw) in raw_workspaces.into_iter().flatten().enumerate() {
                let Some(w) = raw.as_object() else { continue };
                let Some(title) = non_empty(w.get("customTitle").and_then(Value::as_str)) else {
                    continue;
                };
                let ws_id = match w.get("workspaceId").and_then(Value::as_str) {
                    Some(id) => id.to_string(),
                    None => format!("session-{i}"),
                };
                // A group's anchor is the hidden group header, not a real
                // workspace — importing it would surface a row nobody had.
                if anchor_ids.contains(&ws_id) {
                    continue;
                }
                let cwd = w.get("currentDirectory").and_then(Value::as_str).map(String::from);
                let raw_panels = w.get("panels").and_then(Value::as_array);
                // One TAB per cmux surface (panel), named from its title; no
                // panels = one unnamed tab carrying the workspace's cwd.
                let tabs = match raw_panels.filter(|p| !p.is_empty()) {
                    Some(panels) => panels
                        .iter()
                        .map(|p| {
                            let rec = p.as_object();
                            CmuxImportTab {
                                name: rec
                                    .and_then(|r| r.get("title").and_then(Value::as_str))
                                    .and_then(trimmed),
                                directory: rec
                                    .and_then(|r| r.get("directory").and_then(Value::as_str))
                                    .map(String::from),
                            }
                        })
                        .collect(),
                    None => vec![CmuxImportTab { name: None, directory: cwd.clone() }],
                };
                session_titles.insert(title.clone());
                let gid = w.get("groupId").and_then(Value::as_str);
                match gid {
                    Some(gid) if group_ids.contains(gid) => {
                        if let Some(group) = groups.iter_mut().find(|g| g.id == gid) {
                            group.member_ids.push(ws_id.clone());
                        }
                    }
                    // A stale groupId (no matching group) imports FLAT — the
                    // same treatment bootState gives one on load.
                    _ => {}
                }
                workspaces.push(CmuxImportWorkspace {
                    id: ws_id,
                    title,
                    cwd,
                    tabs,
                    // #73: the workspace's cmux accent color → nearest palette hex.
                    color: map_cmux_color(w.get("customColor").and_then(Value::as_str)),
                });
            }
        }
    }

    // --- merge: session primary, non-colliding actions as extra flat rows ----
    let mut extra_seq = 0usize;
    for mut action in action_workspaces {
        if session_titles.contains(&action.title) {
            continue;
        }
        action.id = format!("{}-{}", action.id, extra_seq);
        extra_seq += 1;
        workspaces.push(action);
    }

    Ok(CmuxImportPlan { workspaces, groups })
}

fn parse_json_file(text: Option<&str>, label: &str) -> Result<Option<Value>, String> {
    match text {
        None => Ok(None),
        Some(text) => serde_json::from_str(text)
            .map(Some)
            .map_err(|_| format!("{label} is malformed: not valid JSON")),
    }
}

/// The TS `typeof x === "string" && x !== ""` guard.
fn non_empty(value: Option<&str>) -> Option<String> {
    match value {
        Some(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

/// The TS `typeof x === "string" && x.trim() !== "" ? x.trim() : null` guard:
/// a blank-only name imports unnamed, a real name imports TRIMMED.
fn trimmed(value: &str) -> Option<String> {
    let t = value.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

// --- apply -------------------------------------------------------------------

/// A name that survives collision with EVERYTHING already in `existing`:
/// `X`, then `X from cmux`, then `X from cmux 2`, `X from cmux 3`, … Nothing
/// is ever overwritten.
pub fn unique_name(existing: &std::collections::HashSet<&str>, base: &str) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{base} from cmux")
        } else {
            format!("{base} from cmux {n}")
        };
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

/// The default display name for a NEW tab in a workspace: "Tab N" past any
/// taken number (browser-style stable numbering — the TS nextTabName).
fn next_tab_name(tabs: &[Tab]) -> String {
    let taken: std::collections::HashSet<&str> =
        tabs.iter().filter_map(|t| t.name.as_deref()).collect();
    let mut n = tabs.len() + 1;
    while taken.contains(format!("Tab {n}").as_str()) {
        n += 1;
    }
    format!("Tab {n}")
}

/// Apply an import plan to a store state (the pure tree-op equivalent of the
/// TS applyImportPlan): groups land flat at the top level (flags carried over
/// only when true — false stays ABSENT on the wire), workspaces land in
/// source order with one named tab per cmux surface, members file into their
/// groups, colliding names get the ` from cmux` suffix. Returns a NEW state;
/// nothing here touches I/O. `gen_id` supplies the umux-side node ids (the
/// CLI's clock/pid generator; tests pass a deterministic sequence) — ids
/// never enter the parity comparison.
pub fn apply_import_plan(
    data: &WorkspaceData,
    plan: &CmuxImportPlan,
    gen_id: &mut dyn FnMut() -> String,
) -> WorkspaceData {
    if plan.workspaces.is_empty() && plan.groups.is_empty() {
        return data.clone();
    }
    let mut next = data.clone();

    // Groups first, flat at the top level.
    let mut group_id_map: HashMap<String, String> = HashMap::new();
    for g in &plan.groups {
        let taken: std::collections::HashSet<&str> =
            next.groups.iter().map(|x| x.name.as_str()).collect();
        let name = unique_name(&taken, &g.name);
        let gid = gen_id();
        next.groups.push(Group {
            id: gid.clone(),
            name,
            collapsed: g.collapsed.then_some(true),
            pinned: g.pinned.then_some(true),
            parent_id: None,
            // #73: the imported group's palette color from the plan.
            color: g.color.clone(),
        });
        next.order.push(gid.clone());
        group_id_map.insert(g.id.clone(), gid);
    }

    // plan ws id -> plan group id (cmux groups are flat; the TS map's
    // `set` means a workspace claimed by two groups lands in the LAST one).
    let mut owner_of: HashMap<&str, &str> = HashMap::new();
    for g in &plan.groups {
        for member in &g.member_ids {
            owner_of.insert(member.as_str(), g.id.as_str());
        }
    }

    for w in &plan.workspaces {
        let taken: std::collections::HashSet<&str> =
            next.workspaces.iter().map(|x| x.name.as_str()).collect();
        let name = unique_name(&taken, &w.title);
        let ws_id = gen_id();
        // The seed tab (born named "Tab 1"), then one extra tab per further
        // surface — all with positional names; the surface names land after.
        let mut tabs: Vec<Tab> = vec![Tab {
            id: gen_id(),
            layout: Some(LayoutNode::Leaf { id: gen_id() }),
            name: Some("Tab 1".into()),
            pinned: None,
            color: None,
        }];
        for _ in 1..w.tabs.len() {
            tabs.push(Tab {
                id: gen_id(),
                layout: Some(LayoutNode::Leaf { id: gen_id() }),
                name: Some(next_tab_name(&tabs)),
                pinned: None,
                color: None,
            });
        }
        next.workspaces.push(Workspace {
            id: ws_id.clone(),
            name,
            panels: Vec::new(),
            layout: None,
            tabs,
            pinned: None,
            group_id: None,
            // #73: the imported workspace's palette color from the plan.
            color: w.color.clone(),
        });
        next.order.push(ws_id.clone());

        // Per-tab: surface name (an untitled surface keeps its positional
        // name) and the surface's directory onto its panel — the workspace's
        // cwd as the fallback for the FIRST tab only.
        let imported = next.workspaces.last_mut().expect("just pushed");
        for (i, source) in w.tabs.iter().enumerate() {
            let tab = &mut imported.tabs[i];
            if let Some(name) = &source.name {
                tab.name = Some(name.clone());
            }
            let directory = if i == 0 {
                source.directory.clone().or_else(|| w.cwd.clone())
            } else {
                source.directory.clone()
            }
            .filter(|d| !d.is_empty());
            if let (Some(dir), Some(LayoutNode::Leaf { id })) =
                (directory, tab.layout.as_ref())
            {
                upsert_panel_cwd(&mut imported.panels, id, &dir);
            }
        }

        if let Some(plan_gid) = owner_of.get(w.id.as_str()) {
            if let Some(umux_gid) = group_id_map.get(*plan_gid) {
                move_into_group(&mut next, &ws_id, umux_gid);
            }
        }
    }
    next
}

/// The TS upsertPanelCwd narrowed to one workspace's config panels: set the
/// panel's workingDirectory, creating the record when absent; empty is
/// already filtered by the caller.
fn upsert_panel_cwd(panels: &mut Vec<Panel>, panel_id: &str, cwd: &str) {
    match panels.iter_mut().find(|p| p.id == panel_id) {
        Some(panel) => panel.working_directory = Some(cwd.to_string()),
        None => panels.push(Panel {
            id: panel_id.to_string(),
            working_directory: Some(cwd.to_string()),
            ssh_target: None,
        }),
    }
}

/// File a workspace into a group: appended at the END of that group's
/// children in the shared display order — the TS moveNode append branch.
fn move_into_group(data: &mut WorkspaceData, node_id: &str, parent_id: &str) {
    let parent_of = |data: &WorkspaceData, node: &str| -> Option<String> {
        data.groups
            .iter()
            .find(|g| g.id == node)
            .and_then(|g| g.parent_id.clone())
            .or_else(|| {
                data.workspaces
                    .iter()
                    .find(|w| w.id == node)
                    .and_then(|w| w.group_id.clone())
            })
    };
    let without: Vec<String> = data
        .order
        .iter()
        .filter(|id| id.as_str() != node_id)
        .cloned()
        .collect();
    // After the parent's LAST child; with no children yet, right after the
    // parent itself; a parent missing from `order` appends at the very end
    // (the TS moveNode's append branch, mirrored branch for branch).
    let insert_at = match without
        .iter()
        .rposition(|id| parent_of(data, id).as_deref() == Some(parent_id))
    {
        Some(last) => last + 1,
        None => without
            .iter()
            .position(|id| id == parent_id)
            .map_or(without.len(), |p| p + 1),
    };
    let mut order = without;
    order.insert(insert_at, node_id.to_string());
    data.order = order;
    if let Some(w) = data.workspaces.iter_mut().find(|w| w.id == node_id) {
        w.group_id = Some(parent_id.to_string());
    }
}

// --- preview (--dry-run's plan; the parity tree) ------------------------------

/// One imported workspace row: the FINAL (collision-resolved) name, the final
/// group name, and the tab count.
#[derive(Serialize, PartialEq, Debug, Clone)]
pub struct PreviewWorkspace {
    pub name: String,
    pub group_name: Option<String>,
    pub tab_count: usize,
}

/// One imported group row with how many planned workspaces file into it.
#[derive(Serialize, PartialEq, Debug, Clone)]
pub struct PreviewGroup {
    pub name: String,
    pub child_count: usize,
}

/// The would-be result, summarized for `--dry-run`: the planned state plus
/// the rows the import would ADD (everything not already in `live`).
#[derive(Serialize, PartialEq, Debug, Clone)]
pub struct ImportPreview {
    pub planned: WorkspaceData,
    pub workspaces: Vec<PreviewWorkspace>,
    pub groups: Vec<PreviewGroup>,
}

/// Dry-run an import plan against the live state: the caller applies the
/// plan once (`apply_import_plan`) and hands both states in, and the new
/// rows are summarized — the Rust twin of the wizard's buildImportPreview.
/// Nothing is persisted here.
pub fn build_import_preview(live: &WorkspaceData, planned: &WorkspaceData) -> ImportPreview {
    let live_ws: std::collections::HashSet<&str> =
        live.workspaces.iter().map(|w| w.id.as_str()).collect();
    let live_groups: std::collections::HashSet<&str> =
        live.groups.iter().map(|g| g.id.as_str()).collect();
    let group_name_of = |gid: &Option<String>| -> Option<String> {
        gid.as_ref()
            .and_then(|id| planned.groups.iter().find(|g| &g.id == id))
            .map(|g| g.name.clone())
    };
    ImportPreview {
        planned: planned.clone(),
        workspaces: planned
            .workspaces
            .iter()
            .filter(|w| !live_ws.contains(w.id.as_str()))
            .map(|w| PreviewWorkspace {
                name: w.name.clone(),
                group_name: group_name_of(&w.group_id),
                tab_count: w.tabs.len(),
            })
            .collect(),
        groups: planned
            .groups
            .iter()
            .filter(|g| !live_groups.contains(g.id.as_str()))
            .map(|g| PreviewGroup {
                name: g.name.clone(),
                child_count: planned
                    .workspaces
                    .iter()
                    .filter(|w| w.group_id.as_ref() == Some(&g.id))
                    .count(),
            })
            .collect(),
    }
}

/// One node of the sidebar-like preview tree: a group with its imported
/// children nested, or a flat workspace.
#[derive(Serialize, PartialEq, Debug, Clone)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PreviewNode {
    Group {
        name: String,
        child_count: usize,
        children: Vec<PreviewChild>,
    },
    Workspace {
        name: String,
        tab_count: usize,
    },
}

/// One imported workspace nested under its group.
#[derive(Serialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PreviewChild {
    pub name: String,
    pub tab_count: usize,
}

/// Shape the preview rows into the sidebar-like tree: groups first (in
/// planned order) with their imported members nested, then group-less
/// workspaces in source order — the same picture the sidebar shows after a
/// real import.
pub fn build_preview_tree(preview: &ImportPreview) -> Vec<PreviewNode> {
    let mut nodes = Vec::new();
    for g in &preview.groups {
        let children: Vec<PreviewChild> = preview
            .workspaces
            .iter()
            .filter(|w| w.group_name.as_deref() == Some(g.name.as_str()))
            .map(|w| PreviewChild {
                name: w.name.clone(),
                tab_count: w.tab_count,
            })
            .collect();
        nodes.push(PreviewNode::Group {
            name: g.name.clone(),
            child_count: children.len(),
            children,
        });
    }
    for w in &preview.workspaces {
        if w.group_name.is_none() {
            nodes.push(PreviewNode::Workspace {
                name: w.name.clone(),
                tab_count: w.tab_count,
            });
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic id sequence for apply tests.
    fn seq_ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("gen-{n}")
        }
    }

    // T-P1 (per-file damage: a PRESENT but invalid config errors, naming the
    // file — before any state exists):
    #[test]
    fn malformed_config_names_the_file() {
        let err = parse_cmux_sources(Some("{ not json"), None).unwrap_err();
        assert!(err.contains("cmux.json"), "{err}");
        assert!(err.contains("not valid JSON"), "{err}");
    }

    // T-P2 (per-file damage: wrong top-level shape):
    #[test]
    fn non_object_config_names_the_file() {
        let err = parse_cmux_sources(Some("[]"), None).unwrap_err();
        assert_eq!(
            err,
            "cmux.json is malformed: expected an object at the top level"
        );
    }

    // T-P3 (per-file damage: "actions" of the wrong shape):
    #[test]
    fn non_object_actions_error() {
        let err = parse_cmux_sources(Some(r#"{"actions": 5}"#), None).unwrap_err();
        assert_eq!(err, "cmux.json is malformed: \"actions\" must be an object");
    }

    // T-P4 (absent sources are NORMAL — a flat import from whatever exists):
    #[test]
    fn absent_sources_yield_an_empty_plan() {
        let plan = parse_cmux_sources(None, None).unwrap();
        assert!(plan.workspaces.is_empty());
        assert!(plan.groups.is_empty());
    }

    // T-P5 (per-entry damage is tolerated: junk entries skipped):
    #[test]
    fn damaged_entries_are_skipped() {
        let config = r#"{"actions": {
            "a": {"type": "command"},
            "b": {"type": "workspace", "workspace": {"name": ""}},
            "c": {"type": "workspace", "workspace": {"name": "Solo", "cwd": "/tmp",
                   "layout": {"pane": {"surfaces": [{"name": "  "}, {"name": " Main "}]}}}}
        }}"#;
        let plan = parse_cmux_sources(Some(config), None).unwrap();
        assert_eq!(plan.workspaces.len(), 1);
        // The TS merge ALWAYS appends the -<seq> counter, even with no
        // session workspaces alongside — "action-0-0", not "action-0".
        assert_eq!(plan.workspaces[0].id, "action-0-0");
        assert_eq!(plan.workspaces[0].title, "Solo");
        assert_eq!(plan.workspaces[0].tabs.len(), 2);
        assert_eq!(plan.workspaces[0].tabs[0].name, None, "blank name imports unnamed");
        assert_eq!(plan.workspaces[0].tabs[1].name.as_deref(), Some("Main"), "trimmed");
    }

    // T-P6 (the session store is primary and anchors are skipped):
    #[test]
    fn session_groups_claim_members_and_anchors_are_skipped() {
        let session = r#"{"windows": [{"tabManager": {
            "workspaceGroups": [
                {"id": "g1", "name": "Grupa", "isCollapsed": true, "isPinned": true,
                 "anchorWorkspaceId": "anchor-1"}
            ],
            "workspaces": [
                {"workspaceId": "anchor-1", "customTitle": "Grupa 1"},
                {"workspaceId": "w1", "customTitle": "Real", "groupId": "g1",
                 "currentDirectory": "/x", "panels": [{"title": "Main", "directory": "/x"}]},
                {"workspaceId": "w2", "customTitle": "Stale", "groupId": "nope"}
            ]
        }}]}"#;
        let plan = parse_cmux_sources(None, Some(session)).unwrap();
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].member_ids, ["w1"]);
        assert!(plan.groups[0].collapsed && plan.groups[0].pinned);
        // w2's stale groupId imports FLAT; the anchor never becomes a row.
        assert_eq!(plan.workspaces.len(), 2);
        assert_eq!(plan.workspaces[0].id, "w1");
        assert_eq!(plan.workspaces[1].id, "w2");
    }

    // T-P7 (actions merge: matching titles skipped, ids in document order):
    #[test]
    fn actions_merge_after_the_session() {
        let config = r#"{"actions": {
            "first": {"type": "workspace", "workspace": {"name": "Project A", "cwd": "/a",
                      "layout": {"pane": {"surfaces": [{"name": "S1"}, {"name": "S2"}]}}}},
            "second": {"type": "workspace", "workspace": {"name": "Extra", "cwd": "/e"}}
        }}"#;
        let session = r#"{"windows": [{"tabManager": {
            "workspaces": [{"workspaceId": "w1", "customTitle": "Project A"}]
        }}]}"#;
        let plan = parse_cmux_sources(Some(config), Some(session)).unwrap();
        assert_eq!(plan.workspaces.len(), 2);
        assert_eq!(plan.workspaces[0].id, "w1", "session first");
        // "Project A" action skipped (session wins); "Extra" lands as
        // action-1-0 (action-1 because the FIRST action claimed id action-0).
        assert_eq!(plan.workspaces[1].id, "action-1-0");
        assert_eq!(plan.workspaces[1].title, "Extra");
        assert_eq!(plan.workspaces[1].tabs[0].directory.as_deref(), Some("/e"));
    }

    // T-U1 (collision suffixes: X, X from cmux, X from cmux 2):
    #[test]
    fn unique_name_suffixes_are_numbered() {
        let taken: std::collections::HashSet<&str> =
            ["Proj", "Proj from cmux", "Proj from cmux 2"].into_iter().collect();
        assert_eq!(unique_name(&Default::default(), "Fresh"), "Fresh");
        assert_eq!(unique_name(&taken, "Proj"), "Proj from cmux 3");
    }

    // T-U2 (apply: an empty plan is a no-op):
    #[test]
    fn empty_plan_changes_nothing() {
        let data = WorkspaceData::default();
        let out = apply_import_plan(&data, &parse_cmux_sources(None, None).unwrap(), &mut seq_ids());
        assert_eq!(out, data);
    }

    // T-U3 (apply: suffixes, one tab per surface, cwd onto panels, grouping
    // and the shared order):
    #[test]
    fn apply_imports_groups_tabs_directories_and_suffixes() {
        let session = r#"{"windows": [{"tabManager": {
            "workspaceGroups": [{"id": "g1", "name": "Grupa", "isCollapsed": true}],
            "workspaces": [
                {"workspaceId": "w1", "customTitle": "Real", "groupId": "g1",
                 "panels": [{"title": "Main", "directory": "/x"}]},
                {"workspaceId": "w2", "customTitle": "Real", "groupId": "g1",
                 "panels": []}
            ]
        }}]}"#;
        let plan = parse_cmux_sources(None, Some(session)).unwrap();
        let live = WorkspaceData {
            workspaces: vec![Workspace {
                id: "live-1".into(),
                name: "Real".into(),
                ..Default::default()
            }],
            order: vec!["live-1".into()],
            ..Default::default()
        };

        let out = apply_import_plan(&live, &plan, &mut seq_ids());

        // Names: "Real" taken → "Real from cmux" then "Real from cmux 2".
        assert_eq!(
            out.workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["Real", "Real from cmux", "Real from cmux 2"]
        );
        // The first import: one named tab + its panel carries the directory.
        let first = &out.workspaces[1];
        assert_eq!(first.tabs.len(), 1);
        assert_eq!(first.tabs[0].name.as_deref(), Some("Main"));
        assert_eq!(first.panels.len(), 1);
        assert_eq!(first.panels[0].working_directory.as_deref(), Some("/x"));
        // The second: no panels → one unnamed tab, cwd fallback null → NO
        // panel record at all.
        let second = &out.workspaces[2];
        assert_eq!(second.tabs.len(), 1);
        assert_eq!(second.tabs[0].name.as_deref(), Some("Tab 1"));
        assert!(second.panels.is_empty());
        // Both file into the imported group; order interleaves correctly.
        let group = &out.groups[0];
        assert_eq!(group.name, "Grupa");
        assert_eq!(group.collapsed, Some(true));
        let group_id = group.id.as_str();
        assert!(out
            .workspaces
            .iter()
            .filter(|w| w.group_id.as_deref() == Some(group_id))
            .count()
            == 2);
        assert_eq!(out.order.len(), out.workspaces.len() + out.groups.len());
    }

    // --- Colors from cmux (#73) ------------------------------------------------

    #[test]
    fn debug_json_variants() {
        println!(
            "compact: {:?}",
            serde_json::from_str::<Value>(
                r##"{"windows": [{"tabManager": {"workspaceGroups": [], "workspaces": []}}]}"##
            )
        );
        println!(
            "multiline: {:?}",
            serde_json::from_str::<Value>(r##"{"windows": [{"tabManager": {
            "workspaceGroups": [
                {"id": "g1", "name": "g", "customColor": "#ec4899"}
            ],
            "workspaces": [
                {"workspaceId": "w1", "customTitle": "k", "customColor": "#ff0000"}
            ]
        }}]}}"##)
        );
    }

    // T-C73-1 (map_cmux_color: an exact palette hex imports 1:1, an
    // off-palette one lands on its NEAREST palette entry, garbage imports as
    // NO color; case-insensitive):
    #[test]
    fn map_cmux_color_matches_and_maps() {
        // Exact palette entries pass through untouched.
        assert_eq!(map_cmux_color(Some("#ec4899")).as_deref(), Some("#ec4899"));
        assert_eq!(map_cmux_color(Some("#EAB308")).as_deref(), Some("#eab308"));
        // Off-palette: pure red lands on the palette's red (#ef4444).
        assert_eq!(map_cmux_color(Some("#ff0000")).as_deref(), Some("#ef4444"));
        // Garbage and non-hex forms import as NO color.
        assert_eq!(map_cmux_color(Some("red")), None);
        assert_eq!(map_cmux_color(Some("#fff")), None);
        assert_eq!(map_cmux_color(Some("")), None);
        assert_eq!(map_cmux_color(None), None);
    }

    // T-C73-2 (parse: workspace AND group customColor from the session store
    // land on the plan, mapped to the palette; a workspace without the field
    // imports colorless):
    #[test]
    fn parse_reads_custom_color_from_session() {
        let session = r##"{"windows": [{"tabManager": {
            "workspaceGroups": [
                {"id": "g1", "name": "Grupa", "customColor": "#ec4899"},
                {"id": "g2", "name": "Bez", "customColor": "nope"}
            ],
            "workspaces": [
                {"workspaceId": "w1", "customTitle": "Kolorowy", "customColor": "#ff0000"},
                {"workspaceId": "w2", "customTitle": "Czysty"}
            ]
        }}]}"##;
        let plan = parse_cmux_sources(None, Some(session)).unwrap();

        assert_eq!(plan.groups.len(), 2);
        assert_eq!(plan.groups[0].color.as_deref(), Some("#ec4899"));
        assert_eq!(plan.groups[1].color, None, "garbage imports colorless");
        assert_eq!(plan.workspaces.len(), 2);
        // Off-palette red → nearest palette entry (red #ef4444).
        assert_eq!(plan.workspaces[0].color.as_deref(), Some("#ef4444"));
        assert_eq!(plan.workspaces[1].color, None);
    }

    // T-C73-3 (apply: the plan color lands on the created Workspace and
    // Group — persisted by the ordinary save flow):
    #[test]
    fn apply_carries_color_onto_the_created_nodes() {
        let session = r##"{"windows": [{"tabManager": {
            "workspaceGroups": [{"id": "g1", "name": "Grupa", "customColor": "#a855f7"}],
            "workspaces": [
                {"workspaceId": "w1", "customTitle": "Kolorowy",
                 "customColor": "#ec4899", "groupId": "g1"}
            ]
        }}]}"##;
        let plan = parse_cmux_sources(None, Some(session)).unwrap();

        let out = apply_import_plan(&WorkspaceData::default(), &plan, &mut seq_ids());

        assert_eq!(out.workspaces[0].color.as_deref(), Some("#ec4899"));
        assert_eq!(out.groups[0].color.as_deref(), Some("#a855f7"));
    }
}
