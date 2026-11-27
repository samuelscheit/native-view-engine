//! Self-gated clean-frame skip parity: `compute_layout` returns `false` (no pass) only when
//! nothing layout-relevant changed, and every layout input reopens the pass with results equal to
//! a from-scratch layout.

use std::sync::Arc;

use super::hoisted_tests::{VP, assert_matches_layout_of};
use crate::TaffyStoreLayout;
use nve_components::tables::{
    AlignSelf, Display, FlexDirection, Layout, Overflow, Position, TextContent, Typography, Visual,
};
use crate::TextMeasurer;
use nve_engine::Viewport;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Transaction};

/// Deterministic measurer whose width depends on text, size, and weight, so every text input
/// changes the measured box (unlike weight-invariant measurers).
struct WeightedMeasurer {
    scale: f32,
}

impl TextMeasurer for WeightedMeasurer {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self.measure_with_weight(text, font_size, 400, max_width)
    }

    fn measure_with_weight(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let natural =
            text.chars().count() as f32 * font_size * 0.5 * self.scale * f32::from(font_weight)
                / 400.0;
        (
            max_width.map_or(natural, |w| natural.min(w)),
            font_size * 1.25,
        )
    }
}

struct Fixture {
    store: NodeStore,
    changes: ChangeSet,
    layout: TaffyStoreLayout,
    measurer: Arc<dyn TextMeasurer>,
    parent: NodeId,
    label: NodeId,
    boxed: NodeId,
}

impl Fixture {
    fn new() -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let (parent, label, boxed) = {
            let mut tx = store.tx();
            let [parent, label, row, boxed, inner] = [(); 5].map(|_| tx.create());
            tx.set_children(NodeId::ROOT, &[parent]);
            tx.set_children(parent, &[label, row]);
            tx.set_children(row, &[boxed]);
            tx.set_children(boxed, &[inner]);
            tx.edit::<Layout>(parent)
                .flex_direction(FlexDirection::Column);
            // `label` inherits its typography from `parent` and sizes to its text.
            tx.edit::<Typography>(parent)
                .font_size(16.0)
                .font_weight(400);
            tx.edit::<TextContent>(label).text("Hello");
            tx.edit::<Layout>(label).align_self(AlignSelf::FlexStart);
            // Attaching a row is itself layout-relevant; paint-only frames edit existing rows.
            tx.edit::<Typography>(label).color(nve_store::Rgba::BLACK);
            tx.edit::<Visual>(boxed).opacity(1.0);
            // `boxed` holds content wider than the 400pt row: its automatic minimum size keeps
            // it at content width until it becomes a scroll container.
            tx.edit::<Layout>(row).flex_direction(FlexDirection::Row);
            tx.edit::<Layout>(inner)
                .width(Length::points(600.0))
                .height(Length::points(30.0));
            tx.commit_into(&mut changes);
            (parent, label, boxed)
        };
        let measurer: Arc<dyn TextMeasurer> = Arc::new(WeightedMeasurer { scale: 1.0 });
        let mut layout = TaffyStoreLayout::new();
        layout.set_text_measurer(Arc::clone(&measurer));
        layout.sync_changes(&store, &changes);
        assert!(
            layout.compute_layout(&store, NodeId::ROOT, VP),
            "first frame lays out"
        );
        Self {
            store,
            changes,
            layout,
            measurer,
            parent,
            label,
            boxed,
        }
    }

    /// Commits `edit` and runs one frame; returns whether a layout pass ran.
    fn commit(&mut self, edit: impl FnOnce(&mut Transaction<'_>)) -> bool {
        let mut tx = self.store.tx();
        edit(&mut tx);
        tx.commit_into(&mut self.changes);
        self.layout.sync_changes(&self.store, &self.changes);
        self.layout.compute_layout(&self.store, NodeId::ROOT, VP)
    }

    /// A frame with an empty transaction.
    fn frame(&mut self) -> bool {
        self.commit(|_| {})
    }

    fn assert_matches_fresh(&self) {
        let mut fresh = TaffyStoreLayout::new();
        fresh.set_text_measurer(Arc::clone(&self.measurer));
        assert_matches_layout_of(&self.store, &self.layout, fresh);
    }

    fn rect(&self, id: NodeId) -> (f32, f32, f32, f32) {
        let r = self.layout.layout_of(id).expect("laid out");
        (r.x, r.y, r.width, r.height)
    }

    /// The reopened pass ran, matches a fresh layout, moved `watched`, and the next empty
    /// frame skips again.
    fn assert_reopened(
        &mut self,
        ran: bool,
        watched: NodeId,
        before: (f32, f32, f32, f32),
        what: &str,
    ) {
        assert!(ran, "{what} must reopen the layout pass");
        self.assert_matches_fresh();
        assert_ne!(
            self.rect(watched),
            before,
            "{what} must change the layout of {watched:?}"
        );
        let pass = self.layout.layout_gen;
        assert!(!self.frame(), "a clean frame after {what} must skip");
        assert_eq!(self.layout.layout_gen, pass);
        self.assert_matches_fresh();
    }
}

