// model — the TUI's saved state over the TERM store (B3, v1.9.0).
//
// Workspaces, groups, tabs and pane trees, read at startup from the term
// store (`store_core::paths::term_config_path`) and persisted on every
// structural change through `store_core` ONLY — no direct file I/O here (the
// CLI-write/app-write/TUI-write-one-code-path rule). The TUI never touches
// the desktop store's paths: both stores' locations come from
// `store_core::paths`, and only the `term_*` ones are used (the separation
// the B7 AC pins).
//
// Id generation mirrors the CLI's (`Ids` — clock/pid-seeded xorshift, v4
// format): the store treats ids as opaque strings.

use std::path::PathBuf;

use store_core::settings_store::{Settings, SettingsStore};
use store_core::workspace_store::{
    Group, LayoutNode, Orientation, Tab, Workspace, WorkspaceData, WorkspaceStore,
};

/// One sidebar row: a group header or a workspace (with its group, for the
/// indent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEntry {
    Group {
        id: String,
        name: String,
        collapsed: bool,
        color: Option<String>,
    },
    Workspace {
        id: String,
        name: String,
        group: Option<String>,
        pinned: bool,
        color: Option<String>,
    },
}

/// Clock/pid-seeded id generator (the CLI's shape, dependency-free).
pub struct Ids(u64);

impl Ids {
    pub fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Ids(nanos ^ ((std::process::id() as u64) << 32) ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A random v4-format UUID string (version/variant bits set).
    pub fn next(&mut self) -> String {
        let a = self.next_u64().to_le_bytes();
        let b = self.next_u64().to_le_bytes();
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&a);
        bytes[8..].copy_from_slice(&b);
        let mut hex = String::with_capacity(36);
        for (i, byte) in bytes.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                hex.push('-');
            }
            hex.push_str(&format!("{byte:02x}"));
        }
        hex.replace_range(14..15, "4");
        hex.replace_range(19..20, "8");
        let variant = ["8", "9", "a", "b"][(self.next_u64() % 4) as usize];
        hex.replace_range(24..25, variant);
        hex
    }
}

impl Default for Ids {
    fn default() -> Self {
        Self::new()
    }
}

/// The TUI's saved state: the term store's data + the term settings, with
/// the mutation surface the TUI needs. Every mutation marks `dirty`, and
/// `persist` writes through the store wrappers — one code path with the CLI
/// and the app.
pub struct TermModel {
    pub data: WorkspaceData,
    pub settings: Settings,
    ws_store: WorkspaceStore,
    settings_store: SettingsStore,
    dirty: bool,
}

