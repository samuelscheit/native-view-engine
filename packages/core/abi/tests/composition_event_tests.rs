#![cfg(feature = "wire")]

use nve_abi::{
    decode_events, encode_events, EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId,
    NveDirectEvent, NveEventBatch, NveEventBatchRef, NveEventPayloadTag, SurfaceId,
};

#[test]
fn composition_events_roundtrip_event_ring_and_direct_ffi_batch() {
    assert_eq!(NativeEventKind::CompositionStart as u8, 9);
    assert_eq!(NativeEventKind::CompositionUpdate as u8, 10);
    assert_eq!(NativeEventKind::CompositionEnd as u8, 11);
    assert_eq!(EventMask::COMPOSITION, 1 << 8);

    let events = vec![
        NativeEvent {
            surface_id: SurfaceId(1),
            node_id: NodeId(10),
            timestamp_us: 100,
            kind: NativeEventKind::CompositionStart,
            payload: EventPayload::Text("ni".into()),
        },
        NativeEvent {
            surface_id: SurfaceId(1),
            node_id: NodeId(10),
            timestamp_us: 200,
            kind: NativeEventKind::CompositionUpdate,
            payload: EventPayload::Text("nihao".into()),
        },
        NativeEvent {
            surface_id: SurfaceId(1),
            node_id: NodeId(10),
            timestamp_us: 300,
            kind: NativeEventKind::CompositionEnd,
            payload: EventPayload::Text("你好".into()),
        },
    ];
    let encoded = encode_events(&events);
    let decoded = decode_events(&encoded).expect("decode composition events");
    assert_eq!(decoded, events);

    let direct = [
        NveDirectEvent::composition(
            SurfaceId(1),
            NodeId(10),
            100,
            NativeEventKind::CompositionStart,
            "ni",
        ),
        NveDirectEvent::composition(
            SurfaceId(1),
            NodeId(10),
            200,
            NativeEventKind::CompositionUpdate,
            "nihao",
        ),
        NveDirectEvent::composition(
            SurfaceId(1),
            NodeId(10),
            300,
            NativeEventKind::CompositionEnd,
            "你好",
        ),
    ];
    let batch = NveEventBatch::from_slice(&direct);
    // SAFETY: `direct` stays alive for the lifetime of `view`.
    let view = unsafe { NveEventBatchRef::from_raw(batch) };
    assert_eq!(view.len(), 3);
    assert_eq!(view.get(0).unwrap().kind(), Some(NativeEventKind::CompositionStart));
    assert_eq!(view.get(0).unwrap().payload_tag(), NveEventPayloadTag::Text as u8);
    assert_eq!(view.get(0).unwrap().text(), "ni");
    assert_eq!(view.get(1).unwrap().kind(), Some(NativeEventKind::CompositionUpdate));
    assert_eq!(view.get(1).unwrap().text(), "nihao");
    assert_eq!(view.get(2).unwrap().kind(), Some(NativeEventKind::CompositionEnd));
    assert_eq!(view.get(2).unwrap().text(), "你好");
}
