//! 1,001-node card list exercising every hoisting shape, shared by the `taffy_relayout_1k_hoisted`
//! zero-allocation guard and Criterion bench.
//!
//! Each of the 100 cards (`position: relative`, row) holds 5 in-flow text leaves, 2
//! `position: absolute` badges hoisted to the card, and an inner relative box whose
//! `position: fixed` toast bubbles past both the inner box and the card to the root.

use nve_components::tables::{FlexDirection, Layout, Position, TextContent, Visual};
use nve_engine::Viewport;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

pub const CARDS: usize = 100;
pub const VIEWPORT: Viewport = Viewport {
    width: 1280.0,
    height: 800.0,
    scale_factor: 1.0,
};

pub struct HoistedTree {
    pub cards: Vec<NodeId>,
    pub leaves: Vec<NodeId>,
    pub badges: Vec<NodeId>,
    pub toasts: Vec<NodeId>,
}

pub fn build_hoisted_tree(store: &mut NodeStore, changes: &mut ChangeSet) -> HoistedTree {
    let mut tree = HoistedTree {
        cards: Vec::with_capacity(CARDS),
        leaves: Vec::with_capacity(CARDS * 5),
        badges: Vec::with_capacity(CARDS * 2),
        toasts: Vec::with_capacity(CARDS),
    };
    let mut tx = store.tx();
    for card_index in 0..CARDS {
        let card = tx.create();
        tx.edit::<Layout>(card)
            .flex_direction(FlexDirection::Row)
            .position(Position::Relative)
            .height(Length::points(40.0))
            .column_gap(Length::points(4.0));
        let mut kids = Vec::with_capacity(8);
        for _ in 0..5 {
            let leaf = tx.create();
            tx.edit::<Layout>(leaf)
                .flex_grow(1.0)
                .height(Length::points(32.0));
            tx.edit::<Visual>(leaf).background(Rgba::rgb(30, 41, 59));
            tx.edit::<TextContent>(leaf).text("cell");
            tree.leaves.push(leaf);
            kids.push(leaf);
        }
        for (inset_left, inset_right) in [
            (Length::points(2.0), Length::AUTO),
            (Length::AUTO, Length::points(2.0)),
        ] {
            let badge = tx.create();
            tx.edit::<Layout>(badge)
                .position(Position::Absolute)
                .inset_top(Length::points(2.0))
                .inset_left(inset_left)
                .inset_right(inset_right)
                .width(Length::points(12.0))
                .height(Length::points(12.0));
            tree.badges.push(badge);
            kids.push(badge);
        }
        let inner = tx.create();
        let toast = tx.create();
        tx.edit::<Layout>(inner)
            .width(Length::points(20.0))
            .height(Length::points(20.0));
        tx.edit::<Layout>(toast)
            .position(Position::Fixed)
            .inset_right(Length::points(16.0))
            .inset_bottom(Length::points(16.0 + card_index as f32))
            .width(Length::points(200.0))
            .height(Length::points(48.0));
        tx.set_children(inner, &[toast]);
        kids.push(inner);
        tx.set_children(card, &kids);
        tree.toasts.push(toast);
        tree.cards.push(card);
    }
    tx.set_children(NodeId::ROOT, &tree.cards);
    tx.commit_into(changes);
    tree
}

/// Value index for `frame` that differs from the previous visit of the same node (`len` nodes are
/// visited round-robin), so the store never elides the edit as a no-op.
#[inline]
fn fresh_step(frame: usize, len: usize, values: usize) -> usize {
    (frame + frame / len) % values
}

/// Paint-only frame: one leaf's `Visual::opacity` changes (values `0.5..=0.9` never include the
/// built default `1.0`).
pub fn commit_paint_frame(
    store: &mut NodeStore,
    changes: &mut ChangeSet,
    tree: &HoistedTree,
    frame: usize,
) {
    let len = tree.leaves.len();
    let mut tx = store.tx();
    tx.edit::<Visual>(tree.leaves[frame % len])
        .opacity(0.5 + fresh_step(frame, len, 5) as f32 * 0.1);
    tx.commit_into(changes);
}

/// Out-of-flow frame: one absolute badge's `inset_top` changes, re-running its card's and the
/// root's positioning passes (both rewrite their hoisted lists). Values `3..=6` never include the
/// built `2.0`, so a badge's first visit is a real edit too.
pub fn commit_badge_frame(
    store: &mut NodeStore,
    changes: &mut ChangeSet,
    tree: &HoistedTree,
    frame: usize,
) {
    let len = tree.badges.len();
    let mut tx = store.tx();
    tx.edit::<Layout>(tree.badges[frame % len])
        .inset_top(Length::points(3.0 + fresh_step(frame, len, 4) as f32));
    tx.commit_into(changes);
}
