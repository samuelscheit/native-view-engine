//! Criterion benchmarks for `nve-accessibility-windows` `NveSceneView` -> `AccessKitTreeBridge`
//! full and incremental mirroring.

#[cfg(not(target_arch = "wasm32"))]
mod native_bench {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{InteractionEvent, NveLayoutRect, NveSceneView};
    use nve_accessibility_windows::{NoopResources, WinA11yBridge};
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_tables::tables::{core_realization_order, Accessibility, AriaRole, Interaction};

    fn build_store(count: usize) -> (NodeStore, ChangeSet, Vec<NodeId>) {
        let mut store = NodeStore::new();
        store.set_realization_order(core_realization_order());
        let mut changes = ChangeSet::default();
        let nodes = {
            let mut tx = store.tx();
            let mut list = Vec::with_capacity(count);
            for i in 0..count {
                let n = tx.create();
                tx.edit::<Accessibility>(n)
                    .role(Some(AriaRole::Button))
                    .label(&format!("Row {i}"));
                tx.edit::<Interaction>(n).listen(InteractionEvent::Press);
                list.push(n);
            }
            tx.set_children(NodeId::ROOT, &list);
            tx.commit_into(&mut changes);
            list
        };
        (store, changes, nodes)
    }

    pub fn bench_win_a11y(c: &mut Criterion) {
        let (mut store, mut changes, nodes) = build_store(500);
        let mut tables = Vec::new();
        let mut dirty_buf = Vec::new();
        store.publish_table_views(&changes, &mut tables, &mut dirty_buf);
        let rects = vec![
            NveLayoutRect {
                x: 0.0,
                y: 0.0,
                width: 240.0,
                height: 24.0,
            };
            store.node_rows().len()
        ];

        c.bench_function("win_a11y_full_mirror_500_nodes", |b| {
            let mut epoch = 0u32;
            let mut mirror = WinA11yBridge::new();
            mirror.activate();
            b.iter(|| {
                epoch = epoch.wrapping_add(1);
                let rows = store.node_rows();
                let scene = NveSceneView {
                    frame_id: 1,
                    scene_epoch: epoch,
                    node_capacity: rows.len() as u32,
                    rows: rows.as_ptr().cast(),
                    rects: rects.as_ptr(),
                    text_slab: store.text_slab().as_ptr(),
                    text_slab_len: store.text_slab().len() as u32,
                    children_slab: store.children_slab().as_ptr().cast(),
                    children_slab_len: store.children_slab().len() as u32,
                    tables: tables.as_ptr(),
                    table_count: tables.len() as u32,
                    ..NveSceneView::default()
                };
                // SAFETY: `scene` references live local slices for this iteration.
                let upd = unsafe {
                    mirror.sync_scene(black_box(&scene), &NoopResources, 0, 1.0, "Bench")
                };
                black_box(upd.map_or(0, |u| u.nodes.len()))
            });
        });

        let mut mirror = WinA11yBridge::new();
        mirror.activate();
        let rows = store.node_rows();
        let init_scene = NveSceneView {
            frame_id: 1,
            scene_epoch: 1,
            node_capacity: rows.len() as u32,
            rows: rows.as_ptr().cast(),
            rects: rects.as_ptr(),
            text_slab: store.text_slab().as_ptr(),
            text_slab_len: store.text_slab().len() as u32,
            children_slab: store.children_slab().as_ptr().cast(),
            children_slab_len: store.children_slab().len() as u32,
            tables: tables.as_ptr(),
            table_count: tables.len() as u32,
            ..NveSceneView::default()
        };
        // SAFETY: `init_scene` references live local slices.
        let _ = unsafe { mirror.sync_scene(&init_scene, &NoopResources, 0, 1.0, "Bench") };

        let target = nodes[250];
        let dirty_idx = [target.index() as u32];
        let mut frame = 1u64;
        c.bench_function("win_a11y_incremental_single_label_500_nodes", |b| {
            b.iter(|| {
                frame += 1;
                let mut tx = store.tx();
                tx.edit::<Accessibility>(target)
                    .label(if frame.is_multiple_of(2) { "A" } else { "B" });
                tx.commit_into(&mut changes);
                store.publish_table_views(&changes, &mut tables, &mut dirty_buf);
                let rows = store.node_rows();
                let scene = NveSceneView {
                    frame_id: frame,
                    scene_epoch: 1,
                    node_capacity: rows.len() as u32,
                    dirty_count: 1,
                    dirty_indices: dirty_idx.as_ptr(),
                    rows: rows.as_ptr().cast(),
                    rects: rects.as_ptr(),
                    text_slab: store.text_slab().as_ptr(),
                    text_slab_len: store.text_slab().len() as u32,
                    children_slab: store.children_slab().as_ptr().cast(),
                    children_slab_len: store.children_slab().len() as u32,
                    tables: tables.as_ptr(),
                    table_count: tables.len() as u32,
                    ..NveSceneView::default()
                };
                // SAFETY: `scene` references live local slices for this iteration.
                let upd = unsafe {
                    mirror.sync_scene(black_box(&scene), &NoopResources, 0, 1.0, "Bench")
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
