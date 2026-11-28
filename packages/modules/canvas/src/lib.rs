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
    use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore, ValidatedWords};

    #[test]
    fn canvas_table_and_ephemeral_draw_list_work() {
        let mut rec = DrawListRecorder::new();
        rec.clear(ColorRgba::BLACK)
            .fill_rect(0.0, 0.0, 100.0, 50.0, ColorRgba::WHITE)
            .fill_rounded_rect(4.0, 5.0, 20.0, 10.0, 3.0, ColorRgba::rgb(1, 2, 3))
            .fill_circle(12.0, 14.0, 6.0, ColorRgba::rgb(4, 5, 6))
            .stroke_line(0.0, 0.0, 10.0, 10.0, 2.0, ColorRgba::rgb(7, 8, 9))
            .stroke_polyline(&[(0.0, 0.0), (5.0, 10.0), (10.0, 2.0)], 1.5, ColorRgba::rgb(9, 8, 7))
            .clip_rect(0.0, 0.0, 50.0, 50.0)
            .restore_clip();
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
        assert_eq!(got.len(), dl.ops().len());

        let mut words = Vec::new();
        assert!(dl.write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid DrawList");
        let view = DrawList::view(vw);
        let mut rebuilt = DrawListRecorder::new();
        for op in view.ops() {
            match op {
                CanvasOpRef::Clear { color } => { rebuilt.clear(color); }
                CanvasOpRef::FillRect { x, y, w, h, color } => { rebuilt.fill_rect(x, y, w, h, color); }
                CanvasOpRef::FillRoundedRect { x, y, w, h, radius, color } => {
                    rebuilt.fill_rounded_rect(x, y, w, h, radius, color);
                }
                CanvasOpRef::FillCircle { cx, cy, radius, color } => {
                    rebuilt.fill_circle(cx, cy, radius, color);
                }
                CanvasOpRef::StrokeLine { x0, y0, x1, y1, width, color } => {
                    rebuilt.stroke_line(x0, y0, x1, y1, width, color);
                }
                CanvasOpRef::StrokePolyline { points, width, color } => {
                    let pts: Vec<(f32, f32)> = points.iter().collect();
                    rebuilt.stroke_polyline(&pts, width, color);
                }
                CanvasOpRef::ClipRect { x, y, w, h } => { rebuilt.clip_rect(x, y, w, h); }
                CanvasOpRef::RestoreClip => { rebuilt.restore_clip(); }
            }
        }
        assert_eq!(rebuilt.finish(), dl);
        assert!(!DrawList::validate(&[]));
        assert!(!DrawList::validate(&[0x7FFF_FFFF]));
    }
}
