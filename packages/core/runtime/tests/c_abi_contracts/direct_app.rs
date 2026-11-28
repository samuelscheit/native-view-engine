//! Direct-mode `RuntimeApp` column reallocation, transient node cancellation, dirty-row
//! deduplication, and `rt_last_error` UTF-8 boundary contract tests.

use std::ffi::c_void;

use nve_abi::{
    EventBatchBuf, FrameId, NveRuntimeStatus, SurfaceId, TimestampUs, ValidatedEvents,
};
use nve_engine::Viewport;
use nve_runtime::{AppFrame, FrameTickHook, RuntimeApp, RuntimeBuilder};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Transaction};
use nve_tables::tables::{Layout, Scroll, Visual};

use super::get_api;

/// Direct-mode `RuntimeApp` whose `dispatch_events` reallocates the sparse `Scroll` column
/// (`rows` + `owners`) WITHOUT modifying `node_rows`, `text_slab`, or `children_slab` (P0 #1).
struct TableReallocInEventApp {
    store: NodeStore,
    changes: ChangeSet,
    nodes: Vec<NodeId>,
    mounted: bool,
}

impl TableReallocInEventApp {
    fn new() -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let mut nodes = Vec::with_capacity(64);
        {
            let mut tx = store.tx();
            for _ in 0..64 {
                let id = tx.create();
                tx.edit::<Layout>(id)
                    .width(Length::points(20.0))
                    .height(Length::points(10.0));
                nodes.push(id);
            }
            // Attach sparse `Scroll` on ONLY node 0 initially (`row_count == 1`).
            tx.edit::<Scroll>(nodes[0]);
            tx.set_children(NodeId::ROOT, &nodes);
            tx.commit_into(&mut changes);
        }
        Self {
            store,
            changes,
            nodes,
            mounted: false,
        }
    }
}

impl RuntimeApp for TableReallocInEventApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if !self.mounted {
            self.mounted = true;
            AppFrame::Direct {
                changes: &self.changes,
            }
        } else {
            AppFrame::Clean
        }
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        // Attach sparse `Scroll` on all 63 remaining existing nodes: reallocates `Scroll`'s
        // `rows` and `owners` Vecs WITHOUT reallocating `node_rows`, `text_slab`, or `children_slab`!
        let mut tx = self.store.tx();
        for &id in &self.nodes[1..] {
            tx.edit::<Scroll>(id);
        }
        tx.commit_into(&mut self.changes);
        true
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

#[test]
fn direct_app_table_column_reallocation_in_dispatch_events_invalidates_published_view() {
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .app(Box::new(TableReallocInEventApp::new()))
        .build();

    assert!(rt.tick(0.016).expect("initial mount tick"));
    let (orig_node_rows, orig_scroll_rows, orig_scroll_owners) = {
        let g = rt.acquire_scene().expect("acquire 1");
        let v = g.view();
        assert!(!v.tables.is_null() && v.table_count >= 1);
        // SAFETY: `g` holds the reader lock for `table_count` entries.
        let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
        let scr = tables
            .iter()
            .find(|t| t.slot == Scroll::slot().0 as u32)
            .expect("Scroll table published");
        assert_eq!(scr.row_count, 1);
        assert!(!scr.rows.is_null() && !scr.owners.is_null());
        (v.rows, scr.rows, scr.owners)
    };

    // Trigger sparse `Scroll` column reallocation during `dispatch_events` (without touching node_rows).
    let mut ev_buf = EventBatchBuf::new();
    ev_buf.push_press(SurfaceId::ROOT, NodeId::new(2, 0), TimestampUs(100), 5.0, 5.0);
    assert!(rt.dispatch_events(ev_buf.as_validated()).expect("dispatch_events"));

    // `acquire_scene` BEFORE the next `tick` MUST return a cleared view rather than dangling pointers!
    {
        let g = rt.acquire_scene().expect("acquire after table realloc");
        assert!(
            g.view().tables.is_null() && g.view().table_count == 0 && g.view().rows.is_null(),
            "reallocating Scroll table column in dispatch_events must invalidate published_view"
        );
    }

    // Next `tick` republishes fresh valid `Scroll` table view with all 64 rows while `node_rows`
    // never moved (proving the catch came strictly from `published_tables` validation!).
    assert!(rt.tick(0.016).expect("republish tick"));
    {
        let g = rt.acquire_scene().expect("acquire after republish");
        let v = g.view();
        assert_eq!(v.rows, orig_node_rows, "node_rows must not have moved");
        // SAFETY: `g` holds the reader lock for `table_count` entries.
        let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
        let scr = tables
            .iter()
            .find(|t| t.slot == Scroll::slot().0 as u32)
            .expect("Scroll table republished");
        assert_eq!(scr.row_count, 64);
        assert_ne!(scr.rows, orig_scroll_rows);
        assert_ne!(scr.owners, orig_scroll_owners);
    }
}

/// Direct app + `FrameTickHook` exercising create-in-app -> remove-in-hook within ONE `tick()`
/// AND duplicate `Visual` edits across app + hook (regression test for P1 #4 and P2 #2).
struct CreateInAppApp {
    store: NodeStore,
    changes: ChangeSet,
    persistent: NodeId,
    mounted: bool,
}