#[test]
fn clean_frames_skip_and_paint_only_edits_stay_skipped() {
    let mut f = Fixture::new();
    let pass = f.layout.layout_gen;
    assert!(!f.frame(), "an empty frame skips");
    let (boxed, label) = (f.boxed, f.label);
    assert!(!f.commit(|tx| {
        tx.edit::<Visual>(boxed).opacity(0.5);
        tx.edit::<Typography>(label).color(nve_store::Rgba::WHITE);
    }));
    assert_eq!(
        f.layout.layout_gen, pass,
        "paint-only edits never run a pass"
    );
    f.assert_matches_fresh();
    // A viewport change reopens with no store change.
    assert!(
        f.layout
            .compute_layout(&f.store, NodeId::ROOT, Viewport { width: 320.0, ..VP })
    );
    assert!(
        !f.layout
            .compute_layout(&f.store, NodeId::ROOT, Viewport { width: 320.0, ..VP })
    );
}

#[test]
fn text_content_and_inherited_typography_reopen_the_pass() {
    let mut f = Fixture::new();
    let (parent, label) = (f.parent, f.label);

    let before = f.rect(label);
    let ran = f.commit(|tx| {
        tx.edit::<TextContent>(label).text("Hello, wider world");
    });
    f.assert_reopened(ran, label, before, "TextContent::text");

    // Edited on the parent only: reaches the label through inheritance.
    let before = f.rect(label);
    let ran = f.commit(|tx| {
        tx.edit::<Typography>(parent).font_size(24.0);
    });
    f.assert_reopened(ran, label, before, "inherited Typography::font_size");

    let before = f.rect(label);
    let ran = f.commit(|tx| {
        tx.edit::<Typography>(parent).font_weight(700);
    });
    f.assert_reopened(ran, label, before, "inherited Typography::font_weight");
}

#[test]
fn layout_affecting_visual_fields_reopen_the_pass() {
    let mut f = Fixture::new();
    let boxed = f.boxed;
    assert_eq!(
        f.rect(boxed).2,
        600.0,
        "automatic minimum size holds content width"
    );

    let before = f.rect(boxed);
    let ran = f.commit(|tx| {
        tx.edit::<Visual>(boxed).border_width(7.0);
    });
    f.assert_reopened(ran, boxed, before, "Visual::border_width");
    assert_eq!(f.rect(boxed).2, 614.0);

    // Scroll containers have a zero automatic minimum size: `boxed` shrinks into the row.
    let before = f.rect(boxed);
    let ran = f.commit(|tx| {
        tx.edit::<Visual>(boxed).overflow_x(Overflow::Scroll);
    });
    f.assert_reopened(ran, boxed, before, "Visual::overflow_x");
    assert_eq!(f.rect(boxed).2, 400.0);
}

#[test]
fn measurer_swap_and_invalidate_all_reopen_the_pass() {
    let mut f = Fixture::new();
    let label = f.label;

    let before = f.rect(label);
    f.measurer = Arc::new(WeightedMeasurer { scale: 2.0 });
    f.layout.set_text_measurer(Arc::clone(&f.measurer));
    let ran = f.layout.compute_layout(&f.store, NodeId::ROOT, VP);
    f.assert_reopened(ran, label, before, "set_text_measurer");

    let pass = f.layout.layout_gen;
    f.layout.invalidate_all(&f.store);
    assert!(
        f.layout.compute_layout(&f.store, NodeId::ROOT, VP),
        "invalidate_all must reopen"
    );
    assert_ne!(f.layout.layout_gen, pass);
    f.assert_matches_fresh();
    assert!(!f.frame(), "a clean frame after invalidate_all must skip");
}

