//! Standalone vector Canvas capability (`packages/modules/canvas`).
//!
//! Pure-spec root crate with zero backend dependencies: defines [`Canvas`] (`#[derive(NveTable)]`),
//! the object-safe [`CanvasPainter`] / [`CanvasSink`] / [`GuardedPainter`] pipeline, optional
//! in-process [`DirectCanvasRegistry`] + [`write_canvas`], the flat 4-byte-aligned [`DrawList`]
//! (`nve_store::FlatResource`, `nve.res.draw_list`), and [`DRAW_LIST_CODEC_DESC`] for
//! cross-language host decoder codegen.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod desc;
mod direct;
mod draw_list;
mod flat;
mod painter;
mod polyline;
mod types;

pub use desc::{ClipDepthTracker, DRAW_LIST_CODEC_DESC};
pub use direct::{write_canvas, DirectCanvasRegistry, DirectPaintFn};
pub use draw_list::{DrawList, DrawListRecorder};
pub use flat::{CanvasOpRef, DrawListOpsIter, DrawListView};
pub use painter::{CanvasPaintSize, CanvasPainter, CanvasSink, GuardedPainter};
pub use polyline::{PolylinePoints, PolylinePointsIter};
pub use types::{
    paint_sparkline, CanvasOp, ImageResizeMode, MAX_CLIP_DEPTH, MAX_DRAW_OPS, MAX_POLYLINE_POINTS,
};

use nve_store::{NveTable, Res};

/// Vector canvas node table (`nve.canvas`).
#[derive(NveTable)]
#[nve_table(name = "nve.canvas", storage = "sparse", flags = "publish")]
pub struct Canvas {
    #[effect(realize, paint)]
    pub draw_list: Res<DrawList>,
    /// Monotonic repaint counter (increment for ephemeral in-place draw-list or direct updates).
    #[effect(paint)]
    pub generation: u32,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
            .stroke_polyline(
                &[(0.0, 0.0), (5.0, 10.0), (10.0, 2.0)],
                1.5,
                ColorRgba::rgb(9, 8, 7),
            )
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

        let row = store.try_row::<Canvas>(n).expect("canvas row");
        assert_eq!(row.generation(), 1);
        let got = store
            .resource::<DrawList>(row.draw_list())
            .expect("draw_list resource");
        assert_eq!(got.len(), dl.ops().len());

