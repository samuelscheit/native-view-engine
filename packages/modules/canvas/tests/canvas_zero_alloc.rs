//! Thread-local `CountingAlloc` zero-allocation hot-path guards for `nve-canvas`:
//! - positive control probe
//! - `DrawList::validate` and `DrawListView::ops`
//! - `DrawListView::replay` into `GuardedPainter<S>` (all 8 ops + 66-deep overflow clip + 64-pt polyline)
//! - `DirectCanvasRegistry::paint` and 120 Hz `invalidate` + `tx.commit_into`
//! - `GuardedPainter` NaN/Inf polyline splitting
//! - `paint_sparkline` stack-buffered painting (up to 256 samples)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::ColorRgba;
use nve_canvas::{
    paint_sparkline, CanvasOp, CanvasOpRef, CanvasPaintSize, CanvasPainter, CanvasSink,
    DirectCanvasRegistry, DrawList, DrawListRecorder, GuardedPainter, PolylinePoints,
    MAX_CLIP_DEPTH,
};
use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore, ValidatedWords};

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

static SERIAL_TEST: Mutex<()> = Mutex::new(());

struct CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System` while counting per-thread allocations.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let r = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (after.wrapping_sub(before), r)
}

#[derive(Default)]
struct CountingSink {
    ops: usize,
    vertices: usize,
    clips_pushed: usize,
    clips_popped: usize,
    checksum: u64,
}

impl CanvasSink for CountingSink {
    fn clear(&mut self, color: ColorRgba) {
        self.ops += 1;
        self.checksum = self.checksum.wrapping_add(u64::from(color.to_u32()));
    }
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        self.ops += 1;
        self.checksum = self
            .checksum
            .wrapping_add(u64::from(x.to_bits() ^ y.to_bits() ^ w.to_bits() ^ h.to_bits()))
            .wrapping_add(u64::from(color.to_u32()));
    }
    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    ) {
        self.ops += 1;
        self.checksum = self
            .checksum
            .wrapping_add(u64::from(
                x.to_bits() ^ y.to_bits() ^ w.to_bits() ^ h.to_bits() ^ radius.to_bits(),
            ))
            .wrapping_add(u64::from(color.to_u32()));
    }
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        self.ops += 1;
        self.checksum = self
            .checksum
            .wrapping_add(u64::from(cx.to_bits() ^ cy.to_bits() ^ radius.to_bits()))
            .wrapping_add(u64::from(color.to_u32()));
    }
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        self.ops += 1;
        self.checksum = self
            .checksum
            .wrapping_add(u64::from(
                x0.to_bits() ^ y0.to_bits() ^ x1.to_bits() ^ y1.to_bits() ^ width.to_bits(),
            ))
            .wrapping_add(u64::from(color.to_u32()));
    }
    fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        self.ops += 1;
        self.checksum = self
            .checksum
            .wrapping_add(u64::from(width.to_bits() ^ color.to_u32()));
        for (px, py) in points.iter() {
            self.vertices += 1;
            self.checksum = self
                .checksum
                .wrapping_add(u64::from(px.to_bits() ^ py.to_bits()));
        }
    }
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.clips_pushed += 1;
        self.checksum = self
            .checksum
            .wrapping_add(u64::from(x.to_bits() ^ y.to_bits() ^ w.to_bits() ^ h.to_bits()));
    }
    fn pop_clip(&mut self) {
        self.clips_popped += 1;
    }
}

#[test]
fn counting_alloc_probe_detects_real_heap_allocation() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (probe_allocs, probe_vec) = measure_allocs(|| {
        let mut v = Vec::with_capacity(64);
        v.push(black_box(42u32));
        v
    });
    black_box(&probe_vec);
    assert!(
        probe_allocs >= 1,
        "CountingAlloc positive probe must observe >= 1 allocation, got {probe_allocs}"
    );
}

#[test]
fn draw_list_validate_and_view_ops_allocate_zero_bytes() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut ops = vec![
        CanvasOp::Clear {
            color: ColorRgba::BLACK,
        },
        CanvasOp::FillRect {
            x: 1.0,
            y: 2.0,
            w: 40.0,
            h: 20.0,
            color: ColorRgba::WHITE,
        },
        CanvasOp::FillRoundedRect {
            x: 4.0,
            y: 5.0,
            w: 20.0,
            h: 10.0,
            radius: 3.0,
            color: ColorRgba::rgb(1, 2, 3),
        },
        CanvasOp::FillCircle {
            cx: 12.0,
            cy: 14.0,
            radius: 6.0,
            color: ColorRgba::rgb(4, 5, 6),
        },
        CanvasOp::StrokeLine {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 10.0,
            width: 2.0,
            color: ColorRgba::rgb(7, 8, 9),
        },
        CanvasOp::StrokePolyline {
            points: vec![(0.0, 0.0), (5.0, 10.0), (10.0, 2.0), (16.0, 8.0)],
            width: 1.5,
            color: ColorRgba::rgb(9, 8, 7),
        },
    ];
    for _ in 0..(MAX_CLIP_DEPTH + 2) {
        ops.push(CanvasOp::ClipRect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        });
    }
    ops.push(CanvasOp::RestoreClip);
    ops.push(CanvasOp::RestoreClip);
    ops.push(CanvasOp::RestoreClip);

    let mut words = Vec::new();
    assert!(DrawList::from_ops(ops).write_words(&mut words));
    let mut bad_words = words.clone();
    bad_words[4] = f32::NAN.to_bits();

    let (allocs, (validated_count, ops_seen, pts_seen)) = measure_allocs(|| {
        let mut valid_hits = 0usize;
        let mut ops = 0usize;
        let mut pts = 0usize;
        for _ in 0..64 {
            assert!(DrawList::validate(black_box(&words)));
            assert!(!DrawList::validate(black_box(&bad_words)));
            valid_hits += 1;

            let vw =
                ValidatedWords::<DrawList>::try_new(black_box(&words)).expect("valid draw list");
            let view = DrawList::view(vw);
            for op in view.ops() {
                ops += 1;
                match op {
                    CanvasOpRef::StrokePolyline {
                        points,
                        width,
                        color,
                    } => {
                        black_box((width, color));
                        for pt in points.iter() {
                            black_box(pt);
                            pts += 1;
                        }
                    }
                    other => {
                        black_box(other);
                    }
                }
            }
        }
        (valid_hits, ops, pts)
    });

    assert_eq!(validated_count, 64);
    assert_eq!(ops_seen, 64 * (6 + MAX_CLIP_DEPTH + 2 + 3));
    assert_eq!(pts_seen, 64 * 4);
    assert_eq!(
        allocs, 0,
        "DrawList::validate and DrawListView::ops must allocate 0 bytes"
    );
}

#[test]
fn draw_list_view_replay_into_guarded_sink_allocates_zero() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let poly_64: Vec<(f32, f32)> = (0..64)
        .map(|i| (i as f32 * 2.0, ((i * 7) % 32) as f32))
        .collect();
    let mut ops = vec![
        CanvasOp::Clear {
            color: ColorRgba::BLACK,
        },
        CanvasOp::FillRect {
            x: 1.0,
            y: 2.0,
            w: 40.0,
            h: 20.0,
            color: ColorRgba::WHITE,
        },
        CanvasOp::FillRoundedRect {
            x: 4.0,
            y: 5.0,
            w: 20.0,
            h: 10.0,
            radius: 3.0,
            color: ColorRgba::rgb(1, 2, 3),
        },
        CanvasOp::FillCircle {
            cx: 12.0,
            cy: 14.0,
            radius: 6.0,
            color: ColorRgba::rgb(4, 5, 6),
        },
        CanvasOp::StrokeLine {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 10.0,
            width: 2.0,
            color: ColorRgba::rgb(7, 8, 9),
        },
        CanvasOp::StrokePolyline {
            points: poly_64,
            width: 1.5,
            color: ColorRgba::rgb(9, 8, 7),
        },
    ];
    for _ in 0..(MAX_CLIP_DEPTH + 2) {
        ops.push(CanvasOp::ClipRect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        });
    }
    ops.push(CanvasOp::RestoreClip);
    ops.push(CanvasOp::RestoreClip);

    let mut words = Vec::new();
    assert!(DrawList::from_ops(ops).write_words(&mut words));
    let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid draw list");
    let view = DrawList::view(vw);

    let (allocs, sink) = measure_allocs(|| {
        let mut sink = CountingSink::default();
        for _ in 0..32 {
            let mut painter = GuardedPainter::new(&mut sink);
            black_box(view).replay(&mut painter);
        }
        sink
    });
    assert_eq!(allocs, 0, "DrawListView::replay into GuardedPainter must allocate 0 bytes");
    assert_eq!(sink.ops, 32 * 6);
    assert_eq!(sink.vertices, 32 * 64);
    assert_eq!(sink.clips_pushed, 32 * MAX_CLIP_DEPTH);
    assert_eq!(sink.clips_popped, 32 * MAX_CLIP_DEPTH);
}

#[test]
fn direct_registry_paint_and_invalidate_allocate_zero() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let reg = DirectCanvasRegistry::new();
    let frame = Arc::new(AtomicU32::new(0));
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();

    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        let frame_cb = Arc::clone(&frame);
        reg.bind(&mut tx, n, move |p, size| {
            let f = frame_cb.load(Ordering::Relaxed) as f32;
            p.clear(ColorRgba::BLACK);
            p.fill_rect(f % 16.0, 2.0, size.width * 0.5, size.height * 0.5, ColorRgba::WHITE);
        });
        tx.commit_into(&mut changes);
        n
    };

    // Warm 1 invalidation + commit cycle so `ChangeSet` scratch buffers reach steady-state capacity.
    {
        let mut tx = store.tx();
        assert!(reg.invalidate(&mut tx, node).is_some());
        tx.commit_into(&mut changes);
    }

    let size = CanvasPaintSize::new(320.0, 180.0, 2.0);
    let (allocs, sink) = measure_allocs(|| {
        let mut sink = CountingSink::default();
        for i in 0..120u32 {
            frame.store(i, Ordering::Relaxed);
            {
                let mut tx = store.tx();
                let g = reg.invalidate(&mut tx, node).expect("bound node");
                black_box(g);
                tx.commit_into(&mut changes);
            }
            let mut painter = GuardedPainter::new(&mut sink);
            assert!(reg.paint(node, &mut painter, size));
        }
        sink
    });
    assert_eq!(
        allocs, 0,
        "120 Hz DirectCanvasRegistry::invalidate + commit + paint must allocate 0 bytes"
    );
    assert_eq!(sink.ops, 120 * 2);
}

#[test]
fn guarded_painter_nan_polyline_split_allocates_zero() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let pts = [
        (0.0f32, 1.0),
        (2.0, 3.0),
        (f32::NAN, 4.0),
        (5.0, 6.0),
        (7.0, 8.0),
        (9.0, f32::INFINITY),
        (10.0, 11.0),
        (f32::NEG_INFINITY, 12.0),
        (13.0, 14.0),
        (15.0, 16.0),
    ];

    let (allocs, sink) = measure_allocs(|| {
        let mut sink = CountingSink::default();
        for _ in 0..64 {
            let mut painter = GuardedPainter::new(&mut sink);
            painter.stroke_polyline(black_box(&pts), 2.0, ColorRgba::WHITE);
        }
        sink
    });
    assert_eq!(allocs, 0, "NaN/Inf polyline splitting must allocate 0 bytes");
    // 4 finite sub-slices with 2 + 2 + 1 + 2 = 7 finite vertices.
    assert_eq!(sink.ops, 64 * 4);
    assert_eq!(sink.vertices, 64 * 7);
}

#[test]
fn paint_sparkline_stack_buffer_allocates_zero() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut samples = [0.0f32; 256];
    for (i, slot) in samples.iter_mut().enumerate() {
        *slot = if i % 37 == 0 {
            f32::NAN
        } else {
            ((i as f32) * 0.25).sin() * 40.0 + 50.0
        };
    }

    let (allocs, sink) = measure_allocs(|| {
        let mut sink = CountingSink::default();
        for _ in 0..32 {
            let mut painter = GuardedPainter::new(&mut sink);
            paint_sparkline(
                &mut painter,
                black_box(&samples),
                320.0,
                72.0,
                ColorRgba::rgb(56, 189, 248),
                ColorRgba::rgb(15, 23, 42),
            );
        }
        sink
    });
    assert_eq!(allocs, 0, "paint_sparkline (<= 256 samples) must allocate 0 bytes");
    assert_eq!(sink.clips_pushed, 32);
    assert_eq!(sink.clips_popped, 32);
    assert!(sink.vertices > 0);
    // Sanity: DrawListRecorder::sparkline matches paint_sparkline op stream.
    let dl = DrawList::sparkline(
        &samples,
        320.0,
        72.0,
        ColorRgba::rgb(56, 189, 248),
        ColorRgba::rgb(15, 23, 42),
    );
    let mut rec = DrawListRecorder::new();
    rec.sparkline(
        &samples,
        320.0,
        72.0,
        ColorRgba::rgb(56, 189, 248),
        ColorRgba::rgb(15, 23, 42),
    );
    assert_eq!(dl, rec.finish());
}

#[test]
fn record_into_words_ephemeral_with_allocates_zero_when_warm() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let poly = [(0.0f32, 0.0), (5.0, 10.0), (10.0, 2.0), (16.0, 8.0)];
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<nve_canvas::Canvas>(n)
            .draw_list_ephemeral_with(|out| {
                DrawList::record_into_words(out, |rec| {
                    rec.clear(ColorRgba::BLACK)
                        .fill_rect(1.0, 2.0, 40.0, 20.0, ColorRgba::WHITE)
                        .fill_rounded_rect(4.0, 5.0, 20.0, 10.0, 3.0, ColorRgba::rgb(1, 2, 3))
                        .fill_circle(12.0, 14.0, 6.0, ColorRgba::rgb(4, 5, 6))
                        .stroke_line(0.0, 0.0, 10.0, 10.0, 2.0, ColorRgba::rgb(7, 8, 9))
                        .stroke_polyline(&poly, 1.5, ColorRgba::rgb(9, 8, 7));
                });
                true
            })
            .generation(1);
        tx.commit_into(&mut changes);
        n
    };

    // Warm 1 additional ephemeral update so `ResourceStore`'s scratch `Vec<u32>` and `ChangeSet`
    // reach steady-state capacity.
    {
        let mut tx = store.tx();
        tx.edit::<nve_canvas::Canvas>(node)
            .draw_list_ephemeral_with(|out| {
                DrawList::record_into_words(out, |rec| {
                    rec.clear(ColorRgba::BLACK)
                        .fill_rect(2.0, 2.0, 40.0, 20.0, ColorRgba::WHITE)
                        .fill_rounded_rect(4.0, 5.0, 20.0, 10.0, 3.0, ColorRgba::rgb(1, 2, 3))
                        .fill_circle(12.0, 14.0, 6.0, ColorRgba::rgb(4, 5, 6))
                        .stroke_line(0.0, 0.0, 10.0, 10.0, 2.0, ColorRgba::rgb(7, 8, 9))
                        .stroke_polyline(&poly, 1.5, ColorRgba::rgb(9, 8, 7));
                });
                true
            })
            .generation(2);
        tx.commit_into(&mut changes);
    }

    let (allocs, ()) = measure_allocs(|| {
        for gen_num in 3..63u32 {
            let mut tx = store.tx();
            tx.edit::<nve_canvas::Canvas>(node)
                .draw_list_ephemeral_with(|out| {
                    DrawList::record_into_words(out, |rec| {
                        rec.clear(ColorRgba::BLACK)
                            .fill_rect((gen_num % 16) as f32, 2.0, 40.0, 20.0, ColorRgba::WHITE)
                            .fill_rounded_rect(4.0, 5.0, 20.0, 10.0, 3.0, ColorRgba::rgb(1, 2, 3))
                            .fill_circle(12.0, 14.0, 6.0, ColorRgba::rgb(4, 5, 6))
                            .stroke_line(0.0, 0.0, 10.0, 10.0, 2.0, ColorRgba::rgb(7, 8, 9))
                            .stroke_polyline(black_box(&poly), 1.5, ColorRgba::rgb(9, 8, 7));
                    });
                    true
                })
                .generation(gen_num);
            tx.commit_into(&mut changes);
        }
    });
    assert_eq!(
        allocs, 0,
        "warm DrawList::record_into_words + draw_list_ephemeral_with must allocate 0 bytes"
    );
    let row = store.try_row::<nve_canvas::Canvas>(node).expect("canvas");
    assert_eq!(
        store.resource::<DrawList>(row.draw_list()).expect("dl").len(),
        6
    );
}

#[test]
fn direct_registry_paint_allows_reentrant_remove_and_invalidate_without_deadlock() {
    let reg = DirectCanvasRegistry::new();
    let store = Arc::new(Mutex::new(NodeStore::new()));
    let mut changes = ChangeSet::default();
    let (node_self_remove, node_peer) = {
        let mut s = store.lock().unwrap_or_else(|e| e.into_inner());
        let mut tx = s.tx();
        let a = tx.create();
        let b = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b]);
        let reg_cb = reg.clone();
        let store_cb = Arc::clone(&store);
        reg.bind(&mut tx, a, move |p, _| {
            // Re-entrantly remove ourselves and invalidate a sibling node inside `paint`!
            assert!(reg_cb.remove(a));
            let mut s = store_cb.lock().unwrap_or_else(|e| e.into_inner());
            let mut inner_tx = s.tx();
            assert!(reg_cb.invalidate(&mut inner_tx, b).is_some());
            p.clear(ColorRgba::BLACK);
        });
        reg.bind(&mut tx, b, |p, _| p.clear(ColorRgba::WHITE));
        tx.commit_into(&mut changes);
        (a, b)
    };

    let mut sink = CountingSink::default();
    {
        let mut painter = GuardedPainter::new(&mut sink);
        assert!(reg.paint(
            node_self_remove,
            &mut painter,
            CanvasPaintSize::new(32.0, 32.0, 1.0)
        ));
    }
    assert!(!reg.contains(node_self_remove));
    assert!(reg.contains(node_peer));
    assert_eq!(sink.ops, 1);
}

#[test]
fn draw_list_sink_suppresses_draws_inside_max_draw_ops_skipped_clip() {
    use nve_canvas::MAX_DRAW_OPS;

    let dl = DrawList::record_with_capacity(MAX_DRAW_OPS, |rec| {
        // Fill exactly `MAX_DRAW_OPS - 1` draw ops so only 1 op slot remains.
        for _ in 0..(MAX_DRAW_OPS - 1) {
            rec.clear(ColorRgba::BLACK);
        }
        // This clip needs 2 slots (ClipRect + RestoreClip), so it MUST be skipped.
        rec.clip_rect(0.0, 0.0, 4.0, 4.0);
        // This draw MUST NOT escape unclipped into the parent clip!
        rec.fill_rect(0.0, 0.0, 100.0, 100.0, ColorRgba::rgb(255, 0, 0));
        rec.restore_clip();
    });
    assert_eq!(dl.len(), MAX_DRAW_OPS - 1);
    assert!(
        dl.ops().all(|op| matches!(op, CanvasOpRef::Clear { .. })),
        "draws inside a MAX_DRAW_OPS-skipped clip must never escape unclipped"
    );
}
