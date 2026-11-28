//! Standalone vector Canvas capability (`packages/modules/canvas`).
//!
//! Pure-spec root crate with zero backend dependencies: defines [`Canvas`] (`#[derive(NveTable)]`),
//! the flat 4-byte-aligned [`DrawList`] (`nve_store::FlatResource`, `nve.res.draw_list`,
//! supporting ephemeral in-place updates for 120 Hz canvas repaints without hashing), and
//! [`DRAW_LIST_CODEC_DESC`] for cross-language host decoder codegen.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod desc;
mod draw_list;
mod flat;
mod types;

pub use desc::{ClipDepthTracker, DRAW_LIST_CODEC_DESC};
pub use draw_list::{DrawList, DrawListRecorder};
pub use flat::{CanvasOpRef, DrawListOpsIter, DrawListView, PolylinePoints, PolylinePointsIter};
pub use types::{CanvasOp, ImageResizeMode, MAX_CLIP_DEPTH, MAX_DRAW_OPS, MAX_POLYLINE_POINTS};

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

    #[test]
    fn flat_draw_list_validation_golden_vectors() {
        // Empty valid stream (`op_count == 0`).
        assert!(DrawList::validate(&[0]));

        // Zero-point polyline and negative-zero non-negative floats are accepted.
        let mut rec = DrawListRecorder::new();
        rec.stroke_polyline(&[], -0.0, ColorRgba::WHITE)
            .fill_circle(1.0, 2.0, 0.0, ColorRgba::BLACK)
            .fill_rounded_rect(0.0, 0.0, 10.0, 10.0, -0.0, ColorRgba::WHITE)
            .stroke_line(0.0, 0.0, 5.0, 5.0, 0.0, ColorRgba::WHITE);
        let mut words = Vec::new();
        assert!(rec.finish().write_words(&mut words));
        assert!(DrawList::validate(&words));

        // Trailing word rejected.
        let mut trailing = words.clone();
        trailing.push(0);
        assert!(!DrawList::validate(&trailing));

        // Truncated streams rejected.
        for cut in 0..words.len() {
            assert!(!DrawList::validate(&words[..cut]), "cut={cut}");
        }

        // Unknown opcode rejected.
        assert!(!DrawList::validate(&[1, 0]));
        assert!(!DrawList::validate(&[1, 9]));
        assert!(!DrawList::validate(&[1, u32::MAX]));

        // Op count limit and polyline point count limit enforced.
        assert!(!DrawList::validate(&[(MAX_DRAW_OPS as u32) + 1]));
        assert!(!DrawList::validate(&[
            1,
            6,
            1.0_f32.to_bits(),
            0xFFFF_FFFF,
            (MAX_POLYLINE_POINTS as u32) + 1,
        ]));

        // Non-finite floats rejected across ops and polyline coordinates.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let b = bad.to_bits();
            assert!(!DrawList::validate(&[1, 2, b, 0, 0, 0, 0]));
            assert!(!DrawList::validate(&[1, 3, 0, 0, 0, 0, b, 0]));
            assert!(!DrawList::validate(&[1, 4, 0, 0, b, 0]));
            assert!(!DrawList::validate(&[1, 5, 0, 0, 0, 0, b, 0]));
            assert!(!DrawList::validate(&[1, 6, b, 0, 0]));
            assert!(!DrawList::validate(&[1, 6, 1.0_f32.to_bits(), 0, 1, b, 0]));
            assert!(!DrawList::validate(&[1, 6, 1.0_f32.to_bits(), 0, 1, 0, b]));
            assert!(!DrawList::validate(&[1, 7, 0, 0, 0, b]));
        }

        // Strictly negative radius/width rejected; negative x/y/w/h coords still allowed.
        let neg = (-0.5_f32).to_bits();
        assert!(DrawList::validate(&[1, 2, neg, neg, neg, neg, 0]));
        assert!(DrawList::validate(&[1, 7, neg, neg, neg, neg]));
        assert!(!DrawList::validate(&[1, 3, 0, 0, 10.0_f32.to_bits(), 10.0_f32.to_bits(), neg, 0]));
        assert!(!DrawList::validate(&[1, 4, 0, 0, neg, 0]));
        assert!(!DrawList::validate(&[1, 5, 0, 0, 1.0_f32.to_bits(), 1.0_f32.to_bits(), neg, 0]));
        assert!(!DrawList::validate(&[1, 6, neg, 0, 0]));

        // `write_words` refuses invalid raw `DrawList::from_ops` and rolls back output buffer.
        let bad_dl = DrawList::from_ops(vec![CanvasOp::FillCircle {
            cx: 0.0,
            cy: 0.0,
            radius: -1.0,
            color: ColorRgba::WHITE,
        }]);
        let mut scratch = vec![0xDEAD_BEEF];
        assert!(!bad_dl.write_words(&mut scratch));
        assert_eq!(scratch, vec![0xDEAD_BEEF]);

        // `DrawListRecorder` sanitizes negative radius/width to 0.0.
        let mut sanitized = DrawListRecorder::new();
        sanitized
            .fill_rounded_rect(0.0, 0.0, 10.0, 10.0, -4.0, ColorRgba::WHITE)
            .fill_circle(5.0, 5.0, -2.0, ColorRgba::WHITE)
            .stroke_line(0.0, 0.0, 1.0, 1.0, -3.0, ColorRgba::WHITE)
            .stroke_polyline(&[(0.0, 0.0)], -1.0, ColorRgba::WHITE);
        let mut out = Vec::new();
        assert!(sanitized.finish().write_words(&mut out));
        assert!(DrawList::validate(&out));

        // Descriptor constants match crate constants and `FlatResource::KIND`.
        assert_eq!(DRAW_LIST_CODEC_DESC.max_ops as usize, MAX_DRAW_OPS);
        assert_eq!(DRAW_LIST_CODEC_DESC.ops.len(), 8);
        assert_eq!(
            DRAW_LIST_CODEC_DESC.scope.expect("clip scope").max_depth as usize,
            MAX_CLIP_DEPTH
        );
        assert_eq!(
            nve_abi::Key64::of(DRAW_LIST_CODEC_DESC.kind_name),
            <DrawList as FlatResource>::KIND
        );
    }

    #[test]
    fn clip_depth_tracker_balances_66_nested_clips_and_2_restores() {
        let mut tracker = ClipDepthTracker::new();
        let mut pushed = 0usize;
        for _ in 0..66 {
            if tracker.push() {
                pushed += 1;
            }
        }
        assert_eq!(pushed, MAX_CLIP_DEPTH);
        assert_eq!(tracker.depth(), MAX_CLIP_DEPTH);
        assert_eq!(tracker.overflow(), 2);

        // Two restores only cancel the two overflowed clips; zero live restores emitted.
        assert!(!tracker.pop());
        assert!(!tracker.pop());
        assert_eq!(tracker.depth(), MAX_CLIP_DEPTH);
        assert_eq!(tracker.overflow(), 0);

        // Third restore pops one live clip; remaining 63 unwind at end of stream.
        assert!(tracker.pop());
        assert_eq!(tracker.depth(), MAX_CLIP_DEPTH - 1);
        assert_eq!(tracker.drain_open(), MAX_CLIP_DEPTH - 1);
        assert_eq!(tracker.depth(), 0);
        assert!(!tracker.pop());
    }
}
