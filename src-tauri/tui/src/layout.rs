// layout — the pane split tree + cell geometry (B3, v1.9.0).
//
// The Rust sibling of the frontend's PaneLayout (`src/PaneLayout.ts`), in
// terminal CELLS instead of pixels, over the SAME model the stores persist
// (`store_core::workspace_store::LayoutNode` — leaf = panel, split =
// divider). Pure, no I/O: split/close/set-ratio/geometry/hit-tests are all
// total functions on the tree, so the whole layout core is unit-testable
// without a terminal.
//
// Minimum-size rule mirrors the TS suite: a split/clamp that would push a
// side below the minimum leaves the tree UNCHANGED (the caller no-ops on an
// unchanged result); a degenerate axis (<= 2 * min along the split axis)
// cannot satisfy both sides, so the current shape stands.

use ratatui::layout::Rect;
use store_core::workspace_store::{LayoutNode, Orientation};

/// Minimum pane width in cells (the TS MIN_PANEL_PX = 80 px ≈ 10 columns at
/// an 8 px cell; the TUI needs no title text inside a pane, so slightly
/// narrower is still usable).
pub const MIN_PANE_COLS: u16 = 4;

/// Minimum pane height in cells (the border box + one content row).
pub const MIN_PANE_ROWS: u16 = 3;

/// Split the leaf `leaf_id` in `orientation`: the original leaf keeps its id
/// as the FIRST child, a fresh leaf `new_leaf_id` is the SECOND. Returns
/// `None` when `leaf_id` is not in the tree. Unlimited depth (v0.2 rule).
pub fn split_leaf(
    tree: &LayoutNode,
    leaf_id: &str,
    orientation: Orientation,
    split_id: &str,
    new_leaf_id: &str,
) -> Option<LayoutNode> {
    match tree {
        LayoutNode::Leaf { id } => {
            if id != leaf_id {
                return None;
            }
            Some(LayoutNode::Split {
                id: split_id.to_string(),
                orientation,
                ratio: 0.5,
                first: Box::new(LayoutNode::Leaf {
                    id: leaf_id.to_string(),
                }),
                second: Box::new(LayoutNode::Leaf {
                    id: new_leaf_id.to_string(),
                }),
            })
        }
        LayoutNode::Split {
            id,
            orientation: o,
            ratio,
            first,
            second,
        } => {
            let f = split_leaf(first, leaf_id, orientation, split_id, new_leaf_id);
            if let Some(f) = f {
                return Some(LayoutNode::Split {
                    id: id.clone(),
                    orientation: *o,
                    ratio: *ratio,
                    first: Box::new(f),
                    second: second.clone(),
                });
            }
            let s = split_leaf(second, leaf_id, orientation, split_id, new_leaf_id)?;
            Some(LayoutNode::Split {
                id: id.clone(),
                orientation: *o,
                ratio: *ratio,
                first: first.clone(),
                second: Box::new(s),
            })
        }
    }
}

/// Close the leaf `leaf_id`: the sibling is promoted into the parent's slot.
/// `None` when `leaf_id` is not in the tree or IS the whole tree (the last
/// pane closes with the tab, never via this op).
pub fn close_leaf(tree: &LayoutNode, leaf_id: &str) -> Option<LayoutNode> {
    match tree {
        LayoutNode::Leaf { .. } => None,
        LayoutNode::Split {
            id,
            orientation,
            ratio,
            first,
            second,
        } => {
            if let LayoutNode::Leaf { id: fid } = first.as_ref() {
                if fid == leaf_id {
                    return Some((**second).clone());
                }
            }
            if let LayoutNode::Leaf { id: sid } = second.as_ref() {
                if sid == leaf_id {
                    return Some((**first).clone());
                }
            }
            if let Some(f) = close_leaf(first, leaf_id) {
                return Some(LayoutNode::Split {
                    id: id.clone(),
                    orientation: *orientation,
                    ratio: *ratio,
                    first: Box::new(f),
                    second: second.clone(),
                });
            }
            let s = close_leaf(second, leaf_id)?;
            Some(LayoutNode::Split {
                id: id.clone(),
                orientation: *orientation,
                ratio: *ratio,
                first: first.clone(),
                second: Box::new(s),
            })
        }
    }
}

/// Leaf ids in tree order (first child first) — the fallback focus order.
pub fn leaf_ids(tree: &LayoutNode) -> Vec<String> {
    match tree {
        LayoutNode::Leaf { id } => vec![id.clone()],
        LayoutNode::Split { first, second, .. } => {
            let mut ids = leaf_ids(first);
            ids.extend(leaf_ids(second));
            ids
        }
    }
}