        let mut words = Vec::new();
        assert!(dl.write_words(&mut words));
        let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid DrawList");
        let view = DrawList::view(vw);
        let mut rebuilt = DrawListRecorder::new();
        view.replay(&mut rebuilt);
        assert_eq!(rebuilt.finish(), dl);
        assert!(!DrawList::validate(&[]));
        assert!(!DrawList::validate(&[0x7FFF_FFFF]));
    }

    #[test]
    fn flat_draw_list_validation_golden_vectors() {
        // Empty valid stream (`op_count == 0`).
        assert!(DrawList::validate(&[0]));

        // Wire validator accepts zero-point polyline and -0.0/0.0 non-negative floats from raw encoders.
        let raw_zero_dl = DrawList::from_ops(vec![
            CanvasOp::StrokePolyline {
                points: vec![],
                width: -0.0,
                color: ColorRgba::WHITE,
            },
            CanvasOp::FillCircle {
                cx: 1.0,
                cy: 2.0,
                radius: 0.0,
                color: ColorRgba::BLACK,
            },
            CanvasOp::FillRoundedRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                radius: -0.0,
                color: ColorRgba::WHITE,
            },
            CanvasOp::StrokeLine {
                x0: 0.0,
                y0: 0.0,
                x1: 5.0,
                y1: 5.0,
                width: 0.0,
                color: ColorRgba::WHITE,
            },
        ]);
        let mut words = Vec::new();
        assert!(raw_zero_dl.write_words(&mut words));
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
        assert!(!DrawList::validate(&[
            1,
            3,
            0,
            0,
            10.0_f32.to_bits(),
            10.0_f32.to_bits(),
            neg,
            0
        ]));
        assert!(!DrawList::validate(&[1, 4, 0, 0, neg, 0]));
        assert!(!DrawList::validate(&[
            1,
            5,
            0,
            0,
            1.0_f32.to_bits(),
            1.0_f32.to_bits(),
            neg,
            0
        ]));
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

        // `DrawListRecorder` via `GuardedPainter` sanitizes rounded radius and drops degenerate/NaN ops.
        let mut sanitized = DrawListRecorder::new();
        sanitized
            .fill_rounded_rect(0.0, 0.0, 10.0, 10.0, -4.0, ColorRgba::WHITE)
            .fill_circle(5.0, 5.0, -2.0, ColorRgba::WHITE)
            .fill_rect(f32::NAN, 0.0, 10.0, 10.0, ColorRgba::WHITE)
            .stroke_line(0.0, 0.0, 1.0, 1.0, -3.0, ColorRgba::WHITE)
            .stroke_polyline(&[(0.0, 0.0)], -1.0, ColorRgba::WHITE)
            .stroke_polyline(
                &[(1.0, 2.0), (f32::NAN, 0.0), (3.0, 4.0), (5.0, 6.0)],
                2.0,
                ColorRgba::BLACK,
            );
        let dl = sanitized.finish();
        assert_eq!(dl.len(), 3); // 1 rounded rect + 2 finite polyline segments split around NaN
        let mut out = Vec::new();
        assert!(dl.write_words(&mut out));
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

    #[test]
    fn direct_registry_lifecycle_rebind_recycled_slot_and_write_canvas_fallback() {
        let reg = DirectCanvasRegistry::new();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();

        // 1. Bind & verify Arc drop on replace/remove.
        let sentinel = Arc::new(AtomicUsize::new(0));
        let weak = Arc::downgrade(&sentinel);
        let n1 = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[n]);
            let s = Arc::clone(&sentinel);
            let g1 = write_canvas(
                &mut tx,
                n,
                Some(&reg),
                CanvasPaintSize::new(100.0, 50.0, 1.0),
                move |p, size| {
                    s.fetch_add(1, Ordering::Relaxed);
                    p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::WHITE);
                },
            );
            assert!(g1 > 0);
            tx.commit_into(&mut changes);
            n
        };
        drop(sentinel);
        assert_eq!(reg.len(), 1);
        assert!(reg.contains(n1));
        assert!(weak.upgrade().is_some());

        let mut rec = DrawListRecorder::new();
        assert!(reg.paint(n1, &mut rec, CanvasPaintSize::new(64.0, 32.0, 2.0)));
        assert_eq!(rec.finish().len(), 1);
        assert_eq!(
            weak.upgrade().expect("still alive").load(Ordering::Relaxed),
            1
        );

        // 2. Invalidate bumps generation without replacing closure.
        let g2 = {
            let mut tx = store.tx();
            let g = reg.invalidate(&mut tx, n1).expect("bound node invalidates");
            tx.commit_into(&mut changes);
            g
        };
        assert_eq!(store.try_row::<Canvas>(n1).expect("canvas").generation(), g2);

        // 3. Remove node from store & recycle slot: stale NodeId cannot match new generation.
        {
            let mut tx = store.tx();
            tx.remove(n1);
            tx.commit_into(&mut changes);
        }
        let n2 = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[n]);
            tx.commit_into(&mut changes);
            n
        };
        assert_eq!(n2.index(), n1.index());
        assert_ne!(n2, n1);
        assert!(!reg.contains(n2));
        let mut rec_recycled = DrawListRecorder::new();
        assert!(!reg.paint(n2, &mut rec_recycled, CanvasPaintSize::new(10.0, 10.0, 1.0)));

        // Removing old n1 drops the captured Arc immediately.
        assert!(reg.remove(n1));
        assert_eq!(reg.len(), 0);
        assert!(weak.upgrade().is_none(), "captured Arc must drop on remove");

        // 4. `write_canvas` with `None` falls back to recorded `DrawList`.
        {
            let mut tx = store.tx();
            let g = write_canvas(
                &mut tx,
                n2,
                None,
                CanvasPaintSize::new(40.0, 20.0, 1.0),
                |p, size| {
                    p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::BLACK);
                },
            );
            assert!(g > 0);
            tx.commit_into(&mut changes);
        }
        let row2 = store.try_row::<Canvas>(n2).expect("recorded row");
        let dl2 = store
            .resource::<DrawList>(row2.draw_list())
            .expect("recorded draw_list");
        assert_eq!(dl2.len(), 1);
    }

    #[test]
    fn polyline_points_slice_and_words_bitwise_eq() {
        let pts = [(1.5f32, -2.0), (3.0, 4.25)];
        let words = [
            1.5f32.to_bits(),
            (-2.0f32).to_bits(),
            3.0f32.to_bits(),
            4.25f32.to_bits(),
        ];
        let from_slice = PolylinePoints::from_slice(&pts);
        let from_words = PolylinePoints::from_words(&words);
        assert_eq!(from_slice, from_words);
        assert_eq!(from_slice.len(), 2);
        assert_eq!(from_words.truncate(1).len(), 1);
        assert_eq!(from_words.get(1), Some((3.0, 4.25)));
        assert!(from_words.is_validated_words());
        assert!(!from_slice.is_validated_words());
    }

    #[test]
    #[allow(clippy::panic)]
    fn direct_registry_liveness_order_safe_invalidate_and_unbind_zero_leak() {
        let reg = DirectCanvasRegistry::new();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();

        // Create node A, remove it, and recycle its index into live node B bound to `reg`.
        let (stale_a, live_b) = {
            let mut tx = store.tx();
            let a = tx.create();
            tx.set_children(NodeId::ROOT, &[a]);
            tx.commit_into(&mut changes);
            let mut tx = store.tx();
            tx.remove(a);
            tx.commit_into(&mut changes);
            let mut tx = store.tx();
            let b = tx.create();
            tx.set_children(NodeId::ROOT, &[b]);
            tx.commit_into(&mut changes);
            (a, b)
        };
        assert_eq!(stale_a.index(), live_b.index());
        assert_ne!(stale_a, live_b);

        let b_calls = Arc::new(AtomicUsize::new(0));
        {
            let mut tx = store.tx();
            let bc = Arc::clone(&b_calls);
            reg.bind(&mut tx, live_b, move |_, _| {
                bc.fetch_add(1, Ordering::Relaxed);
            });
            tx.commit_into(&mut changes);
        }
        assert_eq!(reg.len(), 1);

        // Passing stale `stale_a` to `bind_arc` panics in `tx.edit`, and MUST NOT clobber
        // `live_b`'s slot or corrupt `live_count`!
        let stale_cb: Arc<DirectPaintFn> = Arc::new(|_, _| {});
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut tx = store.tx();
            reg.bind_arc(&mut tx, stale_a, Arc::clone(&stale_cb));
        }));
        assert!(res.is_err(), "bind_arc on dead NodeId must panic");
        assert_eq!(reg.len(), 1);
        assert!(reg.contains(live_b));
        assert!(!reg.contains(stale_a));
        let mut sink = DrawListRecorder::new();
        assert!(reg.paint(live_b, &mut sink, CanvasPaintSize::new(10.0, 10.0, 1.0)));
        assert_eq!(b_calls.load(Ordering::Relaxed), 1);

        // Removing `live_b` without calling `reg.remove`: `invalidate` must return `None` and
        // evict the stale slot rather than panicking in `tx.edit`.
        {
            let mut tx = store.tx();
            tx.remove(live_b);
            tx.commit_into(&mut changes);
        }
        {
            let mut tx = store.tx();
            assert_eq!(reg.invalidate(&mut tx, live_b), None);
        }
        assert_eq!(reg.len(), 0);
        assert_eq!(Arc::strong_count(&b_calls), 1);

        // Same-transaction `create` + `bind` + `unbind` leaves zero registry leak and detaches Canvas.
        let unbind_guard = Arc::new(AtomicUsize::new(0));
        let c = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.set_children(NodeId::ROOT, &[n]);
            let g = Arc::clone(&unbind_guard);
            reg.bind(&mut tx, n, move |_, _| {
                g.fetch_add(1, Ordering::Relaxed);
            });
            assert_eq!(reg.len(), 1);
            assert!(reg.unbind(&mut tx, n));
            tx.commit_into(&mut changes);
            n
        };
        assert_eq!(reg.len(), 0);
        assert!(!reg.contains(c));
        assert!(!store.has::<Canvas>(c));
        assert_eq!(Arc::strong_count(&unbind_guard), 1);
    }

    #[test]
    #[allow(clippy::panic)]
    fn record_into_words_panic_restores_prefix_and_capacity_and_empty_is_cached() {
        // `DrawList::empty()` shares one cached `Arc<[u32]>`.
        let e1 = DrawList::empty();
        let e2 = DrawList::empty();
        assert!(e1.ptr_eq(&e2));
        assert_eq!(e1.words(), &[0]);

        // `record_into_words` panic guard preserves prefix words and warmed capacity.
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(&[0xDEAD_BEEF, 0xCAFE_BABE]);
        let cap_before = out.capacity();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            DrawList::record_into_words(&mut out, |rec| {
                rec.clip_rect(0.0, 0.0, 20.0, 20.0);
                rec.fill_rect(1.0, 1.0, 5.0, 5.0, ColorRgba::WHITE);
                panic!("simulated recorder closure panic");
            });
        }));
        assert!(res.is_err());
        assert_eq!(out.as_slice(), &[0xDEAD_BEEF, 0xCAFE_BABE]);
        assert!(
            out.capacity() >= cap_before,
            "warmed capacity must be preserved on panic"
        );
    }

    #[test]
    fn paint_sparkline_handles_extreme_finite_f32_span_without_nan() {
        let dl = DrawList::sparkline(
            &[-3.0e38, 0.0, 3.0e38],
            200.0,
            60.0,
            ColorRgba::WHITE,
            ColorRgba::BLACK,
        );
        let mut words = Vec::new();
        assert!(dl.write_words(&mut words));
        assert!(DrawList::validate(&words));
        assert!(
            dl.ops().any(|op| matches!(op, CanvasOpRef::StrokePolyline { points, .. } if points.len() == 3)),
            "extreme finite samples must still produce a valid 3-vertex polyline"
        );
    }
}
