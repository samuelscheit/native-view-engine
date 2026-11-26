use std::cell::RefCell;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, EventMask, EventPayload, NativeEvent, NativeEventKind,
    NveDirectEvent, NveEventBatch, NveEventBatchRef,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;

thread_local! {
    static COMP_LOG: RefCell<Vec<(&'static str, String)>> = const { RefCell::new(Vec::new()) };
}

fn comp_app() -> Element {
    rsx! {
        textinput {
            oncompositionstart: move |e: CompositionEvent| {
                COMP_LOG.with(|l| l.borrow_mut().push(("start", e.data().text.clone())));
            },
            oncompositionupdate: move |e: CompositionEvent| {
                COMP_LOG.with(|l| l.borrow_mut().push(("update", e.data().text.clone())));
            },
            oncompositionend: move |e: CompositionEvent| {
                COMP_LOG.with(|l| l.borrow_mut().push(("end", e.data().text.clone())));
            },
        }
    }
}

#[test]
fn composition_events_set_mask_and_dispatch_via_event_ring_and_direct_ffi() {
    COMP_LOG.with(|l| l.borrow_mut().clear());

    let mut surface = DioxusSurface::new(1, comp_app);
    let (_, _, frame) = surface.render_frame(1);
    let (_, cmds) = CommandDecoder::new(&frame).unwrap().decode_all().unwrap();
    let input_id = cmds
        .iter()
        .find_map(|c| match c {
            Command::CreateNode {
                node_id,
                kind: ComponentKind::TextInput,
                ..
            } => Some(*node_id),
            _ => None,
        })
        .expect("TextInput node");
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::SetEventMask { node_id, mask }
            if *node_id == input_id && (*mask & EventMask::COMPOSITION) != 0
    )));

    assert!(surface.dispatch_events(&[
        NativeEvent {
            surface_id: 1,
            node_id: input_id,
            timestamp_us: 1,
            kind: NativeEventKind::CompositionStart,
            payload: EventPayload::Text("ni".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: input_id,
            timestamp_us: 2,
            kind: NativeEventKind::CompositionUpdate,
            payload: EventPayload::Text("nihao".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: input_id,
            timestamp_us: 3,
            kind: NativeEventKind::CompositionEnd,
            payload: EventPayload::Text("你好".into()),
        },
    ]));

    let direct = [NveDirectEvent::composition(
        1,
        input_id,
        4,
        NativeEventKind::CompositionEnd,
        "完",
    )];
    let batch = NveEventBatch::from_slice(&direct);
    assert!(surface.dispatch_direct_events(unsafe { NveEventBatchRef::from_raw(batch) }));

    COMP_LOG.with(|l| {
        assert_eq!(
            *l.borrow(),
            vec![
                ("start", "ni".into()),
                ("update", "nihao".into()),
                ("end", "你好".into()),
                ("end", "完".into()),
            ]
        );
    });
}
