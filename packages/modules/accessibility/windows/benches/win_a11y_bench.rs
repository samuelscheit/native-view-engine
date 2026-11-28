//! Criterion benchmarks for `nve-accessibility-windows` `NveSceneView` -> `AccessKitTreeBridge`
//! full, incremental-label, and scroll-offset-only mirroring (measuring only `sync_scene`).

#[cfg(not(target_arch = "wasm32"))]
mod native_bench {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::table::NveTableView;
    use nve_abi::{InteractionEvent, NveLayoutRect, NveSceneView, NveScrollOffset};
    use nve_accessibility_windows::{NoopResources, WinA11yBridge};
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_tables::tables::{
        core_realization_order, Accessibility, AriaRole, Interaction, Scroll, ScrollAxes,
    };

    struct FrozenSnapshot {
        store: NodeStore,
        tables: Vec<NveTableView>,
        _dirty_buf: Vec<u32>,
        rects: Vec<NveLayoutRect>,
        scrolls: Vec<NveScrollOffset>,
        dirty_idx: Vec<u32>,
        moved_idx: Vec<u32>,
    }

    impl FrozenSnapshot {
        fn build(count: usize, label_variant: &str, scroll_y: f32) -> (Self, NodeId) {
            let mut store = NodeStore::new();
            store.set_realization_order(core_realization_order());
            let mut changes = ChangeSet::default();
            let (scroller, target) = {
                let mut tx = store.tx();
                let scroller = tx.create();
                tx.edit::<Scroll>(scroller).axes(ScrollAxes::Vertical);
                let mut list = Vec::with_capacity(count);
                for i in 0..count {
                    let n = tx.create();
                    let lbl = if i == count / 2 {
                        label_variant.to_owned()
                    } else {
                        format!("Row {i}")
                    };
                    tx.edit::<Accessibility>(n)
                        .role(Some(AriaRole::Button))
                        .label(&lbl);
                    tx.edit::<Interaction>(n).listen(InteractionEvent::Press);
                    list.push(n);
                }
                tx.set_children(scroller, &list);
                tx.set_children(NodeId::ROOT, &[scroller]);
                tx.commit_into(&mut changes);
                (scroller, list[count / 2])
            };
            let mut tables = Vec::new();
            let mut dirty_buf = Vec::new();
            store.publish_table_views(&changes, &mut tables, &mut dirty_buf);
            let rects = (0..store.node_rows().len())
                .map(|i| NveLayoutRect {
                    x: 0.0,
                    y: (i as f32) * 24.0,
                    width: 240.0,
                    height: 24.0,
                })
                .collect();
            let scrolls = vec![NveScrollOffset {
                node_id: scroller.raw(),
                x: 0.0,
                y: scroll_y,
            }];
            (
                Self {
                    store,
                    tables,
                    _dirty_buf: dirty_buf,
                    rects,
                    scrolls,
                    dirty_idx: vec![target.index() as u32],
                    moved_idx: Vec::new(),
                },
                target,
            )
        }

        fn view(&self, frame_id: u64, scene_epoch: u32, with_dirty: bool) -> NveSceneView {
            let rows = self.store.node_rows();
            NveSceneView {
                frame_id,
                scene_epoch,
                node_capacity: rows.len() as u32,
                dirty_count: if with_dirty {
                    self.dirty_idx.len() as u32
                } else {
                    0
                },
                dirty_indices: if with_dirty {
                    self.dirty_idx.as_ptr()
                } else {
                    core::ptr::null()
                },
                moved_count: self.moved_idx.len() as u32,
                moved_indices: self.moved_idx.as_ptr(),
                rows: rows.as_ptr().cast(),
                rects: self.rects.as_ptr(),
                text_slab: self.store.text_slab().as_ptr(),
                text_slab_len: self.store.text_slab().len() as u32,
                children_slab: self.store.children_slab().as_ptr().cast(),
                children_slab_len: self.store.children_slab().len() as u32,
                tables: self.tables.as_ptr(),
                table_count: self.tables.len() as u32,
                scroll_offsets: self.scrolls.as_ptr(),
                scroll_count: self.scrolls.len() as u32,
                ..NveSceneView::default()
            }
        }
    }

    pub fn bench_win_a11y(c: &mut Criterion) {
        // Pre-build toggling snapshots outside `b.iter()` so benchmarks measure ONLY `sync_scene`.
        let (snap_a, _target) = FrozenSnapshot::build(500, "Label A", 0.0);
        let (snap_b, _) = FrozenSnapshot::build(500, "Label B", 0.0);
        let (snap_scroll_a, _) = FrozenSnapshot::build(500, "Label A", 0.0);
        let (snap_scroll_b, _) = FrozenSnapshot::build(500, "Label A", 64.0);

        c.bench_function("win_a11y_full_mirror_500_nodes", |b| {
            let mut epoch = 0u32;
            let mut mirror = WinA11yBridge::new();
            mirror.activate();
            b.iter(|| {
                epoch = epoch.wrapping_add(1);
                let scene = snap_a.view(1, epoch, false);
                // SAFETY: `scene` references live local slices for this iteration.
                let upd = unsafe {
                    mirror.sync_scene(black_box(&scene), &NoopResources, 0, 1.0, "Bench")
                };
                black_box(upd.map_or(0, |u| u.nodes.len()))
            });
        });

        let mut mirror = WinA11yBridge::new();
        mirror.activate();
        let init_scene = snap_a.view(1, 1, false);
        // SAFETY: `init_scene` references live local slices.
        let _ = unsafe { mirror.sync_scene(&init_scene, &NoopResources, 0, 1.0, "Bench") };

        let mut frame = 1u64;
        c.bench_function("win_a11y_incremental_single_label_500_nodes", |b| {
            b.iter(|| {
                frame = frame.wrapping_add(1);
                let snap = if frame.is_multiple_of(2) {
                    &snap_b
                } else {
                    &snap_a
                };
                let scene = snap.view(frame, 1, true);
                // SAFETY: `scene` references live local slices for this iteration.
                let upd = unsafe {
                    mirror.sync_scene(black_box(&scene), &NoopResources, 0, 1.0, "Bench")
                };
                black_box(upd.map_or(0, |u| u.nodes.len()))
            });
        });

        let mut scroll_mirror = WinA11yBridge::new();
        scroll_mirror.activate();
        let scroll_init = snap_scroll_a.view(1, 1, false);
        // SAFETY: `scroll_init` references live local slices.
        let _ = unsafe { scroll_mirror.sync_scene(&scroll_init, &NoopResources, 0, 1.0, "Bench") };

        let mut sframe = 1u64;
        c.bench_function("win_a11y_scroll_only_500_nodes", |b| {
            b.iter(|| {
                sframe = sframe.wrapping_add(1);
                let snap = if sframe.is_multiple_of(2) {
                    &snap_scroll_b
                } else {
                    &snap_scroll_a
                };
                let scene = snap.view(sframe, 1, false);
                // SAFETY: `scene` references live local slices for this iteration.
                let upd = unsafe {
                    scroll_mirror.sync_scene(black_box(&scene), &NoopResources, 0, 1.0, "Bench")
                };
                black_box(upd.map_or(0, |u| u.nodes.len()))
            });
        });
    }

    criterion_group!(benches, bench_win_a11y);

    pub fn run() {
        benches();
        criterion::Criterion::default()
            .configure_from_args()
            .final_summary();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native_bench::run();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