/// Panics while `armed`, like a faulting host measurer behind `ffi_catch_unwind`.
struct FaultingMeasurer {
    armed: std::sync::atomic::AtomicBool,
    /// Faults only on text containing this (`""`: every text).
    fault_on: &'static str,
}

impl TextMeasurer for FaultingMeasurer {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        assert!(
            !(self.armed.load(std::sync::atomic::Ordering::Relaxed)
                && text.contains(self.fault_on)),
            "host measurer fault"
        );
        WeightedMeasurer { scale: 1.0 }.measure(text, font_size, max_width)
    }
}

#[test]
fn a_pass_unwinding_out_of_the_measurer_stays_dirty_and_retries() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut f = Fixture::new();
    let faulting = Arc::new(FaultingMeasurer {
        armed: AtomicBool::new(true),
        fault_on: "",
    });
    f.layout.set_text_measurer(faulting.clone());
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        f.layout.compute_layout(&f.store, NodeId::ROOT, VP)
    }));
    assert!(unwound.is_err(), "the armed measurer must fault mid-pass");

    faulting.armed.store(false, Ordering::Relaxed);
    assert!(
        f.layout.compute_layout(&f.store, NodeId::ROOT, VP),
        "the frame after an unwound pass must retry, not skip"
    );
    f.measurer = Arc::new(WeightedMeasurer { scale: 1.0 });
    f.assert_matches_fresh();
    assert!(!f.frame());
}

#[test]
fn a_fault_while_measuring_a_fixed_box_never_drops_fixed_subtrees() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut f = Fixture::new();
    let label = f.label;
    let (fixed, fixed_text) = {
        let mut tx = f.store.tx();
        let [fixed, fixed_text] = [(); 2].map(|_| tx.create());
        tx.edit::<Layout>(fixed)
            .position(nve_components::tables::Position::Fixed)
            .inset_right(Length::points(8.0))
            .inset_bottom(Length::points(6.0));
        tx.edit::<TextContent>(fixed_text).text("FIXED toast");
        tx.set_children(fixed, &[fixed_text]);
        tx.set_children(label, &[fixed]);
        tx.commit_into(&mut f.changes);
        (fixed, fixed_text)
    };
    f.layout.sync_changes(&f.store, &f.changes);
    assert!(f.layout.compute_layout(&f.store, NodeId::ROOT, VP));
    assert_eq!(f.layout.hoisted.get(NodeId::ROOT.index()), &[fixed]);
    f.assert_matches_fresh();

    // Every in-flow box measures (and the root's in-flow layout is cached) before the root
    // positioning pass measures the fixed toast's text and faults.
    let faulting = Arc::new(FaultingMeasurer {
        armed: AtomicBool::new(true),
        fault_on: "FIXED",
    });
    f.layout.set_text_measurer(faulting.clone());
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        f.layout.compute_layout(&f.store, NodeId::ROOT, VP)
    }));
    assert!(
        unwound.is_err(),
        "the fixed toast's text must fault mid-pass"
    );

    faulting.armed.store(false, Ordering::Relaxed);
    assert!(f.layout.compute_layout(&f.store, NodeId::ROOT, VP));
    assert_eq!(
        f.layout.hoisted.get(NodeId::ROOT.index()),
        &[fixed],
        "the retry must re-record the root's fixed boxes"
    );
    assert!(
        f.layout.layout_of(fixed_text).is_some(),
        "fixed subtree laid out"
    );
    f.measurer = Arc::new(WeightedMeasurer { scale: 1.0 });
    f.assert_matches_fresh();
}