impl CreateInAppApp {
    fn new() -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let persistent = {
            let mut tx = store.tx();
            let p = tx.create();
            tx.edit::<Layout>(p)
                .width(Length::points(100.0))
                .height(Length::points(40.0));
            tx.edit::<Visual>(p).opacity(1.0);
            let transient = tx.create();
            tx.edit::<Layout>(transient)
                .width(Length::points(50.0))
                .height(Length::points(20.0));
            tx.edit::<Visual>(transient).opacity(0.9);
            tx.set_children(NodeId::ROOT, &[p, transient]);
            tx.commit_into(&mut changes);
            p
        };
        Self {
            store,
            changes,
            persistent,
            mounted: false,
        }
    }
}

impl RuntimeApp for CreateInAppApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if !self.mounted {
            self.mounted = true;
            AppFrame::Direct {
                changes: &self.changes,
            }
        } else {
            AppFrame::Clean
        }
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        false
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

struct RemoveTransientAndEditPersistentHook {
    persistent: NodeId,
}

impl FrameTickHook for RemoveTransientAndEditPersistentHook {
    fn tick(&mut self, tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        let kids: Vec<NodeId> = tx.store().children(NodeId::ROOT).to_vec();
        if kids.len() == 2 {
            let transient = kids[1];
            tx.set_children(NodeId::ROOT, &[self.persistent]);
            tx.remove(transient);
            tx.edit::<Visual>(self.persistent).opacity(0.4);
            return true;
        }
        false
    }
}

#[test]
fn app_create_then_hook_remove_cancels_cleanly_and_deduplicates_table_dirty_rows() {
    let app = CreateInAppApp::new();
    let persistent = app.persistent;
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(app))
        .tick_hook(Box::new(RemoveTransientAndEditPersistentHook { persistent }))
        .build();

    assert!(rt.tick(0.016).expect("app+hook tick"));
    let guard = rt.acquire_scene().expect("acquire scene");
    let v = guard.view();
    assert_eq!(v.removed_count, 0, "transient node born in app and removed in hook must cancel");
    // SAFETY: `guard` holds the reader lock for `dirty_count` indices.
    let dirty = unsafe { std::slice::from_raw_parts(v.dirty_indices, v.dirty_count as usize) };
    for &idx in dirty {
        // SAFETY: `idx < v.node_capacity`.
        let row = unsafe { *v.rows.add(idx as usize) };
        assert!(row.is_live(), "dead transient slot {idx} leaked into dirty_indices");
    }
    // SAFETY: `guard` holds the reader lock for `table_count` entries.
    let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
    let vis = tables
        .iter()
        .find(|t| t.slot == Visual::slot().0 as u32)
        .expect("Visual table published");
    assert_eq!(
        vis.dirty_count, 1,
        "duplicate Visual edits in app + hook must deduplicate to 1 dirty row"
    );
}

struct TwoPumpTransientApp {
    store: NodeStore,
    changes: ChangeSet,
    persistent: NodeId,
    transient_idx: usize,
    phase: u8,
}

impl TwoPumpTransientApp {
    fn new() -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let persistent = {
            let mut tx = store.tx();
            let p = tx.create();
            tx.edit::<Layout>(p)
                .width(Length::points(120.0))
                .height(Length::points(40.0));
            tx.edit::<Visual>(p).opacity(1.0);
            tx.set_children(NodeId::ROOT, &[p]);
            tx.commit_into(&mut changes);
            p
        };
        Self {
            store,
            changes,
            persistent,
            transient_idx: 0,
            phase: 0,
        }
    }
}

impl RuntimeApp for TwoPumpTransientApp {
    fn poll_async_work(&mut self) -> bool {
        if self.phase == 1 {
            self.phase = 2;
            true
        } else {
            false
        }
    }

    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        match self.phase {
            0 => {
                // Pump 1: create a transient node AND edit `persistent`'s `Visual`.
                self.phase = 1;
                let mut tx = self.store.tx();
                let t = tx.create();
                self.transient_idx = t.index();
                tx.edit::<Layout>(t)
                    .width(Length::points(40.0))
                    .height(Length::points(20.0));
                tx.edit::<Visual>(t).opacity(0.9);
                tx.edit::<Visual>(self.persistent).opacity(0.7);
                tx.set_children(NodeId::ROOT, &[self.persistent, t]);
                tx.commit_into(&mut self.changes);
                AppFrame::Direct {
                    changes: &self.changes,
                }
            }
            2 => {
                // Pump 2 in the SAME `tick()`: remove the transient node AND edit `persistent`'s
                // `Visual` a second time (producing duplicate `Visual` dirty row before dedup!).
                self.phase = 3;
                let transient = self.store.children(NodeId::ROOT)[1];
                let mut tx = self.store.tx();
                tx.set_children(NodeId::ROOT, &[self.persistent]);
                tx.remove(transient);
                tx.edit::<Visual>(self.persistent).opacity(0.3);
                tx.commit_into(&mut self.changes);
                AppFrame::Direct {
                    changes: &self.changes,
                }
            }
            _ => AppFrame::Clean,
        }
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        false
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

#[test]
fn two_pump_create_then_remove_and_duplicate_table_edits_are_clean_and_deduplicated() {
    let app = TwoPumpTransientApp::new();
    let persistent = app.persistent;
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(app))
        .tick_hook(Box::new(RemoveTransientAndEditPersistentHook { persistent }))
        .build();

