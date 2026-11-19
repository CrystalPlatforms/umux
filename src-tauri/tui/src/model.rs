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
    LayoutNode, Orientation, Tab, Workspace, WorkspaceData, WorkspaceStore,
};

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
