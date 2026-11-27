use nve_abi::{
    EventBatchBuf, EventSet, NodeId, SurfaceId, TextInputEvent, TimestampUs, ValidatedEvents,
};

#[test]
fn composition_events_roundtrip_wire_and_direct_ffi_batch() {
    assert_eq!(TextInputEvent::CompositionStart as u8, 2);
    assert_eq!(TextInputEvent::CompositionUpdate as u8, 3);
    assert_eq!(TextInputEvent::CompositionEnd as u8, 4);
    let comp_set: EventSet<TextInputEvent> = TextInputEvent::CompositionStart
        | TextInputEvent::CompositionUpdate
        | TextInputEvent::CompositionEnd;
    assert_eq!(comp_set.len(), 3);

    let mut batch_buf = EventBatchBuf::new();
    batch_buf.push_composition(
        SurfaceId::ROOT,
        NodeId::new(10, 0),
        TimestampUs(100),
        TextInputEvent::CompositionStart,
        "ni",
    );
    batch_buf.push_composition(
        SurfaceId::ROOT,
        NodeId::new(10, 0),
        TimestampUs(200),
        TextInputEvent::CompositionUpdate,
        "nihao",
    );
    batch_buf.push_composition(
        SurfaceId::ROOT,
        NodeId::new(10, 0),
        TimestampUs(300),
        TextInputEvent::CompositionEnd,
        "你好",
    );

    let encoded = batch_buf.to_wire_bytes();
    let decoded = EventBatchBuf::decode_wire(&encoded).expect("decode composition events");
    assert_eq!(decoded, batch_buf);

    let raw = batch_buf.as_raw_batch();
    // SAFETY: `batch_buf` stays alive for the lifetime of `view`.
    let view = unsafe { ValidatedEvents::try_from_raw(raw) }.expect("validate raw batch");
    assert_eq!(view.len(), 3);
    assert_eq!(
        view.get(0).unwrap().text_input(),
        Some(TextInputEvent::CompositionStart)
    );
    assert_eq!(view.get(0).unwrap().text(), "ni");
    assert_eq!(
        view.get(1).unwrap().text_input(),
        Some(TextInputEvent::CompositionUpdate)
    );
    assert_eq!(view.get(1).unwrap().text(), "nihao");
    assert_eq!(
        view.get(2).unwrap().text_input(),
        Some(TextInputEvent::CompositionEnd)
    );
    assert_eq!(view.get(2).unwrap().text(), "你好");
}
