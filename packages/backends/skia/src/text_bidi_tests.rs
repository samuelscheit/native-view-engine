//! BiDi text shaping, grapheme-safe cursor hit-testing, and IME cursor rect tests over `NodeStore`.

use nve_components::tables::{Layout, TextInput, Typography};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

use crate::SkiaBackend;

#[test]
fn text_input_bidi_caret_selection_and_ime_cursor_rect() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 320.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );

    let mut tx = store.tx();
    let input = tx.create();
    tx.set_children(NodeId::ROOT, &[input]);
    tx.edit::<Layout>(input)
        .width(Length::points(260.0))
        .height(Length::points(36.0))
        .padding_left(Length::points(8.0));
    tx.edit::<TextInput>(input).value("Hello مرحبا 🌍");
    tx.edit::<Typography>(input).font_size(16.0);
    tx.commit_into(&mut changes);

    backend.commit(&store, &changes).unwrap();
    backend.set_focused_node(&store, Some(input.raw()));
    assert_eq!(backend.focused_node(), Some(input.raw()));
    assert!(backend.is_text_input(&store, input));

    let caret_start = backend.cursor_index_at_x(&store, input, 0.0).unwrap();
    assert_eq!(caret_start, 0);

    let val = backend.text_input_value(&store, input).unwrap();
    let rects = backend.selection_rects_for_range(&store, input, 0, val.len());
    assert!(!rects.is_empty());

    backend.set_ime_preedit(&store, Some("かな".into()), Some((0, 6)));
    let ime_rect = backend.ime_cursor_rect(&store).unwrap();
    assert!(ime_rect.width >= 1.0 && ime_rect.height > 0.0);
}