impl TermModel {
    /// Open the TERM store (never the desktop one): paths come from
    /// `store_core::paths` only. A missing/corrupt file is the default
    /// state — the store's own fallback rule.
    pub fn open() -> Self {
        let ws_store = WorkspaceStore::new(store_core::paths::term_config_path());
        let settings_store = SettingsStore::new(store_core::paths::term_settings_path());
        TermModel {
            data: ws_store.load(),
            settings: settings_store.load(),
            ws_store,
            settings_store,
            dirty: false,
        }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Persist any pending structural change through `store_core` ONLY.
    pub fn persist(&mut self) {
        if self.dirty {
            let _ = self.ws_store.save(&self.data);
            self.dirty = false;
        }
    }

    /// Persist the settings half (the launch-mode key and friends).
    pub fn persist_settings(&self) {
        let _ = self.settings_store.save(&self.settings);
    }

    // --- workspaces ---------------------------------------------------------

    /// The next unique workspace name ("Workspace N", browser-style).
    pub fn next_workspace_name(&self) -> String {
        let taken: std::collections::HashSet<&str> =
            self.data.workspaces.iter().map(|w| w.name.as_str()).collect();
        let mut n = self.data.workspaces.len() + 1;
        while taken.contains(format!("Workspace {n}").as_str()) {
            n += 1;
        }
        format!("Workspace {n}")
    }

    /// Create a workspace with one tab of one pane; returns its id.
    pub fn create_workspace(&mut self, ids: &mut Ids) -> String {
        let name = self.next_workspace_name();
        let ws_id = ids.next();
        let tab = seed_tab(ids);
        self.data.workspaces.push(Workspace {
            id: ws_id.clone(),
            name,
            panels: vec![],
            layout: None,
            tabs: vec![tab],
            pinned: None,
            group_id: None,
            color: None,
        });
        self.data.order.push(ws_id.clone());
        self.dirty = true;
        ws_id
    }

    /// Rename a workspace by id (first match). Returns whether it existed.
    pub fn rename_workspace(&mut self, ws_id: &str, new_name: String) -> bool {
        match self.data.workspaces.iter_mut().find(|w| w.id == ws_id) {
            Some(w) => {
                w.name = new_name;
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    /// Delete a workspace (its tabs and panes with it) and its order entry.
    pub fn delete_workspace(&mut self, ws_id: &str) -> bool {
        let before = self.data.workspaces.len();
        self.data.workspaces.retain(|w| w.id != ws_id);
        if self.data.workspaces.len() != before {
            self.data.order.retain(|id| id != ws_id);
            self.dirty = true;
            return true;
        }
        false
    }

    pub fn workspace(&self, ws_id: &str) -> Option<&Workspace> {
        self.data.workspaces.iter().find(|w| w.id == ws_id)
    }

    pub fn workspace_mut(&mut self, ws_id: &str) -> Option<&mut Workspace> {
        self.data.workspaces.iter_mut().find(|w| w.id == ws_id)
    }

    // --- tabs ---------------------------------------------------------------

    /// Add a tab (one pane) to a workspace; returns its id.
    pub fn create_tab(&mut self, ws_id: &str, ids: &mut Ids) -> Option<String> {
        let ws = self.workspace_mut(ws_id)?;
        let tab = seed_tab(ids);
        let tab_id = tab.id.clone();
        ws.tabs.push(tab);
        self.dirty = true;
        Some(tab_id)
    }

    /// Rename a tab.
    pub fn rename_tab(&mut self, tab_id: &str, new_name: String) -> bool {
        for ws in &mut self.data.workspaces {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.name = Some(new_name);
                self.dirty = true;
                return true;
            }
        }
        false
    }

    /// Set (Some palette hex) or clear (None) a tab's color (#70).
    pub fn set_tab_color(&mut self, tab_id: &str, color: Option<String>) -> bool {
        for ws in &mut self.data.workspaces {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.color = color;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    pub fn tab_color(&self, tab_id: &str) -> Option<String> {
        self.data
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .find(|t| t.id == tab_id)
            .and_then(|t| t.color.clone())
    }

    pub fn group_color(&self, group_id: &str) -> Option<String> {
        self.data
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .and_then(|g| g.color.clone())
    }

    pub fn tab_pinned(&self, tab_id: &str) -> bool {
        self.data
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .find(|t| t.id == tab_id)
            .and_then(|t| t.pinned)
            .unwrap_or(false)
    }

    /// Pin / unpin a tab (pinned tabs render first).
    pub fn set_tab_pinned(&mut self, tab_id: &str, pinned: bool) -> bool {
        for ws in &mut self.data.workspaces {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.pinned = Some(pinned);
                self.dirty = true;
                return true;
            }
        }
        false
    }

    /// Close a tab (its pane sessions die in the runtime). Returns whether
    /// it existed.
    pub fn close_tab(&mut self, tab_id: &str) -> bool {
        for ws in &mut self.data.workspaces {
            let before = ws.tabs.len();
            ws.tabs.retain(|t| t.id != tab_id);
            if ws.tabs.len() != before {
                self.dirty = true;
                return true;
            }
        }
        false
    }

    // --- panes --------------------------------------------------------------

    /// Split the pane `leaf_id` of tab `tab_id` in `orientation`. Returns
    /// `(split_id, new_leaf_id)` or `None` when the tab/pane does not exist.
    pub fn split_pane(
        &mut self,
        tab_id: &str,
        leaf_id: &str,
        orientation: Orientation,
        ids: &mut Ids,
    ) -> Option<(String, String)> {
        let split_id = ids.next();
        let new_leaf = ids.next();
        for ws in &mut self.data.workspaces {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                let tree = tab.layout.clone().unwrap_or(LayoutNode::Leaf {
                    id: leaf_id.to_string(),
                });
                if let Some(next) =
                    crate::layout::split_leaf(&tree, leaf_id, orientation, &split_id, &new_leaf)
                {
                    tab.layout = Some(next);
                    self.dirty = true;
                    return Some((split_id, new_leaf));
                }
                return None;
            }
        }
        None
    }

    /// Close one pane of a tab (the sibling is promoted; the LAST pane of a
    /// tab belongs to closing the tab, so this refuses it).
    pub fn close_pane(&mut self, tab_id: &str, leaf_id: &str) -> bool {
        for ws in &mut self.data.workspaces {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                let Some(tree) = tab.layout.clone() else {
                    return false;
                };
                if let Some(next) = crate::layout::close_leaf(&tree, leaf_id) {
                    tab.layout = Some(next);
                    self.dirty = true;
                    return true;
                }
                return false;
            }
        }
        false
    }

    /// Persist a drag-resize (the pure layout op's result lands here).
    pub fn apply_layout(&mut self, tab_id: &str, tree: LayoutNode) -> bool {
        for ws in &mut self.data.workspaces {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.layout = Some(tree);
                self.dirty = true;
                return true;
            }
        }
        false
    }

    pub fn tab_layout(&self, tab_id: &str) -> Option<LayoutNode> {
        self.data
            .workspaces
            .iter()
            .flat_map(|w| w.tabs.iter())
            .find(|t| t.id == tab_id)
            .and_then(|t| t.layout.clone())
    }

    pub fn tab_of_pane(&self, leaf_id: &str) -> Option<String> {
        for ws in &self.data.workspaces {
            for tab in &ws.tabs {
                if let Some(tree) = &tab.layout {
                    if crate::layout::leaf_ids(tree).iter().any(|l| l == leaf_id) {
                        return Some(tab.id.clone());
                    }
                }
            }
        }
        None
    }

    /// Workspace ids in sidebar order (order entries that are workspaces).
    pub fn workspace_order(&self) -> Vec<String> {
        let known: std::collections::HashSet<&str> = self
            .data
            .workspaces
            .iter()
            .map(|w| w.id.as_str())
            .collect();
        let mut ids: Vec<String> = self
            .data
            .order
            .iter()
            .filter(|id| known.contains(id.as_str()))
            .cloned()
            .collect();
        // A workspace missing from `order` (pre-order configs) joins the end.
        for w in &self.data.workspaces {
            if !ids.contains(&w.id) {
                ids.push(w.id.clone());
            }
        }
        ids
    }

    /// One row of the sidebar tree: a group header or a workspace (with the
    /// group it sits under, for the indent).
    pub fn sidebar_tree(&self) -> Vec<SidebarEntry> {
        // The PO-mandated hierarchy (round 5): every group header first,
        // followed by ITS OWN workspaces (in sidebar order, indented by the
        // renderer), and the ungrouped workspaces below the groups.
        let mut groups_by_order: Vec<&Group> = self.data.groups.iter().collect();
        groups_by_order.sort_by_key(|g| {
            self.data
                .order
                .iter()
                .position(|id| id == &g.id)
                .unwrap_or(usize::MAX)
        });
        // A pinned group leads at its own level (#52) — a stable sort keeps
        // the relative order inside the two buckets.
        groups_by_order.sort_by_key(|g| !g.pinned.unwrap_or(false));
        let mut rows = Vec::new();
        for g in groups_by_order {
            rows.push(SidebarEntry::Group {
                id: g.id.clone(),
                name: g.name.clone(),
                collapsed: g.collapsed.unwrap_or(false),
                color: g.color.clone(),
            });
            let mut members: Vec<&Workspace> = self
                .data
                .order
                .iter()
                .filter_map(|id| self.data.workspaces.iter().find(|w| w.id == *id))
                .filter(|w| w.group_id.as_deref() == Some(g.id.as_str()))
                .collect();
            members.sort_by_key(|w| !w.pinned.unwrap_or(false));
            for w in members {
                rows.push(SidebarEntry::Workspace {
                    id: w.id.clone(),
                    name: w.name.clone(),
                    group: w.group_id.clone(),
                    pinned: w.pinned.unwrap_or(false),
                    color: w.color.clone(),
                });
            }
        }
        let mut ungrouped: Vec<&Workspace> = self
            .data
            .order
            .iter()
            .filter_map(|id| self.data.workspaces.iter().find(|w| w.id == *id))
            .filter(|w| w.group_id.is_none())
            .collect();
        ungrouped.sort_by_key(|w| !w.pinned.unwrap_or(false));
        for w in ungrouped {
            rows.push(SidebarEntry::Workspace {
                id: w.id.clone(),
                name: w.name.clone(),
                group: None,
                pinned: w.pinned.unwrap_or(false),
                color: w.color.clone(),
            });
        }
        // A workspace missing from `order` entirely (pre-order configs).
        for w in &self.data.workspaces {
            if !self.data.order.contains(&w.id) && w.group_id.is_none() {
                rows.push(SidebarEntry::Workspace {
                    id: w.id.clone(),
                    name: w.name.clone(),
                    group: None,
                    pinned: w.pinned.unwrap_or(false),
                    color: w.color.clone(),
                });
            }
        }
        rows
    }

    /// Pin / unpin a workspace (leads within its own container).
    pub fn set_workspace_pinned(&mut self, ws_id: &str, pinned: bool) -> bool {
        if let Some(w) = self.data.workspaces.iter_mut().find(|w| w.id == ws_id) {
            w.pinned = Some(pinned);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn workspace_pinned(&self, ws_id: &str) -> bool {
        self.data
            .workspaces
            .iter()
            .find(|w| w.id == ws_id)
            .and_then(|w| w.pinned)
            .unwrap_or(false)
    }

    /// Set (Some palette hex) or clear (None) a workspace color (#69).
    pub fn set_workspace_color(&mut self, ws_id: &str, color: Option<String>) -> bool {
        if let Some(w) = self.data.workspaces.iter_mut().find(|w| w.id == ws_id) {
            w.color = color;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn rename_group(&mut self, group_id: &str, new_name: String) -> bool {
        if let Some(g) = self.data.groups.iter_mut().find(|g| g.id == group_id) {
            g.name = new_name;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn set_group_pinned(&mut self, group_id: &str, pinned: bool) -> bool {
        if let Some(g) = self.data.groups.iter_mut().find(|g| g.id == group_id) {
            g.pinned = Some(pinned);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn group_pinned(&self, group_id: &str) -> bool {
        self.data
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .and_then(|g| g.pinned)
            .unwrap_or(false)
    }

    pub fn set_group_color(&mut self, group_id: &str, color: Option<String>) -> bool {
        if let Some(g) = self.data.groups.iter_mut().find(|g| g.id == group_id) {
            g.color = color;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Dissolve a group (#51): its workspaces return to the top level.
    pub fn unpack_group(&mut self, group_id: &str) -> bool {
        let exists = self.data.groups.iter().any(|g| g.id == group_id);
        if !exists {
            return false;
        }
        self.remove_group(group_id)
    }

    /// Delete a group AND every workspace inside it (the desktop\u{2019}s
    /// destructive subtree delete; the runtime kills the sessions first).
    pub fn delete_group_with_contents(&mut self, group_id: &str) -> Vec<String> {
        let members: Vec<String> = self
            .data
            .workspaces
            .iter()
            .filter(|w| w.group_id.as_deref() == Some(group_id))
            .map(|w| w.id.clone())
            .collect();
        for ws_id in &members {
            self.data.workspaces.retain(|w| w.id != *ws_id);
            self.data.order.retain(|id| id != ws_id);
        }
        self.remove_group(group_id);
        members
    }
    pub fn add_group(&mut self, id: String, name: String) {
        self.data.groups.push(Group {
            id: id.clone(),
            name,
            collapsed: Some(false),
            pinned: None,
            parent_id: None,
            color: None,
        });
        self.data.order.push(id);
        self.dirty = true;
    }

    /// Remove a group; its workspaces return to the top level.
    pub fn remove_group(&mut self, group_id: &str) -> bool {
        let Some(pos) = self.data.groups.iter().position(|g| g.id == group_id) else {
            return false;
        };
        self.data.groups.remove(pos);
        self.data.order.retain(|id| id != group_id);
        for w in &mut self.data.workspaces {
            if w.group_id.as_deref() == Some(group_id) {
                w.group_id = None;
            }
        }
        self.dirty = true;
        true
    }

    /// File a workspace under a group (None = top level).
    pub fn set_workspace_group(&mut self, ws_id: &str, group_id: Option<String>) -> bool {
        if let Some(gid) = &group_id {
            if !self.data.groups.iter().any(|g| g.id == *gid) {
                return false;
            }
        }
        if let Some(w) = self.data.workspaces.iter_mut().find(|w| w.id == ws_id) {
            w.group_id = group_id;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Collapse / expand a group's header.
    pub fn toggle_group_collapsed(&mut self, group_id: &str) -> bool {
        if let Some(g) = self.data.groups.iter_mut().find(|g| g.id == group_id) {
            g.collapsed = Some(!g.collapsed.unwrap_or(false));
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn group_count(&self) -> usize {
        self.data.groups.len()
    }

    pub fn group_name_taken(&self, name: &str) -> bool {
        self.data.groups.iter().any(|g| g.name == name)
    }
}

/// A fresh tab: one pane, named "Tab N" for its workspace.
fn seed_tab(ids: &mut Ids) -> Tab {
    Tab {
        id: ids.next(),
        layout: Some(LayoutNode::Leaf { id: ids.next() }),
        name: None,
        pinned: None,
        shell: None,
        color: None,
    }
}

/// The display name for a tab (positional "Tab N" when untitled).
pub fn tab_display_name(tabs_before: usize, tab: &Tab) -> String {
    tab.name
        .clone()
        .unwrap_or_else(|| format!("Tab {}", tabs_before + 1))
}

/// Unused-today helper kept public for the store separation story: WHERE the
/// term store lives (the TUI's ONLY store root).
pub fn term_store_root() -> PathBuf {
    store_core::paths::term_config_dir()
}

#[cfg(test)]
mod tests {
    //! The B3 ACs that touch the store: a restart restores what the TUI
    //! persisted (round-trip through `store_core`, tmpdir), and the TUI's
    //! writes land ONLY in the term store — the desktop store's files are
    //! never created (the separation test). The env-var tests serialize on a
    //! mutex: `UMUX_CONFIG_DIR` is process-global (paths.rs's documented
    //! reason in-process tests would race).

    use super::*;
    use crate::layout;
    use std::sync::Mutex;

    /// Serializes every test that touches `UMUX_CONFIG_DIR`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_tmp_config(test: impl FnOnce(&std::path::Path)) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("UMUX_CONFIG_DIR", dir.path());
        test(dir.path());
        std::env::remove_var("UMUX_CONFIG_DIR");
    }

    fn ws(ws_id: &str, name: &str) -> Workspace {
        Workspace {
            id: ws_id.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn next_workspace_name_skips_taken_numbers() {
        let mut model = TermModel {
            data: WorkspaceData {
                workspaces: vec![ws("w1", "Workspace 1"), ws("w2", "Workspace 3")],
                ..Default::default()
            },
            settings: Settings::default(),
            ws_store: WorkspaceStore::new(std::env::temp_dir().join("umux-tui-test-unused")),
            settings_store: SettingsStore::new(std::env::temp_dir().join("umux-tui-test-unused")),
            dirty: false,
        };
        // Browser-style: the count sets the next number (len + 1); a taken
        // name bumps the number until unique.
        assert_eq!(model.next_workspace_name(), "Workspace 4");
        model.data.workspaces.push(ws("w3", "Workspace 4"));
        assert_eq!(model.next_workspace_name(), "Workspace 5");
    }

    #[test]
    fn structural_changes_mark_dirty_and_persist_writes_once() {
        with_tmp_config(|_| {
            let mut ids = Ids::new();
            let mut model = TermModel::open();
            assert!(!model.dirty(), "a fresh open is clean");
            let ws_id = model.create_workspace(&mut ids);
            assert!(model.dirty());
            model.persist();
            assert!(!model.dirty(), "persist clears the flag");
            // A second persist with nothing pending writes nothing (no error
            // channel to observe — the flag is the observable contract).
            model.persist();
        });
    }

    #[test]
    fn a_restart_restores_workspaces_tabs_and_split_layout() {
        with_tmp_config(|_| {
            let mut ids = Ids::new();
            let (ws_id, tab_id, first_leaf, second_leaf);
            {
                let mut model = TermModel::open();
                ws_id = model.create_workspace(&mut ids);
                tab_id = model.create_tab(&ws_id, &mut ids).unwrap();
                let tree = model
                    .tab_layout(&tab_id)
                    .expect("a fresh tab carries a one-pane layout");
                first_leaf = layout::leaf_ids(&tree)[0].clone();
                let (_, leaf_id) = model
                    .split_pane(&tab_id, &first_leaf, Orientation::Horizontal, &mut ids)
                    .expect("the pane splits");
                second_leaf = leaf_id;
                model.rename_workspace(&ws_id, "proj".into());
                model.persist();
            }
            // A fresh open — the restart — reads back everything.
            let model = TermModel::open();
            let restored = model.workspace(&ws_id).expect("workspace survives");
            assert_eq!(restored.name, "proj");
            let tree = model.tab_layout(&tab_id).unwrap();
            assert_eq!(layout::leaf_ids(&tree), vec![first_leaf, second_leaf]);
        });
    }

    #[test]
    fn the_tui_writes_only_the_term_store() {
        with_tmp_config(|dir| {
            let mut ids = Ids::new();
            let mut model = TermModel::open();
            model.create_workspace(&mut ids);
            model.persist();

            // The term store file exists, under term/.
            let term_path = dir.join("term").join("workspaces.json");
            assert!(term_path.exists(), "the term store was written: {}", term_path.display());

            // The desktop store's file was NEVER created (the separation AC).
            let desk_path = dir.join("workspaces.json");
            assert!(!desk_path.exists(), "the desktop store must stay untouched");
        });
    }

    #[test]
    fn workspace_order_survives_deletion_and_unknown_ids() {
        let mut model = TermModel {
            data: WorkspaceData {
                workspaces: vec![ws("w1", "one"), ws("w2", "two")],
                order: vec!["w2".into(), "ghost".into(), "w1".into()],
                ..Default::default()
            },
            settings: Settings::default(),
            ws_store: WorkspaceStore::new(std::env::temp_dir().join("umux-tui-test-unused")),
            settings_store: SettingsStore::new(std::env::temp_dir().join("umux-tui-test-unused")),
            dirty: false,
        };
        assert_eq!(model.workspace_order(), vec!["w2".to_string(), "w1".to_string()]);
        assert!(model.delete_workspace("w1"));
        assert_eq!(model.workspace_order(), vec!["w2".to_string()]);
        assert!(!model.delete_workspace("w1"), "deleting twice refuses");
    }

    #[test]
    fn close_pane_refuses_the_last_pane_and_promotes_the_sibling() {
        let mut ids = Ids::new();
        let mut model = TermModel {
            data: WorkspaceData::default(),
            settings: Settings::default(),
            ws_store: WorkspaceStore::new(std::env::temp_dir().join("umux-tui-test-unused")),
            settings_store: SettingsStore::new(std::env::temp_dir().join("umux-tui-test-unused")),
            dirty: false,
        };
        let ws_id = model.create_workspace(&mut ids);
        let tab_id = model.workspace(&ws_id).unwrap().tabs[0].id.clone();
        let leaves = layout::leaf_ids(&model.tab_layout(&tab_id).unwrap());
        // The single pane of a fresh tab cannot be closed via this op.
        assert!(!model.close_pane(&tab_id, &leaves[0]));

        let (_, second) = model
            .split_pane(&tab_id, &leaves[0], Orientation::Horizontal, &mut ids)
            .unwrap();
        assert!(model.close_pane(&tab_id, &second));
        let tree = model.tab_layout(&tab_id).unwrap();
        assert_eq!(layout::leaf_ids(&tree), leaves[..1].to_vec());
    }

    #[test]
    fn ids_are_unique_v4_shaped_strings() {
        let mut ids = Ids::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let id = ids.next();
            assert_eq!(id.len(), 36, "{id}");
            assert!(id.matches('-').count() == 4, "{id}");
            assert!(seen.insert(id), "ids must not repeat");
        }
    }

    #[test]
    fn tab_display_name_falls_back_to_position() {
        let mut ids = Ids::new();
        let bare = |id: String| Tab {
            id,
            layout: None,
            name: None,
            pinned: None,
            color: None,
            shell: None,
        };
        let named = Tab {
            name: Some("shell".into()),
            ..bare(ids.next())
        };
        assert_eq!(tab_display_name(0, &named), "shell");
        let unnamed = bare(ids.next());
        assert_eq!(tab_display_name(0, &unnamed), "Tab 1");
        assert_eq!(tab_display_name(2, &unnamed), "Tab 3");
    }
    #[test]
    fn the_sidebar_tree_nests_workspaces_under_their_groups() {
        with_tmp_config(|_| {
            let mut ids = Ids::new();
            let mut model = TermModel::open();
            // One ungrouped workspace, then a group with a member workspace.
            let ws_ungrouped = model.create_workspace(&mut ids);
            model.add_group("g1".into(), "Group 1".into());
            let ws_member = model.create_workspace(&mut ids);
            assert!(model.set_workspace_group(&ws_member, Some("g1".into())));

            let tree = model.sidebar_tree();
            assert_eq!(tree.len(), 3, "group header + member + ungrouped");
            // The GROUP header comes first, the member workspace sits UNDER
            // it (round-5 hierarchy), and the ungrouped one trails.
            assert!(matches!(
                &tree[0],
                SidebarEntry::Group { id, collapsed: false, .. } if id == "g1"
            ));
            assert!(matches!(
                &tree[1],
                SidebarEntry::Workspace { id, group: Some(g), .. } if id == &ws_member && g == "g1"
            ));
            assert!(matches!(
                &tree[2],
                SidebarEntry::Workspace { id, group: None, .. } if id == &ws_ungrouped
            ));
        });
    }
}
