//! Inherited typography re-measurement parity: every measure-affecting `Typography` field edited
//! on an ancestor re-measures the inheriting text leaf, and a paint-only `color` edit never runs a
//! pass — for Taffy always and for real RN Yoga when compiled in.

use std::sync::{Arc, Mutex, MutexGuard};

use nve_tables::tables::{
    FontStyle, OverflowWrap, TextContent, Typography, WhiteSpace, WordBreak,
};
use nve_engine::Viewport;
use nve_layout::{StoreLayoutEngine, TaffyStoreLayout, TextMeasurer, TextStyle};
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, Transaction};

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
    scale_factor: 1.0,
};

/// Records the style of every measurement.
#[derive(Default)]
struct Recorder(Mutex<Vec<TextStyle<String>>>);

impl Recorder {
    fn seen(&self) -> MutexGuard<'_, Vec<TextStyle<String>>> {
        self.0.lock().expect("recorder lock")
    }
}

impl TextMeasurer for Recorder {
    fn measure(&self, text: &str, style: &TextStyle<&str>, _max_width: Option<f32>) -> (f32, f32) {
        self.seen().push(style.map_family(|f| (*f).to_owned()));
        (text.len() as f32 * 8.0, 20.0)
    }
}

type Edit = fn(&mut Transaction<'_>, NodeId);
type Check = fn(&TextStyle<String>) -> bool;

/// `(what, edit, check)`: each edit on the parent and what the leaf's next measurement must see.
fn edits() -> [(&'static str, Edit, Check); 9] {
    [
        (
            "line_height",
            |tx, p| {
                tx.edit::<Typography>(p).line_height(Length::points(40.0));
            },
            |s| s.line_height == Length::points(40.0),
        ),
        (
            "letter_spacing",
            |tx, p| {
                tx.edit::<Typography>(p).letter_spacing(3.0);
            },
            |s| s.letter_spacing == 3.0,
        ),
        (
            "white_space",
            |tx, p| {
                tx.edit::<Typography>(p).white_space(WhiteSpace::PreWrap);
            },
            |s| s.white_space == WhiteSpace::PreWrap,
        ),
        (
            "max_lines",
            |tx, p| {
                tx.edit::<Typography>(p).max_lines(core::num::NonZeroU16::new(3));
            },
            |s| s.max_lines == 3,
        ),
        (
            "font_style",
            |tx, p| {
                tx.edit::<Typography>(p).font_style(FontStyle::Italic);
            },
            |s| s.font_style == FontStyle::Italic,
        ),
        (
            "word_break",
            |tx, p| {
                tx.edit::<Typography>(p).word_break(WordBreak::BreakAll);
            },
            |s| s.word_break == WordBreak::BreakAll,
        ),
        (
            "overflow_wrap",
            |tx, p| {
                tx.edit::<Typography>(p)
                    .overflow_wrap(OverflowWrap::Anywhere);
            },
            |s| s.overflow_wrap == OverflowWrap::Anywhere,
        ),
        (
            "font_family",
            |tx, p| {
                tx.edit::<Typography>(p).font_family("Arial");
            },
            |s| s.font_family == "Arial",
        ),
        // Same length: the string arena replaces it in place under the same span.
        (
            "same-length font_family",
            |tx, p| {
                tx.edit::<Typography>(p).font_family("Inter");
            },
            |s| s.font_family == "Inter",
        ),
    ]
}

fn run_engine(name: &str, mut engine: impl StoreLayoutEngine) {
    let recorder = Arc::new(Recorder::default());
    engine.set_text_measurer(recorder.clone());
    let mut store = NodeStore::new();
    let (parent, label) = {
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        let [parent, label] = [(); 2].map(|_| tx.create());
        tx.set_children(NodeId::ROOT, &[parent]);
        tx.set_children(parent, &[label]);
        tx.edit::<Typography>(parent).font_size(16.0);
        tx.edit::<TextContent>(label).text("Hello");
        tx.commit_into(&mut changes);
        engine.sync_changes(&store, &changes);
        (parent, label)
    };
    assert!(
        engine.compute_layout(&store, NodeId::ROOT, VP),
        "{name}: first frame"
    );
    let mut frame = |store: &mut NodeStore, edit: &dyn Fn(&mut Transaction<'_>)| {
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        edit(&mut tx);
        tx.commit_into(&mut changes);
        engine.sync_changes(store, &changes);
        recorder.seen().clear();
        engine.compute_layout(store, NodeId::ROOT, VP)
    };
    for (what, edit, check) in edits() {
        assert!(
            frame(&mut store, &|tx| edit(tx, parent)),
            "{name}: {what} reopens the pass"
        );
        let seen = recorder.seen();
        assert!(!seen.is_empty(), "{name}: {what} re-measures {label:?}");
        assert!(
            seen.iter().all(check),
            "{name}: {what} reaches the measurer: {seen:?}"
        );
    }
    assert!(
        !frame(&mut store, &|tx| {
            tx.edit::<Typography>(parent).color(Rgba::WHITE);
        }),
        "{name}: an inherited color edit never relayouts"
    );
    assert!(
        recorder.seen().is_empty(),
        "{name}: color never re-measures"
    );
}

#[test]
fn inherited_measure_fields_remeasure_and_color_never_relayouts() {
    run_engine("taffy", TaffyStoreLayout::new());
    if YogaLayoutEngine::is_available() {
        run_engine("yoga", YogaLayoutEngine::new().expect("real Yoga"));
    }
}
