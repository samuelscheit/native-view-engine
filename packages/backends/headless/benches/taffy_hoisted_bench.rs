//! Criterion `taffy_relayout_1k_hoisted`: `TaffyStoreLayout` sync + relayout of a 1,001-node tree
//! with 200 absolute badges and 100 multi-level fixed toasts, measuring latency and (via a
//! thread-local counting allocator) heap allocations per frame for each lane.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
#[path = "../tests/support/counting_alloc.rs"]
mod counting_alloc;
#[cfg(not(target_arch = "wasm32"))]
#[path = "../tests/support/hoisted_tree.rs"]
mod hoisted_tree;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{Criterion, criterion_group};
    use nve_backend_headless::TaffyStoreLayout;
    use nve_components::tables::core_realization_order;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    use crate::counting_alloc::count_allocs;
    use crate::hoisted_tree::{
        HoistedTree, VIEWPORT, build_hoisted_tree, commit_badge_frame, commit_paint_frame,
    };

    struct Fixture {
        store: NodeStore,
        changes: ChangeSet,
        layout: TaffyStoreLayout,
        tree: HoistedTree,
        /// Independent monotonic frame counter per lane (indexed by `Lane as usize`): a shared
        /// counter skips values within a lane, so revisits could rewrite an identical value that
        /// the store elides, and the frame would measure nothing.
        frames: [usize; 3],
    }

    #[derive(Clone, Copy)]
    #[repr(usize)]
    enum Lane {
        /// `Visual::opacity` edit: clean layout, pass skipped.
        Paint,
        /// Absolute badge `inset_top` edit: card + root positioning passes rewrite hoisted lists.
        OutOfFlow,
        /// Every cache invalidated: full Taffy layout + 1,001-node rounding walk.
        Full,
    }

    impl Fixture {
        fn new() -> Self {
            let mut store = NodeStore::new();
            store.set_realization_order(core_realization_order());
            let mut changes = ChangeSet::default();
            let tree = build_hoisted_tree(&mut store, &mut changes);
            let mut layout = TaffyStoreLayout::new();
            layout.sync_changes(&store, &changes);
            layout.compute_layout(&store, NodeId::ROOT, VIEWPORT);
            let mut fixture = Self {
                store,
                changes,
                layout,
                tree,
                frames: [0; 3],
            };
            for lane in [Lane::Paint, Lane::OutOfFlow, Lane::Full] {
                for _ in 0..fixture.tree.badges.len() * 2 {
                    fixture.run(lane);
                }
            }
            fixture
        }

        /// Runs one frame of `lane`; returns whether the store changed and whether a layout pass
        /// ran, so probe loops can prove every measured frame does real work.
        fn run(&mut self, lane: Lane) -> (bool, bool) {
            let i = self.frames[lane as usize];
            self.frames[lane as usize] += 1;
            let changed = match lane {
                Lane::Paint | Lane::OutOfFlow => {
                    let commit = if matches!(lane, Lane::Paint) {
                        commit_paint_frame
                    } else {
                        commit_badge_frame
                    };
                    commit(&mut self.store, &mut self.changes, &self.tree, i);
                    self.layout.sync_changes(&self.store, &self.changes);
                    !self.changes.touched.is_empty()
                }
                Lane::Full => {
                    self.layout.invalidate_all(&self.store);
                    false
                }
            };
            let ran = self
                .layout
                .compute_layout(&self.store, NodeId::ROOT, VIEWPORT);
            black_box(
                self.layout
                    .layout_of(self.tree.toasts[i % self.tree.toasts.len()]),
            );
            (changed, ran)
        }
    }

    fn bench_taffy_relayout_1k_hoisted(c: &mut Criterion) {
        let mut fixture = Fixture::new();
        let mut group = c.benchmark_group("taffy_relayout_1k_hoisted");
        for (name, lane) in [
            ("paint_frame", Lane::Paint),
            ("oof_badge_frame", Lane::OutOfFlow),
            ("full_relayout", Lane::Full),
        ] {
            const FRAMES: usize = 200;
            let (_, allocs) = count_allocs(|| {
                for _ in 0..FRAMES {
                    let (changed, ran) = fixture.run(lane);
                    match lane {
                        Lane::Paint => assert!(changed && !ran, "paint frame must edit and skip"),
                        Lane::OutOfFlow => {
                            assert!(changed && ran, "badge frame must edit and lay out")
                        }
                        Lane::Full => assert!(ran, "full relayout must run a pass"),
                    }
                }
            });
            eprintln!(
                "taffy_relayout_1k_hoisted/{name}: {:.2} heap allocs/frame",
                allocs as f64 / FRAMES as f64
            );
            group.bench_function(name, |b| b.iter(|| fixture.run(black_box(lane))));
        }
        group.finish();
    }

    criterion_group!(benches, bench_taffy_relayout_1k_hoisted);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // The per-frame allocation counts below are only meaningful if the counter is thread-scoped.
    counting_alloc::assert_counts_only_this_thread();
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
