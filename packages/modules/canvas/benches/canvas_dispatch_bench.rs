//! Criterion microbenchmarks comparing direct `GuardedPainter` dispatch vs recorded `DrawList`
//! full roundtrip vs pre-validated `DrawListView::replay` at N ∈ {10, 100, 1_000} primitives.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nve_abi::ColorRgba;
use nve_canvas::{
    CanvasPainter, CanvasSink, DrawList, DrawListRecorder, GuardedPainter, PolylinePoints,
};
use nve_store::{FlatResource, ValidatedWords};

#[derive(Default)]
struct ChecksumSink {
    acc: u64,
}

impl CanvasSink for ChecksumSink {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        self.acc = self.acc.wrapping_add(u64::from(color.to_u32()));
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        self.acc = self
            .acc
            .wrapping_add(u64::from(
                x.to_bits() ^ y.to_bits() ^ w.to_bits() ^ h.to_bits() ^ color.to_u32(),
            ));
    }

    #[inline]
    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    ) {
        self.acc = self.acc.wrapping_add(u64::from(
            x.to_bits()
                ^ y.to_bits()
                ^ w.to_bits()
                ^ h.to_bits()
                ^ radius.to_bits()
                ^ color.to_u32(),
        ));
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        self.acc = self.acc.wrapping_add(u64::from(
            cx.to_bits() ^ cy.to_bits() ^ radius.to_bits() ^ color.to_u32(),
        ));
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        self.acc = self.acc.wrapping_add(u64::from(
            x0.to_bits()
                ^ y0.to_bits()
                ^ x1.to_bits()
                ^ y1.to_bits()
                ^ width.to_bits()
                ^ color.to_u32(),
        ));
    }

    #[inline]
    fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        let mut h = u64::from(width.to_bits() ^ color.to_u32());
        for (px, py) in points.iter() {
            h = h.wrapping_add(u64::from(px.to_bits() ^ py.to_bits()));
        }
        self.acc = self.acc.wrapping_add(h);
    }

    #[inline]
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.acc = self
            .acc
            .wrapping_add(u64::from(x.to_bits() ^ y.to_bits() ^ w.to_bits() ^ h.to_bits()));
    }

    #[inline]
    fn pop_clip(&mut self) {
        self.acc = self.acc.wrapping_add(1);
    }
}

fn emit_mixed_primitives(p: &mut dyn CanvasPainter, count: usize) {
    let poly = [
        (4.0f32, 6.0),
        (10.0, 12.0),
        (16.0, 8.0),
        (22.0, 14.0),
        (28.0, 10.0),
        (34.0, 18.0),
        (40.0, 12.0),
        (46.0, 20.0),
    ];
    p.clear(ColorRgba::rgb(12, 16, 24));
    for i in 0..count {
        let x = ((i * 13) % 240) as f32;
        let y = ((i * 7) % 240) as f32;
        let color = ColorRgba::rgb((i & 0xFF) as u8, ((i * 3) & 0xFF) as u8, 220);
        match i % 5 {
            0 => p.fill_rect(x, y, 8.0, 6.0, color),
            1 => p.fill_rounded_rect(x, y, 8.0, 8.0, 2.0, color),
            2 => p.fill_circle(x + 4.0, y + 4.0, 3.5, color),
            3 => p.stroke_line(x, y, x + 8.0, y + 6.0, 1.5, color),
            _ => p.stroke_polyline(&poly, 1.5, color),
        }
    }
}

fn bench_canvas_dispatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("canvas_dispatch");
    for &n in &[10usize, 100, 1_000] {
        group.bench_with_input(BenchmarkId::new("direct_guarded_painter", n), &n, |b, &n| {
            b.iter(|| {
                let mut sink = ChecksumSink::default();
                {
                    let mut painter = GuardedPainter::new(&mut sink);
                    let dyn_p: &mut dyn CanvasPainter = &mut painter;
                    emit_mixed_primitives(dyn_p, black_box(n));
                }
                black_box(sink.acc)
            });
        });

        group.bench_with_input(
            BenchmarkId::new("recorded_full_roundtrip", n),
            &n,
            |b, &n| {
                let mut words = Vec::with_capacity(n * 8 + 16);
                b.iter(|| {
                    let mut rec = DrawListRecorder::with_capacity(n + 1);
                    emit_mixed_primitives(&mut rec, black_box(n));
                    let dl = rec.finish();
                    words.clear();
                    assert!(dl.write_words(&mut words));
                    let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid");
                    let mut sink = ChecksumSink::default();
                    {
                        let mut painter = GuardedPainter::new(&mut sink);
                        DrawList::view(vw).replay(&mut painter);
                    }
                    black_box(sink.acc)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("recorded_into_words_roundtrip", n),
            &n,
            |b, &n| {
                let mut words = Vec::with_capacity(n * 8 + 16);
                b.iter(|| {
                    words.clear();
                    DrawList::record_into_words(&mut words, |rec| {
                        emit_mixed_primitives(rec, black_box(n));
                    });
                    let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid");
                    let mut sink = ChecksumSink::default();
                    {
                        let mut painter = GuardedPainter::new(&mut sink);
                        DrawList::view(vw).replay(&mut painter);
                    }
                    black_box(sink.acc)
                });
            },
        );

        let dl = DrawList::record_with_capacity(n + 1, |rec| emit_mixed_primitives(rec, n));
        let mut pre_words = Vec::new();
        assert!(dl.write_words(&mut pre_words));
        let pre_vw = ValidatedWords::<DrawList>::try_new(&pre_words).expect("valid");
        let pre_view = DrawList::view(pre_vw);

        group.bench_with_input(BenchmarkId::new("recorded_replay_only", n), &n, |b, _| {
            b.iter(|| {
                let mut sink = ChecksumSink::default();
                {
                    let mut painter = GuardedPainter::new(&mut sink);
                    black_box(pre_view).replay(&mut painter);
                }
                black_box(sink.acc)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_canvas_dispatch);
criterion_main!(benches);
