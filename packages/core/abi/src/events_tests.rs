use super::*;

#[test]
fn roundtrip_event_ring() {
    let events = vec![
        NativeEvent {
            surface_id: 1,
            node_id: 10,
            timestamp_us: 123_456,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 42.5, y: 18.0 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 11,
            timestamp_us: 123_500,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("hello".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: 12,
            timestamp_us: 123_900,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "onSvgLoad".into(),
                data: vec![9, 8, 7],
            },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 13,
            timestamp_us: 124_000,
            kind: NativeEventKind::CompositionStart,
            payload: EventPayload::Text("ni".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: 13,
            timestamp_us: 124_100,
            kind: NativeEventKind::CompositionUpdate,
            payload: EventPayload::Text("nihao".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: 13,
            timestamp_us: 124_200,
            kind: NativeEventKind::CompositionEnd,
            payload: EventPayload::Text("你好".into()),
        },
    ];
    assert_eq!(EventMask::COMPOSITION, 1 << 8);
    // Every kind maps to exactly one bit; only the composition kinds share one; `VALUES` names
    // every bit once and fits the `u32` `nve.interaction.event_mask` word.
    let mut seen = std::collections::BTreeMap::new();
    for raw in 1u8..=255 {
        let Some(kind) = NativeEventKind::from_u8_opt(raw) else { continue };
        let bit = EventMask::of(kind);
        assert!(bit.is_power_of_two() && bit <= u64::from(u32::MAX), "{kind:?}");
        seen.entry(bit).or_insert_with(Vec::new).push(kind);
    }
    assert_eq!(seen.len(), EventMask::VALUES.values.len());
    for (bit, kinds) in &seen {
        assert_eq!(kinds.len() == 3, *bit == EventMask::COMPOSITION, "{kinds:?}");
        assert!(EventMask::VALUES.values.iter().any(|&(_, v)| u64::from(v) == *bit));
    }
    assert_eq!(EventMask::of(NativeEventKind::KeyDown), 1 << 9);
    let encoded = encode_events(&events);
    let decoded = decode_events(&encoded).unwrap();
    assert_eq!(events, decoded);
}

#[test]
fn direct_typed_key_pointer_wheel_roundtrip_zero_size_growth() {
    let events = vec![
        NativeEvent {
            surface_id: 1,
            node_id: 20,
            timestamp_us: 200_000,
            kind: NativeEventKind::KeyDown,
            payload: EventPayload::Key {
                key: "k".into(),
                code: "KeyK".into(),
                modifiers: pack_key_modifiers(true, false, true, false, false),
                key_code: 75,
                is_composing: false,
            },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 21,
            timestamp_us: 200_100,
            kind: NativeEventKind::PointerMove,
            payload: EventPayload::Pointer {
                x: 120.5,
                y: 44.25,
                modifiers: NVE_KEY_MOD_SHIFT,
                flags: 1,
            },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 22,
            timestamp_us: 200_200,
            kind: NativeEventKind::Wheel,
            payload: EventPayload::Wheel {
                delta_x: -4.0,
                delta_y: 24.0,
                modifiers: NVE_KEY_MOD_CTRL,
            },
        },
    ];
    assert_eq!(EventMask::KEY_DOWN, 1 << 9);
    assert_eq!(EventMask::WHEEL, 1 << 17);
    let encoded = encode_events(&events);
    let decoded = decode_events(&encoded).unwrap();
    assert_eq!(events, decoded);

    let mut direct = Vec::new();
    native_events_to_direct(&events, &mut direct);
    let roundtripped: Vec<NativeEvent> = direct
        .iter()
        .map(|d| {
            // SAFETY: `direct` borrows `events` which remain alive across this map.
            let raw = unsafe { NveDirectEventRef::from_raw(d) };
            direct_event_to_native(raw).unwrap()
        })
        .collect();
    assert_eq!(events, roundtripped);
}