#[test]
fn switching_the_layout_root_round_trips_to_fresh_layouts() {
    let mut f = Fixture::new();
    let (parent, label, boxed) = (f.parent, f.label, f.boxed);
    // A `position: fixed` toast below an intermediate container (`row`): hoisted to whichever
    // node is the layout root.
    let row = f.store.parent(boxed).expect("row");
    let mut toast = NodeId::ROOT;
    assert!(f.commit(|tx| {
        // An absolute, viewport-sized `parent` is laid out under `ROOT` with exactly the inputs
        // of a root pass (known size = available space = parent size = viewport), so both share
        // its cache key: a cache entry left from the nested pass would be hit on the way back.
        tx.edit::<Layout>(parent)
            .position(Position::Absolute)
            .inset_left(Length::points(0.0))
            .inset_top(Length::points(0.0))
            .width(Length::points(VP.width))
            .height(Length::points(VP.height));
        toast = tx.create();
        tx.edit::<Layout>(toast)
            .position(Position::Fixed)
            .inset_top(Length::points(7.0))
            .width(Length::points(40.0))
            .height(Length::points(10.0));
        tx.set_children(row, &[boxed, toast]);
        // Non-default state on slots outside the nested root: `ROOT`'s own typography and a
        // `display: contents` sibling of `parent`, which a fresh engine laid out from
        // `parent` never resolves.
        tx.edit::<Typography>(NodeId::ROOT).font_size(24.0);
        let aside = tx.create();
        tx.edit::<Layout>(aside).display(Display::Contents);
        tx.set_children(NodeId::ROOT, &[parent, aside]);
    }));
    let subtree = [parent, label, row, boxed, toast];

    // Round 1 keeps every inherited value, so only the root switch itself can invalidate;
    // round 2 flips the root's `direction`, which nothing below `parent` inherits when nested.
    for round in 0..2 {
        if round == 1 {
            assert!(f.commit(|tx| {
                tx.edit::<Layout>(NodeId::ROOT)
                    .direction(nve_components::tables::Direction::Rtl);
            }));
            assert!(f.layout.resolved_rtl(label), "inherits the root's rtl");
        }
        assert_eq!(f.layout.hoisted.get(NodeId::ROOT.index()), &[parent, toast]);
        f.assert_matches_fresh();

        // Laid out from `parent`: the toast hoists to `parent` and nothing above `parent` is
        // inherited, exactly as in a fresh engine.
        assert!(f.layout.compute_layout(&f.store, parent, VP));
        let mut fresh = TaffyStoreLayout::new();
        fresh.set_text_measurer(Arc::clone(&f.measurer));
        assert!(fresh.compute_layout(&f.store, parent, VP));
        // Inherited state matches a fresh engine on every slot, including `ROOT` and the
        // slots outside the nested root.
        for idx in 0..f.store.capacity() {
            assert_eq!(
                (
                    f.layout.resolved_rtl[idx],
                    f.layout.resolved_text[idx],
                    f.layout.was_contents[idx],
                ),
                (
                    fresh.resolved_rtl[idx],
                    fresh.resolved_text[idx],
                    fresh.was_contents[idx],
                ),
                "inherited state of slot {idx}"
            );
        }
        for id in subtree {
            assert_eq!(f.layout.layout_of(id), fresh.layout_of(id), "{id:?}");
            assert_eq!(
                f.layout.hoisted.get(id.index()),
                fresh.hoisted.get(id.index()),
                "hoisted list of {id:?}"
            );
        }
        assert_eq!(f.layout.hoisted.get(parent.index()), &[toast]);

        // Back to the root: no cache, rect, hoisted, or box-children list of the nested pass
        // survives.
        assert!(f.layout.compute_layout(&f.store, NodeId::ROOT, VP));
        assert!(
            f.layout.hoisted.get(parent.index()).is_empty(),
            "round {round}"
        );
        f.assert_matches_fresh();
        assert!(!f.frame(), "settled again");
    }
}

#[test]
fn invalidating_everything_disarms_an_unwound_pass() {
    let mut f = Fixture::new();
    f.layout.pass_root = Some(f.parent);
    f.layout.invalidate_all(&f.store);
    assert_eq!(f.layout.pass_root, None);
    assert!(f.frame(), "invalidation reopens the pass");
    f.assert_matches_fresh();
}
