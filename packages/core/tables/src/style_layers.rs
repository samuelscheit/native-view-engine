//! Core style-layer bitmasks and pure typed helpers shared across `nve-tables`, `nve-dioxus`,
//! and UI accessibility/audit passes (zero runtime CSS string parsing).

use nve_abi::Dimension;
use nve_store::{Length, NodeId, NodeStore, Rgba};

use crate::tables::Typography;

/// Style-layer bit for [`Layout`](crate::tables::Layout).
pub const STYLE_LAYER_LAYOUT: u8 = 1 << 0;
/// Style-layer bit for [`Visual`](crate::tables::Visual).
pub const STYLE_LAYER_VISUAL: u8 = 1 << 1;
/// Style-layer bit for [`Typography`](crate::tables::Typography).
pub const STYLE_LAYER_TYPOGRAPHY: u8 = 1 << 2;
/// Style-layer mask covering all three core style tables (`Layout`, `Visual`, `Typography`).
pub const STYLE_LAYER_ALL: u8 = STYLE_LAYER_LAYOUT | STYLE_LAYER_VISUAL | STYLE_LAYER_TYPOGRAPHY;

/// Converts a typed [`Dimension`] into a word-table [`Length`].
#[inline]
pub const fn dim_to_length(d: Dimension) -> Length {
    match d {
        Dimension::Auto => Length::AUTO,
        Dimension::Points(v) => Length::points(v),
        Dimension::Percent(v) => Length::percent(v),
    }
}

/// Resolves the effective inherited text color of `id`: its nearest explicit `Typography` color,
/// self included, or opaque black (one `O(depth)` walk, zero allocations).
pub fn effective_text_color(store: &NodeStore, id: NodeId) -> Rgba {
    let mut cur = Some(id);
    while let Some(nid) = cur.filter(|&n| store.is_live(n)) {
        if let Some(t) = store.try_row::<Typography>(nid)
            && t.is_color_explicit()
        {
            return t.color();
        }
        cur = store.parent(nid);
    }
    Rgba::BLACK
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::ChangeSet;

    #[test]
    fn dim_to_length_converts_all_variants() {
        assert_eq!(dim_to_length(Dimension::Auto), Length::AUTO);
        assert_eq!(dim_to_length(Dimension::Points(24.0)), Length::points(24.0));
        assert_eq!(dim_to_length(Dimension::Percent(50.0)), Length::percent(50.0));
    }

    #[test]
    fn effective_text_color_inherits_from_nearest_explicit_ancestor() {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let (parent, mid, leaf) = {
            let mut tx = store.tx();
            let p = tx.create();
            let m = tx.create();
            let l = tx.create();
            tx.set_children(NodeId::ROOT, &[p]);
            tx.set_children(p, &[m]);
            tx.set_children(m, &[l]);
            tx.edit::<Typography>(p).color(Rgba::rgb(37, 99, 235));
            tx.edit::<Typography>(m).font_size(14.0);
            tx.commit_into(&mut changes);
            (p, m, l)
        };
        assert_eq!(effective_text_color(&store, leaf), Rgba::rgb(37, 99, 235));
        assert_eq!(effective_text_color(&store, mid), Rgba::rgb(37, 99, 235));
        assert_eq!(effective_text_color(&store, parent), Rgba::rgb(37, 99, 235));
        assert_eq!(effective_text_color(&store, NodeId::ROOT), Rgba::BLACK);
    }
}