    // Single `tick()` runs Pump 1 (create transient + edit Visual) AND Pump 2 (remove transient +
    // edit Visual again).
    assert!(rt.tick(0.016).expect("two-pump tick"));
    let guard = rt.acquire_scene().expect("acquire scene");
    let v = guard.view();
    assert_eq!(v.removed_count, 0, "transient node born and killed in same tick must cancel out");
    // SAFETY: `guard` holds the reader lock for `dirty_count` items.
    let dirty = unsafe { std::slice::from_raw_parts(v.dirty_indices, v.dirty_count as usize) };
    // Every dirty index must point to a LIVE row (never the dead transient slot 3!).
    for &idx in dirty {
        // SAFETY: `idx < v.node_capacity`.
        let row = unsafe { *v.rows.add(idx as usize) };
        assert!(row.is_live(), "dead transient slot {idx} leaked into dirty_indices");
    }
    // `Visual` table `dirty_rows` must be deduplicated (no duplicate row index for `persistent`)!
    // SAFETY: `guard` holds the reader lock for `table_count` entries.
    let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
    let vis = tables
        .iter()
        .find(|t| t.slot == Visual::slot().0 as u32)
        .expect("Visual table published");
    assert_eq!(vis.dirty_count, 1, "duplicate Visual edits across pumps must deduplicate to 1 dirty row");
    // SAFETY: `vis.dirty_count == 1` and `guard` holds the reader lock.
    let vis_dirty = unsafe { std::slice::from_raw_parts(vis.dirty_rows, vis.dirty_count as usize) };
    assert_eq!(vis_dirty.len(), 1);
}

struct Utf8PanickingHook;

impl FrameTickHook for Utf8PanickingHook {
    fn tick(&mut self, _tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        #[expect(
            clippy::panic,
            reason = "intentional UTF-8 panic to verify rt_last_error char-boundary truncation"
        )]
        {
            panic!("boom🔥é");
        }
    }
}

#[test]
fn rt_last_error_handles_null_cap1_utf8_truncation_and_clears_on_ok() {
    let api = get_api();
    let rt = RuntimeBuilder::new()
        .tick_hook(Box::new(Utf8PanickingHook))
        .build();
    let h = Box::into_raw(Box::new(rt)).cast::<c_void>();

    let mut changed = 0u32;
    // SAFETY: Valid handle on owner thread; triggers caught panic with multi-byte UTF-8 payload.
    let tick_rc = unsafe { (api.tick)(h, 0.016, &mut changed) };
    assert_eq!(tick_rc, NveRuntimeStatus::Panic.as_i32());

    // 1. `(NULL, 0)` returns full byte length without writing.
    // SAFETY: Null pointer with cap=0 is explicitly supported for length queries.
    let full_len = unsafe { (api.last_error)(std::ptr::null_mut(), 0) };
    assert!(full_len > 10);

    // 2. `cap == 1` writes only NUL terminator while returning full byte length.
    let mut tiny = [0xAAu8; 1];
    // SAFETY: Valid 1-byte buffer.
    let len1 = unsafe { (api.last_error)(tiny.as_mut_ptr(), tiny.len()) };
    assert_eq!(len1, full_len);
    assert_eq!(tiny[0], 0);

    // 3. Every small truncation capacity `2..=full_len` must produce valid NUL-terminated UTF-8!
    let mut buf = vec![0xFFu8; full_len + 2];
    for cap in 2..=full_len {
        buf.fill(0xFF);
        // SAFETY: Valid buffer of length `cap`.
        let reported = unsafe { (api.last_error)(buf.as_mut_ptr(), cap) };
        assert_eq!(reported, full_len);
        let nul_pos = buf[..cap]
            .iter()
            .position(|&b| b == 0)
            .expect("must NUL-terminate within cap");
        assert!(
            std::str::from_utf8(&buf[..nul_pos]).is_ok(),
            "rt_last_error split a multi-byte UTF-8 sequence at cap={cap}: {:?}",
            &buf[..nul_pos]
        );
    }

    // 4. Subsequent `Ok` C ABI call clears `last_error` back to 0 bytes.
    // SAFETY: Valid handle; `set_wake_callback(NULL)` succeeds even on poisoned handle.
    let clear_rc = unsafe { (api.set_wake_callback)(h, None, std::ptr::null_mut()) };
    assert_eq!(clear_rc, NveRuntimeStatus::Ok.as_i32());
    // SAFETY: Length query with null/0.
    let cleared_len = unsafe { (api.last_error)(std::ptr::null_mut(), 0) };
    assert_eq!(cleared_len, 0);

    // SAFETY: Destroy handle once on owner thread.
    unsafe { (api.destroy)(h) };
}
