//! Standalone vector Canvas capability (`packages/modules/canvas`).
//!
//! Pure-spec root crate with zero backend dependencies: defines [`Canvas`] (`#[derive(NveTable)]`)
//! and the `NVE_DRAW1` vector [`DrawList`] (`nve_store::FlatResource`, supporting ephemeral in-place
//! updates for 120 Hz canvas repaints without hashing).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod codec;
mod draw_list;
mod flat;
mod types;

pub use codec::{DrawListBuilder, DrawListDecoder};
pub use draw_list::{DrawList, DrawListRecorder};
pub use flat::{CanvasOpRef, DrawListOpsIter, DrawListView, PolylinePoints, PolylinePointsIter};
pub use types::{
    CanvasOp, DrawListError, DrawListHeader, ImageResizeMode, DRAW_LIST_ATTR_PREFIX, MAX_DRAW_OPS,
    MAX_POLYLINE_POINTS, NVE_DRAW_HEADER_SIZE, NVE_DRAW_MAGIC, NVE_DRAW_VERSION,
};

use nve_store::{NveTable, Res};

/// Vector canvas node table (`nve.canvas`).
#[derive(NveTable)]
#[nve_table(name = "nve.canvas", storage = "sparse", flags = "publish")]
pub struct Canvas {
    #[effect(realize, paint)]
    pub draw_list: Res<DrawList>,
    /// Monotonic repaint counter (increment for ephemeral in-place draw-list updates).
    #[effect(paint)]
    pub generation: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::ColorRgba;
    use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore};

    #[test]
    fn canvas_table_and_ephemeral_draw_list_work() {
        let mut rec = DrawListRecorder::new();
        rec.clear(ColorRgba::BLACK)
            .fill_rect(0.0, 0.0, 100.0, 50.0, ColorRgba::WHITE);
        let dl = rec.finish();

        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Canvas>(n)
            .draw_list_ephemeral(dl.clone())
            .generation(1);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let row = store.try_row::<Canvas>(n).unwrap();
        assert_eq!(row.generation(), 1);
        let got = store.resource::<DrawList>(row.draw_list()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got.ops().count(), 2);

        let mut words = Vec::new();
        assert!(dl.write_words(&mut words));
        assert!(DrawList::validate(&words));
        assert_eq!(DrawList::view(&words).len(), 2);
        assert!(!DrawList::validate(&[]));
        assert!(!DrawList::validate(&[0x7FFF_FFFF]));
    }
}