/// Move the divider `split_id` to `ratio`, clamped so neither side of THAT
/// split drops below `min_size` cells along the split's axis (`axis` is the
/// split node's OWN extent, exactly like the TS rule — that is what makes
/// the clamp correct at any nesting depth). Degenerate axis: no change.
pub fn set_ratio(tree: &LayoutNode, split_id: &str, ratio: f64, axis: u16, min_size: u16) -> LayoutNode {
    set_ratio_in(tree, split_id, ratio, axis, min_size)
}

fn set_ratio_in(node: &LayoutNode, split_id: &str, ratio: f64, axis: u16, min_size: u16) -> LayoutNode {
    match node {
        LayoutNode::Leaf { .. } => node.clone(),
        LayoutNode::Split {
            id,
            orientation,
            ratio: r,
            first,
            second,
        } => {
            if id == split_id {
                // `axis` is this split's own extent along its orientation
                // (width for horizontal, height for vertical — the caller
                // picks it out of the divider's split_rect).
                let usable = axis.saturating_sub(1) as f64; // the divider column itself
                let lo = if usable > 0.0 { min_size as f64 / usable } else { 0.0 };
                let hi = 1.0 - lo;
                if lo >= hi {
                    return node.clone();
                }
                let clamped = ratio.clamp(lo, hi);
                if clamped == *r {
                    return node.clone();
                }
                return LayoutNode::Split {
                    id: id.clone(),
                    orientation: *orientation,
                    ratio: clamped,
                    first: first.clone(),
                    second: second.clone(),
                };
            }
            LayoutNode::Split {
                id: id.clone(),
                orientation: *orientation,
                ratio: *r,
                first: Box::new(set_ratio_in(first, split_id, ratio, axis, min_size)),
                second: Box::new(set_ratio_in(second, split_id, ratio, axis, min_size)),
            }
        }
    }
}

/// A divider bar's hit area: the `gap`-wide strip between a split's two
/// children, plus the split node's own extent (`split_rect`) — the drag math
/// needs the parent's full box, exactly like the TS DividerBox.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub id: String,
    pub orientation: Orientation,
    pub rect: Rect,
    pub split_rect: Rect,
}

/// Geometry: carve `area` by the tree. Returns the pane rects (tree order,
/// first child first) and the divider strips. The `ratio` splits the space
/// LEFT AFTER the gap column/row, matching the TS `boxes()`; a rect that
/// cannot satisfy the minimum collapses to `gap`-wide slivers rather than
/// panicking — the size-adaptive floor (AC: no panic at small sizes).
pub fn geometry(
    tree: &LayoutNode,
    area: Rect,
    gap: u16,
) -> (Vec<(String, Rect)>, Vec<Divider>) {
    let mut panes = Vec::new();
    let mut dividers = Vec::new();
    box_node(tree, area, gap, &mut panes, &mut dividers);
    (panes, dividers)
}

fn box_node(
    node: &LayoutNode,
    area: Rect,
    gap: u16,
    panes: &mut Vec<(String, Rect)>,
    dividers: &mut Vec<Divider>,
) {
    match node {
        LayoutNode::Leaf { id } => panes.push((id.clone(), area)),
        LayoutNode::Split {
            id,
            orientation,
            ratio,
            first,
            second,
        } => {
            match orientation {
                Orientation::Horizontal => {
                    let first_len = ((area.width.saturating_sub(gap)) as f64 * *ratio) as u16;
                    let first_rect = Rect {
                        width: first_len.min(area.width),
                        ..area
                    };
                    box_node(first, first_rect, gap, panes, dividers);
                    let bar_x = area.x.saturating_add(first_len).min(
                        area.x.saturating_add(area.width).saturating_sub(gap),
                    );
                    dividers.push(Divider {
                        id: id.clone(),
                        orientation: *orientation,
                        rect: Rect {
                            x: bar_x,
                            y: area.y,
                            width: gap.min(area.width),
                            height: area.height,
                        },
                        split_rect: area,
                    });
                    let second_rect = Rect {
                        x: bar_x.saturating_add(gap),
                        width: area.width.saturating_sub(first_len.saturating_add(gap)),
                        ..area
                    };
                    box_node(second, second_rect, gap, panes, dividers);
                }
                Orientation::Vertical => {
                    let first_len = ((area.height.saturating_sub(gap)) as f64 * *ratio) as u16;
                    let first_rect = Rect {
                        height: first_len.min(area.height),
                        ..area
                    };
                    box_node(first, first_rect, gap, panes, dividers);
                    let bar_y = area.y.saturating_add(first_len).min(
                        area.y.saturating_add(area.height).saturating_sub(gap),
                    );
                    dividers.push(Divider {
                        id: id.clone(),
                        orientation: *orientation,
                        rect: Rect {
                            x: area.x,
                            y: bar_y,
                            width: area.width,
                            height: gap.min(area.height),
                        },
                        split_rect: area,
                    });
                    let second_rect = Rect {
                        y: bar_y.saturating_add(gap),
                        height: area.height.saturating_sub(first_len.saturating_add(gap)),
                        ..area
                    };
                    box_node(second, second_rect, gap, panes, dividers);
                }
            }
        }
    }
}

