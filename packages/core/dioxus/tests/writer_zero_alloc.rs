//! `NveMutationWriter` performs ZERO heap allocations in steady state: same-value and changed
//! attribute updates (standalone + inline `style`), keyed child reorders, subtree removal, and
//! template re-instantiation from a warm `NodeStore` slot pool, all driven through
//! `dioxus_core::WriteMutations`.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
#[path = "support/writer_list.rs"]
mod writer_list;

use counting_alloc::count_allocs;
use nve_abi::SurfaceId;
use nve_tables::tables::{Layout, Visual};
use nve_dioxus::NveMutationWriter;
use nve_store::{Length, NodeId, Rgba};
use writer_list::{
    ITEMS, TABLE_BORDER, all_side_tables_attributes, changed_attributes, inner, item,
    keyed_reorder, mount_list, remove_and_reinstantiate, same_value_attributes, store_changed,
    values,
};

/// Asserts item `k`'s stored attribute values for value set `pick` of
/// [`changed_attributes`].
pub fn assert_item_attributes(writer: &NveMutationWriter, k: usize, pick: usize) {
    let node = writer.resolve_el(item(k)).expect("item bound");
    let store = writer.store();
    let padding = store.try_row::<Layout>(node).map(|l| l.padding_left());
    assert_eq!(
        padding,
        Some(Length::points([8.0, 12.0][pick])),
        "item {k} padding"
    );
    let visual = store.try_row::<Visual>(node).expect("item visual");
    assert_eq!(visual.opacity(), [0.5, 0.75][pick], "item {k} opacity");
    let (bg, radius, width) = [
        (Rgba::rgb(0x1e, 0x29, 0x3b), 4.0, 0.0),
        (Rgba::rgb(0x33, 0x41, 0x55), 0.0, 2.0),
    ][pick];
    assert_eq!(visual.background(), bg, "item {k} inline background");
    assert_eq!(
        visual.border_radius(),
        radius,
        "item {k} inline border-radius"
    );
    assert_eq!(visual.border_width(), width, "item {k} inline border-width");
}

/// Asserts item `k` is a live 4-node subtree whose inner view is bound to [`inner`] and carries
/// value set `pick` of its inline style and table attr.
pub fn assert_item_subtree(writer: &NveMutationWriter, k: usize, pick: usize) {
    let store = writer.store();
    let node = writer.resolve_el(item(k)).expect("item bound");
    let children = store.children(node);
    assert_eq!(children.len(), 2, "item {k}: text + inner view");
    let inner_node: NodeId = children[1];
    assert_eq!(store.children(inner_node).len(), 1, "item {k}: placeholder");
    assert_eq!(
        writer.resolve_el(inner(k)),
        Some(inner_node),
        "item {k}: assign_node_id"
    );
    assert_eq!(writer.element_id_of(inner_node), Some(inner(k)));
    let width = store.try_row::<Layout>(inner_node).map(|l| l.width());
    assert_eq!(
        width,
        Some(Length::points([10.0, 20.0][pick])),
        "item {k} inner width"
    );
    let visual = store.try_row::<Visual>(inner_node).expect("inner visual");
    assert_eq!(
        visual.opacity(),
        [0.25, 0.75][pick],
        "item {k} inner opacity"
    );
    assert_eq!(
        visual.border_radius(),
        [0.0, 3.0][pick],
        "item {k} inner border-radius"
    );
    assert_eq!(
        visual.border_color(),
        TABLE_BORDER[pick],
        "item {k} table attr"
    );
}

#[test]
fn counting_alloc_probe_fires_on_this_thread_only() {
    counting_alloc::assert_counts_only_this_thread();
}

/// Runs `frame` over monotonic frame numbers from `from`: warm-up (four list cycles) grows every
/// scratch buffer, table, and pool to steady size, then eight measured cycles must allocate
/// nothing. Returns the next frame number. `changes` says whether every frame must change the
/// store (no frame may be elided as a no-op, or be a change where none is expected).
fn assert_zero_steady_state(
    name: &str,
    writer: &mut NveMutationWriter,
    from: usize,
    changes: bool,
    mut frame: impl FnMut(&mut NveMutationWriter, usize),
) -> usize {
    let mut run = |writer: &mut NveMutationWriter, frames: core::ops::Range<usize>| {
        for i in frames {
            frame(writer, i);
            assert_eq!(
                store_changed(writer),
                changes,
                "{name}: frame {i} store change"
            );
        }
    };
    let warm = from + ITEMS * 4;
    run(writer, from..warm);
    let end = warm + ITEMS * 8;
    let (_, allocs) = count_allocs(|| run(writer, warm..end));
    assert_eq!(
        allocs,
        0,
        "{name}: {allocs} allocations over {} steady-state frames",
        ITEMS * 8
    );
    end
}

#[test]
fn writer_steady_state_mutations_allocate_nothing() {
    let v = values();
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    mount_list(&mut writer, &v);
    for k in 0..ITEMS {
        assert_item_subtree(&writer, k, 0);
    }

    // Same values: every write is elided (no store change) and allocates nothing.
    changed_attributes(&mut writer, &v, 0);
    assert_zero_steady_state("same-value attributes", &mut writer, 0, false, |w, i| {
        same_value_attributes(w, &v, i)
    });
    assert_item_attributes(&writer, ITEMS - 1, 0);

    let end = assert_zero_steady_state("changed attributes", &mut writer, 1, true, |w, i| {
        changed_attributes(w, &v, i)
    });
    let last = (end - 1) % 2;
    for k in 0..ITEMS {
        assert_item_attributes(&writer, k, last);
    }

    let end = assert_zero_steady_state(
        "all side tables (typed_style + table_attr + standalone + inline)",
        &mut writer,
        0,
        true,
        |w, i| all_side_tables_attributes(w, &v, i),
    );
    let side_pick = (end - 1) % 2;
    for k in 0..ITEMS {
        let node = writer.resolve_el(item(k)).expect("item bound");
        let store = writer.store();
        assert_eq!(
            store.try_row::<Layout>(node).map(|l| l.width()),
            Some(Length::points([120.0, 160.0][side_pick])),
            "item {k} typed_style width"
        );
        assert_eq!(
            store.try_row::<Visual>(node).map(|vis| vis.border_color()),
            Some(TABLE_BORDER[side_pick]),
            "item {k} table_attr border_color"
        );
    }

    let container = writer
        .resolve_el(dioxus_core::ElementId(1))
        .expect("container bound");
    let order_before = writer.store().children(container).to_vec();
    assert_zero_steady_state("keyed reorder", &mut writer, 0, true, keyed_reorder);
    assert_ne!(writer.store().children(container), order_before.as_slice());

    let end = assert_zero_steady_state("remove + re-instantiate", &mut writer, 0, true, |w, i| {
        remove_and_reinstantiate(w, &v, i)
    });

    // Every item is still bound to a live subtree, listed exactly once, with the inner view's
    // style and table attr of its last re-instantiation.
    let children = writer.store().children(container).to_vec();
    assert_eq!(children.len(), ITEMS);
    for k in 0..ITEMS {
        let node = writer.resolve_el(item(k)).expect("item bound");
        assert!(writer.store().is_live(node));
        assert!(children.contains(&node));
        assert_eq!(writer.element_id_of(node), Some(item(k)));
        let last_frame = (0..end).rev().find(|i| i % ITEMS == k).unwrap_or(k);
        assert_item_subtree(&writer, k, (last_frame / ITEMS) % 2);
    }
}