/// Which pane (leaf id) contains the point — the mouse click's focus rule.
pub fn pane_at(panes: &[(String, Rect)], x: u16, y: u16) -> Option<String> {
    panes
        .iter()
        .find(|(_, r)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
        .map(|(id, _)| id.clone())
}

/// Which divider strip contains the point — the drag-resize target.
pub fn divider_at(dividers: &[Divider], x: u16, y: u16) -> Option<Divider> {
    dividers
        .iter()
        .find(|d| {
            x >= d.rect.x
                && x < d.rect.x + d.rect.width
                && y >= d.rect.y
                && y < d.rect.y + d.rect.height
        })
        .cloned()
}

/// Drag arithmetic (pure): the new ratio for `divider` when the pointer sits
/// at (x, y) — the pointer's position along the split's own extent, minus
/// half the gap, over the usable length. Clamped by the SAME minimum rule as
/// [`set_ratio`]. Returns `None` when the extent is degenerate.
pub fn drag_ratio(divider: &Divider, x: u16, y: u16, min_size: u16) -> Option<f64> {
    let (pos, len) = match divider.orientation {
        Orientation::Horizontal => (
            (x.saturating_sub(divider.split_rect.x)) as f64,
            divider.split_rect.width as f64,
        ),
        Orientation::Vertical => (
            (y.saturating_sub(divider.split_rect.y)) as f64,
            divider.split_rect.height as f64,
        ),
    };
    if len <= 0.0 {
        return None;
    }
    let usable = len - 1.0; // the divider column itself
    if usable <= 0.0 {
        return None;
    }
    let lo = min_size as f64 / usable;
    let hi = 1.0 - lo;
    if lo >= hi {
        return None;
    }
    Some((pos / usable).clamp(lo, hi))
}

#[cfg(test)]
mod tests {
    //! The B3 ACs: unlimited splits with enforced per-pane minimums, the
    //! store round-trip model (`LayoutNode`), size-adaptive geometry without
    //! panic. Case shapes mirror the TS PaneLayout suite.

    use super::*;
    use store_core::workspace_store::{LayoutNode, Orientation};

    fn leaf(id: &str) -> LayoutNode {
        LayoutNode::Leaf { id: id.into() }
    }

    fn hsplit(id: &str, ratio: f64, first: LayoutNode, second: LayoutNode) -> LayoutNode {
        LayoutNode::Split {
            id: id.into(),
            orientation: Orientation::Horizontal,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn vsplit(id: &str, ratio: f64, first: LayoutNode, second: LayoutNode) -> LayoutNode {
        LayoutNode::Split {
            id: id.into(),
            orientation: Orientation::Vertical,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, width: w, height: h }
    }

    // --- split_leaf -------------------------------------------------------------

    #[test]
    fn split_leaf_becomes_a_fifty_fifty_split() {
        let tree = leaf("a");
        let next = split_leaf(&tree, "a", Orientation::Horizontal, "s1", "b").unwrap();
        assert_eq!(next, hsplit("s1", 0.5, leaf("a"), leaf("b")));
    }

    #[test]
    fn split_nests_into_the_matching_leaf_only() {
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        let next = split_leaf(&tree, "b", Orientation::Vertical, "s2", "c").unwrap();
        assert_eq!(
            next,
            hsplit("s1", 0.5, leaf("a"), vsplit("s2", 0.5, leaf("b"), leaf("c")))
        );
        // The untouched sibling keeps its id.
        assert_eq!(leaf_ids(&next), vec!["a", "b", "c"]);
    }

    #[test]
    fn split_of_a_missing_leaf_is_none() {
        let tree = leaf("a");
        assert!(split_leaf(&tree, "zz", Orientation::Horizontal, "s", "b").is_none());
    }

    #[test]
    fn splits_are_unlimited_in_depth() {
        let mut tree = leaf("p0");
        for i in 0..20 {
            tree = split_leaf(&tree, &format!("p{i}"), Orientation::Horizontal, &format!("s{i}"), &format!("p{}", i + 1)).unwrap();
        }
        assert_eq!(leaf_ids(&tree).len(), 21);
    }

    // --- close_leaf ---------------------------------------------------------------

    #[test]
    fn close_leaf_promotes_the_sibling() {
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        assert_eq!(close_leaf(&tree, "a").unwrap(), leaf("b"));
        assert_eq!(close_leaf(&tree, "b").unwrap(), leaf("a"));
    }

    #[test]
    fn close_deep_leaf_promotes_the_whole_sibling_subtree() {
        let tree = hsplit("s1", 0.5, vsplit("s2", 0.5, leaf("a"), leaf("b")), leaf("c"));
        let next = close_leaf(&tree, "a").unwrap();
        assert_eq!(next, hsplit("s1", 0.5, leaf("b"), leaf("c")));
    }

    #[test]
    fn closing_the_last_pane_is_refused() {
        // The last pane closes with the tab, never via this op.
        assert!(close_leaf(&leaf("a"), "a").is_none());
    }

    #[test]
    fn closing_a_missing_leaf_is_none() {
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        assert!(close_leaf(&tree, "zz").is_none());
    }

    #[test]
    fn leaf_ids_keep_tree_order_first_child_first() {
        let tree = hsplit("s1", 0.5, leaf("a"), vsplit("s2", 0.5, leaf("b"), leaf("c")));
        assert_eq!(leaf_ids(&tree), vec!["a", "b", "c"]);
    }

    // --- set_ratio ---------------------------------------------------------------

    #[test]
    fn set_ratio_clamps_to_the_minimum_on_both_sides() {
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        // Axis 41, gap 1 → 40 usable cells; min 10 → ratio clamped to
        // [10/40, 30/40] = [0.25, 0.75].
        let next = set_ratio(&tree, "s1", 0.9, 41, 10);
        assert_eq!(next, hsplit("s1", 0.75, leaf("a"), leaf("b")));

        let next = set_ratio(&tree, "s1", 0.1, 41, 10);
        assert_eq!(next, hsplit("s1", 0.25, leaf("a"), leaf("b")));
    }

    #[test]
    fn set_ratio_inside_the_range_applies_verbatim() {
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        let next = set_ratio(&tree, "s1", 0.7, 41, 10);
        assert_eq!(next, hsplit("s1", 0.7, leaf("a"), leaf("b")));
    }

    #[test]
    fn set_ratio_on_a_degenerate_axis_leaves_the_tree_unchanged() {
        // Axis 8 with min 4: usable 7, lo = 4/7 ≈ 0.57, hi ≈ 0.43 → lo >= hi.
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        let next = set_ratio(&tree, "s1", 0.9, 8, MIN_PANE_COLS);
        assert_eq!(next, tree);
    }

    #[test]
    fn set_ratio_targets_only_the_named_split() {
        let tree = hsplit("s1", 0.5, vsplit("s2", 0.5, leaf("a"), leaf("b")), leaf("c"));
        let next = set_ratio(&tree, "s2", 0.9, 41, 10);
        match next {
            LayoutNode::Split { first, .. } => match *first {
                LayoutNode::Split { ratio, .. } => assert_eq!(ratio, 0.75),
                _ => panic!("expected the nested split"),
            },
            _ => panic!("expected the root split"),
        }
    }

    // --- geometry -----------------------------------------------------------------

    #[test]
    fn geometry_carves_a_horizontal_split_with_one_gap_column() {
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        let (panes, dividers) = geometry(&tree, rect(0, 0, 41, 10), 1);
        assert_eq!(
            panes,
            vec![
                ("a".to_string(), rect(0, 0, 20, 10)),
                ("b".to_string(), rect(21, 0, 20, 10)),
            ]
        );
        assert_eq!(dividers.len(), 1);
        assert_eq!(dividers[0].rect, rect(20, 0, 1, 10));
        assert_eq!(dividers[0].split_rect, rect(0, 0, 41, 10));
    }

    #[test]
    fn geometry_carves_a_vertical_split_by_height() {
        let tree = vsplit("s1", 0.5, leaf("a"), leaf("b"));
        let (panes, dividers) = geometry(&tree, rect(0, 0, 20, 31), 1);
        assert_eq!(
            panes,
            vec![
                ("a".to_string(), rect(0, 0, 20, 15)),
                ("b".to_string(), rect(0, 16, 20, 15)),
            ]
        );
        assert_eq!(dividers[0].rect, rect(0, 15, 20, 1));
    }

    #[test]
    fn geometry_returns_panes_in_tree_order() {
        let tree = hsplit("s1", 0.5, vsplit("s2", 0.5, leaf("a"), leaf("b")), leaf("c"));
        let (panes, _) = geometry(&tree, rect(0, 0, 40, 20), 1);
        let ids: Vec<&str> = panes.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn geometry_sums_to_the_area_on_a_nested_tree() {
        // Every cell belongs to exactly one pane (the gap columns excepted).
        let tree = hsplit(
            "s1",
            0.5,
            vsplit("s2", 0.5, leaf("a"), leaf("b")),
            vsplit("s3", 0.5, leaf("c"), leaf("d")),
        );
        let (panes, dividers) = geometry(&tree, rect(0, 0, 41, 21), 1);
        let pane_cells: usize = panes.iter().map(|(_, r)| r.width as usize * r.height as usize).sum();
        let divider_cells: usize = dividers.iter().map(|d| d.rect.width as usize * d.rect.height as usize).sum();
        assert_eq!(pane_cells + divider_cells, 41 * 21);
    }

    #[test]
    fn geometry_at_a_tiny_size_never_panics() {
        // The size-adaptive floor (AC): degenerate areas collapse to slivers.
        let tree = hsplit(
            "s1",
            0.5,
            vsplit("s2", 0.5, leaf("a"), leaf("b")),
            leaf("c"),
        );
        for w in 0..=6u16 {
            for h in 0..=6u16 {
                let (panes, _) = geometry(&tree, rect(0, 0, w, h), 1);
                assert_eq!(panes.len(), 3, "{w}x{h}");
            }
        }
    }

    // --- hit tests + drag arithmetic ---------------------------------------------

    #[test]
    fn pane_at_finds_the_containing_pane() {
        let panes = vec![
            ("a".to_string(), rect(0, 0, 10, 10)),
            ("b".to_string(), rect(11, 0, 10, 10)),
        ];
        assert_eq!(pane_at(&panes, 5, 5).as_deref(), Some("a"));
        assert_eq!(pane_at(&panes, 15, 5).as_deref(), Some("b"));
        // The gap column and the outside hit nothing.
        assert_eq!(pane_at(&panes, 10, 5), None);
        assert_eq!(pane_at(&panes, 100, 100), None);
    }

    #[test]
    fn divider_at_finds_the_drag_strip() {
        let tree = hsplit("s1", 0.5, leaf("a"), leaf("b"));
        let (_, dividers) = geometry(&tree, rect(0, 0, 41, 10), 1);
        assert_eq!(divider_at(&dividers, 20, 3).unwrap().id, "s1");
        assert_eq!(divider_at(&dividers, 5, 3), None);
    }

    #[test]
    fn drag_ratio_follows_the_pointer_and_clamps_to_the_minimum() {
        let divider = Divider {
            id: "s1".into(),
            orientation: Orientation::Horizontal,
            rect: rect(20, 0, 1, 10),
            split_rect: rect(0, 0, 41, 10),
        };
        // usable = 40; pointer at column 20 → 0.5.
        assert!((drag_ratio(&divider, 20, 0, MIN_PANE_COLS).unwrap() - 0.5).abs() < 1e-9);
        // Clamps: left of the minimum and right of it.
        assert!((drag_ratio(&divider, 0, 0, MIN_PANE_COLS).unwrap() - 0.1).abs() < 1e-9);
        assert!((drag_ratio(&divider, 40, 0, MIN_PANE_COLS).unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn drag_ratio_vertical_uses_rows() {
        let divider = Divider {
            id: "s1".into(),
            orientation: Orientation::Vertical,
            rect: rect(0, 15, 20, 1),
            split_rect: rect(0, 0, 20, 31),
        };
        // usable = 30; pointer at row 15 → 0.5.
        assert!((drag_ratio(&divider, 0, 15, MIN_PANE_ROWS).unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn drag_ratio_on_a_degenerate_extent_is_none() {
        let divider = Divider {
            id: "s1".into(),
            orientation: Orientation::Horizontal,
            rect: rect(0, 0, 1, 1),
            split_rect: rect(0, 0, 1, 1),
        };
        assert_eq!(drag_ratio(&divider, 0, 0, MIN_PANE_COLS), None);
    }
}
