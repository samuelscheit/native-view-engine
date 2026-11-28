use core::mem::{align_of, offset_of, size_of};

use crate::event::{
    EventBatchBuf, EventSet, EventValidationError, InteractionEvent, LayoutEvent, NveEvent,
    NveEventBatch, RawEventBytes, ScrollEvent, TextInputEvent, ValidatedEvents, NVE_EVENT_ALIGN,
    NVE_EVENT_INLINE_KEY_CAP, NVE_EVENT_SIZE, NVE_TABLE_CUSTOM, NVE_TABLE_INTERACTION,
    NVE_TABLE_LAYOUT, NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};
use crate::ffi::DecodeError;
use crate::id::{ArenaOffset, ByteLen, EventFlags, EventId, NodeId, SurfaceId, TimestampUs};
use crate::Key64;

#[test]
fn nve_event_and_batch_layout_contracts() {
    assert_eq!(size_of::<RawEventBytes>(), 8);
    assert_eq!(align_of::<RawEventBytes>(), 4);
    assert_eq!(size_of::<NveEvent>(), NVE_EVENT_SIZE);
    assert_eq!(align_of::<NveEvent>(), NVE_EVENT_ALIGN);
    assert_eq!(offset_of!(NveEvent, timestamp), 0);
    assert_eq!(offset_of!(NveEvent, table), 8);
    assert_eq!(offset_of!(NveEvent, surface), 16);
    assert_eq!(offset_of!(NveEvent, node), 20);
    assert_eq!(offset_of!(NveEvent, event), 24);
    assert_eq!(offset_of!(NveEvent, flags), 28);
    assert_eq!(offset_of!(NveEvent, payload), 32);
    assert_eq!(offset_of!(NveEvent, bytes), 56);
    assert_eq!(NVE_EVENT_INLINE_KEY_CAP, 16);
}

#[test]
fn event_enum_and_event_set_operations_and_bitor() {
    assert_eq!(InteractionEvent::COUNT, 12);
    assert_eq!(TextInputEvent::COUNT, 5);
    assert_eq!(ScrollEvent::COUNT, 1);
    assert_eq!(LayoutEvent::COUNT, 1);

    let set = InteractionEvent::Press | InteractionEvent::KeyDown | InteractionEvent::Wheel;
    assert_eq!(set.len(), 3);
    assert!(set.contains(InteractionEvent::Press));
    assert!(set.contains(InteractionEvent::KeyDown));
    assert!(set.contains(InteractionEvent::Wheel));
    assert!(!set.contains(InteractionEvent::Focus));
    let collected: Vec<InteractionEvent> = set.iter().collect();
    assert_eq!(
        collected,
        vec![
            InteractionEvent::Press,
            InteractionEvent::KeyDown,
            InteractionEvent::Wheel
        ]
    );
    // Out-of-range bits are masked out by `from_raw`.
    let clamped = EventSet::<ScrollEvent>::from_raw(u32::MAX);
    assert_eq!(clamped.raw(), 1);
    assert_eq!(clamped.iter().collect::<Vec<_>>(), vec![ScrollEvent::Scroll]);
    assert_eq!(
        InteractionEvent::from_event_id(EventId(InteractionEvent::COUNT as u32)),
        None
    );
}

#[test]
fn event_batch_buf_builders_inline_and_long_keys_custom_and_wire_roundtrip() {
    let mut buf = EventBatchBuf::new();
    let s = SurfaceId::ROOT;
    let n = NodeId::new(7, 2);

    buf.push_press(s, n, TimestampUs(10), 12.5, 24.5);
    buf.push_scroll(s, n, TimestampUs(20), 1.0, 2.0, 300.0, 900.0);
    buf.push_text_change(s, n, TimestampUs(30), "hello 你好");
    buf.push_submit(s, n, TimestampUs(40), "submitted");
    buf.push_composition(s, n, TimestampUs(50), TextInputEvent::CompositionUpdate, "nihao");
    buf.push_layout(s, n, TimestampUs(60), 4.0, 8.0, 120.0, 48.0);
    // Short key (`<= 16 B`) uses inline UTF-8 fast path!
    buf.push_key_down(
        s,
        n,
        TimestampUs(70),
        "Enter",
        "Enter",
        EventFlags::from_modifiers(true, false, true, false, false).union(EventFlags::COMPOSING),
        13,
    );
    // Exact 16-byte UTF-8 key also fits inline.
    let key16 = "0123456789abcdef";
    buf.push_key_up(s, n, TimestampUs(75), key16, "KeyA", EventFlags::EMPTY, 65);
    // Long key (`> 16 B`) automatically spills into KEY_SPLIT_BYTES arena.
    let long_key = "DeadCombinedVeryLongKeyName🚀";
    buf.push_key_down(
        s,
        n,
        TimestampUs(80),
        long_key,
        "Unidentified",
        EventFlags::MOD_CTRL,
        229,
    );
    buf.push_pointer(
        s,
        n,
        TimestampUs(90),
        InteractionEvent::PointerDown,
        11.0,
        22.0,
        3,
        EventFlags::EMPTY
            .with_pointer_button(2)
            .union(EventFlags::CAPTURE),
    );
    buf.push_wheel(s, n, TimestampUs(100), -3.0, 12.0, EventFlags::MOD_ALT);
    buf.push_custom(s, n, TimestampUs(110), "analytics.click", &[1, 2, 3, 4]);
    buf.push_custom_with_flags(
        s,
        n,
        TimestampUs(115),
        "close",
        &[],
        EventFlags::EMPTY
            .with_bubbles(false)
            .union(EventFlags::UTF8_BYTES), // internal framing bit must be stripped
    );
    let mod_key = Key64::of("acme.map_view");
    buf.push_table_event(
        mod_key,
        s,
        n,
        TimestampUs(120),
        EventId(3),
        EventFlags::UTF8_BYTES, // internal framing bit must be stripped for raw bytes!
        [42, 0, 0, 0, 0, 0],
        &[0xFF, 0xFE],
    );

    let view = buf.as_validated();
    assert_eq!(view.len(), 14);

    let e0 = view.get(0).unwrap();
    assert_eq!(e0.interaction(), Some(InteractionEvent::Press));
    assert_eq!((e0.x(), e0.y()), (12.5, 24.5));

    let e1 = view.get(1).unwrap();
    assert_eq!(e1.scroll_event(), Some(ScrollEvent::Scroll));
    assert_eq!((e1.x(), e1.y(), e1.width(), e1.height()), (1.0, 2.0, 300.0, 900.0));

    let e2 = view.get(2).unwrap();
    assert_eq!(e2.text_input(), Some(TextInputEvent::Change));
    assert_eq!(e2.text(), "hello 你好");

    let e6 = view.get(6).unwrap();
    assert_eq!(e6.interaction(), Some(InteractionEvent::KeyDown));
    assert!(e6.flags().contains(EventFlags::INLINE_KEY_UTF8));
    assert_eq!(e6.key(), "Enter");
    assert_eq!(e6.code(), "Enter");
    assert!(e6.meta() && e6.shift() && !e6.ctrl() && e6.is_composing());
    assert_eq!(e6.key_code(), 13);

    let e7 = view.get(7).unwrap();
    assert!(e7.flags().contains(EventFlags::INLINE_KEY_UTF8));
    assert_eq!(e7.key(), key16);
    assert_eq!(e7.code(), "KeyA");

    let e8 = view.get(8).unwrap();
    assert!(e8.flags().contains(EventFlags::KEY_SPLIT_BYTES));
    assert_eq!(e8.key(), long_key);
    assert_eq!(e8.code(), "Unidentified");
    assert!(e8.ctrl());

    let e9 = view.get(9).unwrap();
    assert_eq!(e9.interaction(), Some(InteractionEvent::PointerDown));
    assert_eq!(e9.pointer_id(), 3);
    assert_eq!(e9.pointer_button(), 2);
    assert!(e9.is_capture());

    let e11 = view.get(11).unwrap();
    assert!(e11.is_custom());
    assert!(e11.bubbles());
    assert_eq!(e11.custom_name(), "analytics.click");
    assert_eq!(e11.custom_data(), &[1, 2, 3, 4]);

    let e12 = view.get(12).unwrap();
    assert!(e12.is_custom());
    assert!(!e12.bubbles());
    assert_eq!(
        e12.flags(),
        EventFlags::NO_BUBBLE.union(EventFlags::CUSTOM_NAMED)
    );
    assert_eq!(e12.custom_name(), "close");
    assert!(e12.custom_data().is_empty());

    let e13 = view.get(13).unwrap();
    assert_eq!(e13.table(), mod_key);
    assert_eq!(e13.event(), EventId(3));
    assert_eq!(e13.bytes(), &[0xFF, 0xFE]);
    assert_eq!(e13.text(), ""); // invalid UTF-8 never exposed as &str!

    // Wire roundtrip
    let wire = buf.to_wire_bytes();
    let decoded = EventBatchBuf::decode_wire(&wire).expect("decode_wire");
    assert_eq!(decoded, buf);
    assert_eq!(decoded.as_validated().get(8).unwrap().key(), long_key);
    assert!(decoded.as_validated().get(11).unwrap().bubbles());
    assert!(!decoded.as_validated().get(12).unwrap().bubbles());

    // Truncation / trailing / bad magic / bad version rejections
    assert_eq!(EventBatchBuf::decode_wire(&wire[..10]), Err(DecodeError::Truncated));
    let mut bad_magic = wire.clone();
    bad_magic[0] ^= 0xFF;
    assert!(matches!(
        EventBatchBuf::decode_wire(&bad_magic),
        Err(DecodeError::InvalidMagic(_))
    ));
    let mut bad_ver = wire.clone();
    bad_ver[4] = 99;
    assert_eq!(
        EventBatchBuf::decode_wire(&bad_ver),
        Err(DecodeError::UnsupportedVersion(99))
    );
    let mut trailing = wire.clone();
    trailing.push(0);
    assert_eq!(
        EventBatchBuf::decode_wire(&trailing),
        Err(DecodeError::TrailingBytes(1))
    );
}

#[test]
fn validated_events_rejects_adversarial_inputs_and_accepts_foreign_alignment() {
    // 1. Null pointers with non-zero counts
    let bad_ev_ptr = NveEventBatch {
        events: core::ptr::null(),
        bytes: core::ptr::null(),
        event_count: 1,
        bytes_len: ByteLen::ZERO,
    };
    // SAFETY: Adversarial null pointer test; rejected before dereference.
    let err = unsafe { ValidatedEvents::try_from_raw(bad_ev_ptr) }.unwrap_err();
    assert_eq!(err, EventValidationError::NullEventsPointer);

    let bad_bytes_ptr = NveEventBatch {
        events: core::ptr::null(),
        bytes: core::ptr::null(),
        event_count: 0,
        bytes_len: ByteLen(4),
    };
    // SAFETY: Adversarial null pointer test; rejected before dereference.
    let err = unsafe { ValidatedEvents::try_from_raw(bad_bytes_ptr) }.unwrap_err();
    assert_eq!(err, EventValidationError::NullBytesPointer);

    // 2. Span out of bounds & overflow
    let oob_ev = NveEvent::new(
        NVE_TABLE_TEXT_INPUT,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        TextInputEvent::Change.event_id(),
    )
    .with_flags(EventFlags::UTF8_BYTES)
    .with_bytes(RawEventBytes::new(ArenaOffset(u32::MAX), ByteLen(2)));
    assert!(matches!(
        ValidatedEvents::validate(&[oob_ev], b"abc"),
        Err(EventValidationError::SpanOutOfBounds { index: 0, .. })
    ));

    // 3. Invalid UTF-8 under UTF8_BYTES
    let bad_utf8 = NveEvent::new(
        NVE_TABLE_TEXT_INPUT,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        TextInputEvent::Change.event_id(),
    )
    .with_flags(EventFlags::UTF8_BYTES)
    .with_bytes(RawEventBytes::new(ArenaOffset(0), ByteLen(2)));
    assert_eq!(
        ValidatedEvents::validate(&[bad_utf8], &[0xFF, 0xFE]).unwrap_err(),
        EventValidationError::InvalidUtf8 { index: 0 }
    );

    // 4. Inline key len > 16 and invalid inline UTF-8
    let bad_inline_len = NveEvent::new(
        NVE_TABLE_INTERACTION,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        InteractionEvent::KeyDown.event_id(),
    )
    .with_flags(EventFlags::INLINE_KEY_UTF8)
    .with_payload([0, 17, 0, 0, 0, 0]);
    assert_eq!(
        ValidatedEvents::validate(&[bad_inline_len], &[]).unwrap_err(),
        EventValidationError::InvalidInlineKeyLen { index: 0, len: 17 }
    );

    let bad_inline_utf8 = NveEvent::new(
        NVE_TABLE_INTERACTION,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        InteractionEvent::KeyDown.event_id(),
    )
    .with_flags(EventFlags::INLINE_KEY_UTF8)
    .with_payload([0, 2, 0x0000_FEFF, 0, 0, 0]);
    assert_eq!(
        ValidatedEvents::validate(&[bad_inline_utf8], &[]).unwrap_err(),
        EventValidationError::InvalidUtf8 { index: 0 }
    );

    // 5. Conflicting framing flags
    let conflicting = NveEvent::new(
        NVE_TABLE_INTERACTION,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        InteractionEvent::KeyDown.event_id(),
    )
    .with_flags(EventFlags::INLINE_KEY_UTF8.union(EventFlags::KEY_SPLIT_BYTES));
    assert_eq!(
        ValidatedEvents::validate(&[conflicting], &[]).unwrap_err(),
        EventValidationError::ConflictingFlags { index: 0 }
    );

    // 6. Foreign 8-byte-aligned (non-64-byte-aligned) buffer works soundly & zero-alloc!
    let mut buf = EventBatchBuf::new();
    buf.push_key_down(
        SurfaceId::ROOT,
        NodeId::new(3, 1),
        TimestampUs(42),
        "Escape",
        "Escape",
        EventFlags::MOD_META,
        27,
    );
    let mut unaligned_backing = [0u8; NVE_EVENT_SIZE + 8];
    let start_off = if (unaligned_backing.as_ptr() as usize).is_multiple_of(64) {
        8
    } else {
        0
    };
    unaligned_backing[start_off..start_off + NVE_EVENT_SIZE]
        .copy_from_slice(buf.events()[0].as_bytes());
    let foreign_ptr = unaligned_backing[start_off..].as_ptr().cast::<NveEvent>();
    assert!(!(foreign_ptr as usize).is_multiple_of(64));
    let raw_batch = NveEventBatch {
        events: foreign_ptr,
        bytes: buf.bytes().as_ptr(),
        event_count: 1,
        bytes_len: ByteLen(buf.bytes().len() as u32),
    };
    // SAFETY: `unaligned_backing` and `buf` outlive `validated`.
    let validated = unsafe { ValidatedEvents::try_from_raw(raw_batch) }.expect("foreign batch");
    assert!(validated.as_aligned_slice().is_none());
    let ev = validated.get(0).unwrap();
    assert_eq!(ev.interaction(), Some(InteractionEvent::KeyDown));
    assert_eq!(ev.key(), "Escape");
    assert_eq!(ev.code(), "Escape");
    assert!(ev.meta());

    // 7. Un-canonicalized index-0 C POD handle bits (`0x0500_0000`) are canonicalized inside `get()`.
    let mut raw_null = NveEvent::new(
        NVE_TABLE_INTERACTION,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(77),
        InteractionEvent::Focus.event_id(),
    );
    let mut raw_bytes = *raw_null.as_bytes();
    // Overwrite `surface` (offset 16..20) and `node` (offset 20..24) with non-canonical index-0 bits.
    raw_bytes[16..20].copy_from_slice(&0x0500_0000u32.to_le_bytes());
    raw_bytes[20..24].copy_from_slice(&0x0500_0000u32.to_le_bytes());
    // SAFETY: `NveEvent` is `#[repr(C)]`, 64 bytes, valid for all bit patterns.
    raw_null = unsafe { core::ptr::read_unaligned(raw_bytes.as_ptr().cast::<NveEvent>()) };
    let raw_null_arr = [raw_null];
    let v_null = ValidatedEvents::validate(&raw_null_arr, &[]).expect("validate null bits");
    let g_null = v_null.get(0).unwrap();
    assert_eq!(g_null.surface(), SurfaceId::NULL);
    assert_eq!(g_null.node(), NodeId::NULL);
    assert_eq!(g_null.raw().surface, SurfaceId::NULL);
    assert_eq!(g_null.raw().node, NodeId::NULL);

    // 8. Wire-level `decode_wire` hostile vectors preserve distinct `DecodeError` variants.
    let build_wire = |ev: NveEvent, arena: &[u8]| -> Vec<u8> {
        let mut out = Vec::with_capacity(14 + NVE_EVENT_SIZE + arena.len());
        out.extend_from_slice(b"NVEV");
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(arena.len() as u32).to_le_bytes());
        out.extend_from_slice(ev.as_bytes());
        out.extend_from_slice(arena);
        out
    };
    assert!(matches!(
        EventBatchBuf::decode_wire(&build_wire(oob_ev, b"abc")),
        Err(DecodeError::InvalidEvent(EventValidationError::SpanOutOfBounds { index: 0, .. }))
    ));
    assert_eq!(
        EventBatchBuf::decode_wire(&build_wire(bad_inline_len, &[])),
        Err(DecodeError::InvalidEvent(EventValidationError::InvalidInlineKeyLen {
            index: 0,
            len: 17,
        }))
    );
    assert_eq!(
        EventBatchBuf::decode_wire(&build_wire(conflicting, &[])),
        Err(DecodeError::InvalidEvent(EventValidationError::ConflictingFlags { index: 0 }))
    );
    // Split key cutting inside a multi-byte UTF-8 character (`🚀` = 4 bytes, split at byte 2).
    let split_mid_char = NveEvent::new(
        NVE_TABLE_INTERACTION,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        InteractionEvent::KeyDown.event_id(),
    )
    .with_flags(EventFlags::KEY_SPLIT_BYTES.union(EventFlags::UTF8_BYTES))
    .with_payload([0, 2, 0, 0, 0, 0])
    .with_bytes(RawEventBytes::new(ArenaOffset(0), ByteLen(4)));
    assert_eq!(
        EventBatchBuf::decode_wire(&build_wire(split_mid_char, "🚀".as_bytes())),
        Err(DecodeError::InvalidUtf8)
    );
    // Custom name length exceeding event bytes slice.
    let bad_custom_name = NveEvent::new(
        NVE_TABLE_CUSTOM,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        EventId(0),
    )
    .with_flags(EventFlags::CUSTOM_NAMED)
    .with_payload([10, 0, 0, 0, 0, 0])
    .with_bytes(RawEventBytes::new(ArenaOffset(0), ByteLen(2)));
    assert_eq!(
        EventBatchBuf::decode_wire(&build_wire(bad_custom_name, b"ab")),
        Err(DecodeError::InvalidEvent(EventValidationError::InvalidCustomNameLen {
            index: 0,
            name_len: 10,
            bytes_len: 2,
        }))
    );
    // Split key length exceeding event bytes slice.
    let bad_split_len = NveEvent::new(
        NVE_TABLE_INTERACTION,
        SurfaceId::ROOT,
        NodeId::ROOT,
        TimestampUs(1),
        InteractionEvent::KeyDown.event_id(),
    )
    .with_flags(EventFlags::KEY_SPLIT_BYTES)
    .with_payload([0, 9, 0, 0, 0, 0])
    .with_bytes(RawEventBytes::new(ArenaOffset(0), ByteLen(2)));
    assert_eq!(
        EventBatchBuf::decode_wire(&build_wire(bad_split_len, b"ab")),
        Err(DecodeError::InvalidEvent(EventValidationError::InvalidSplitKeyLen {
            index: 0,
            key_len: 9,
            bytes_len: 2,
        }))
    );

    // Ensure canonical table keys are non-zero and distinct.
    let keys = [
        NVE_TABLE_INTERACTION,
        NVE_TABLE_TEXT_INPUT,
        NVE_TABLE_SCROLL,
        NVE_TABLE_LAYOUT,
        NVE_TABLE_CUSTOM,
    ];
    for (i, &a) in keys.iter().enumerate() {
        assert_ne!(a.0, 0);
        for &b in &keys[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn validated_event_text_excludes_inline_key_split_key_and_custom_utf8() {
    let mut buf = EventBatchBuf::new();
    let s = SurfaceId::ROOT;
    let n = NodeId::new(4, 1);

    // 1. Inline key event with non-empty physical `code` ("Enter" / "KeyA") carries UTF8_BYTES
    //    on `bytes` for `code()`; `text()` MUST return "" (never leak physical key code).
    buf.push_key_down(s, n, TimestampUs(1), "a", "KeyA", EventFlags::EMPTY, 65);
    buf.push_key_up(s, n, TimestampUs(2), "Enter", "Enter", EventFlags::EMPTY, 13);

    // 2. Split key event (`> 16 B`) carries `[key, code]` in `bytes`; `text()` MUST return "".
    let long_key = "DeadCombinedVeryLongKeyName🚀";
    buf.push_key_down(s, n, TimestampUs(3), long_key, "IntlBackslash", EventFlags::EMPTY, 229);

    // 3. Custom named event explicitly flagged with `CUSTOM_NAMED | UTF8_BYTES`: `bytes` holds
    //    `name + payload`; `text()` MUST return "" (never leak `name + payload`).
    let custom_utf8 = NveEvent::new(
        NVE_TABLE_CUSTOM,
        s,
        n,
        TimestampUs(4),
        EventId(0),
    )
    .with_flags(EventFlags::CUSTOM_NAMED.union(EventFlags::UTF8_BYTES))
    .with_payload([5, 0, 0, 0, 0, 0]);
    buf.try_push_raw(custom_utf8, b"eventpayload").expect("valid custom utf8 event");

    // 4. Plain text-input / submit / composition events MUST still return their UTF-8 `text()`.
    buf.push_text_change(s, n, TimestampUs(5), "typed text");
    buf.push_submit(s, n, TimestampUs(6), "submitted text");
    buf.push_composition(s, n, TimestampUs(7), TextInputEvent::CompositionEnd, "漢字");

    let view = buf.as_validated();
    let k_a = view.get(0).unwrap();
    assert_eq!(k_a.key(), "a");
    assert_eq!(k_a.code(), "KeyA");
    assert_eq!(k_a.text(), "", "inline key event must not leak physical key code via text()");

    let k_enter = view.get(1).unwrap();
    assert_eq!(k_enter.key(), "Enter");
    assert_eq!(k_enter.code(), "Enter");
    assert_eq!(k_enter.text(), "", "inline Enter key must not leak code via text()");

    let k_split = view.get(2).unwrap();
    assert_eq!(k_split.key(), long_key);
    assert_eq!(k_split.code(), "IntlBackslash");
    assert_eq!(k_split.text(), "", "split key event must not leak key+code via text()");

    let c_ev = view.get(3).unwrap();
    assert!(c_ev.is_custom());
    assert_eq!(c_ev.custom_name(), "event");
    assert_eq!(c_ev.custom_data(), b"payload");
    assert_eq!(c_ev.text(), "", "CUSTOM_NAMED | UTF8_BYTES must not leak name+payload via text()");

    assert_eq!(view.get(4).unwrap().text(), "typed text");
    assert_eq!(view.get(5).unwrap().text(), "submitted text");
    assert_eq!(view.get(6).unwrap().text(), "漢字");
}

#[test]
fn empty_event_batch_slices_emit_null_pointers() {
    // 1. Both slices empty (`NveEventBatch::from_parts`, `ValidatedEvents::EMPTY`, `EventBatchBuf::new`).
    let empty_parts = NveEventBatch::from_parts(&[], &[]);
    assert!(empty_parts.events.is_null());
    assert!(empty_parts.bytes.is_null());
    assert_eq!(empty_parts.event_count, 0);
    assert_eq!(empty_parts.bytes_len, ByteLen::ZERO);

    let empty_val = ValidatedEvents::EMPTY.as_raw_batch();
    assert!(empty_val.events.is_null());
    assert!(empty_val.bytes.is_null());
    assert_eq!(empty_val.event_count, 0);
    assert_eq!(empty_val.bytes_len, ByteLen::ZERO);

    let empty_buf = EventBatchBuf::new();
    let empty_buf_raw = empty_buf.as_raw_batch();
    assert!(empty_buf_raw.events.is_null());
    assert!(empty_buf_raw.bytes.is_null());
    let empty_buf_val_raw = empty_buf.as_validated().as_raw_batch();
    assert!(empty_buf_val_raw.events.is_null());
    assert!(empty_buf_val_raw.bytes.is_null());

    // 2. Non-empty events with empty bytes -> `events` non-null, `bytes` NULL.
    let mut press_only = EventBatchBuf::new();
    press_only.push_press(SurfaceId::ROOT, NodeId::ROOT, TimestampUs(1), 1.0, 2.0);
    let po_raw = press_only.as_raw_batch();
    assert!(!po_raw.events.is_null());
    assert!(po_raw.bytes.is_null());
    assert_eq!(po_raw.event_count, 1);
    assert_eq!(po_raw.bytes_len, ByteLen::ZERO);
    let po_val_raw = press_only.as_validated().as_raw_batch();
    assert!(!po_val_raw.events.is_null());
    assert!(po_val_raw.bytes.is_null());

    // 3. Empty events with non-empty bytes -> `events` NULL, `bytes` non-null.
    let bytes_only = NveEventBatch::from_parts(&[], b"abc");
    assert!(bytes_only.events.is_null());
    assert!(!bytes_only.bytes.is_null());
    assert_eq!(bytes_only.event_count, 0);
    assert_eq!(bytes_only.bytes_len, ByteLen(3));
    let val_bytes_only = ValidatedEvents::validate(&[], b"abc").unwrap().as_raw_batch();
    assert!(val_bytes_only.events.is_null());
    assert!(!val_bytes_only.bytes.is_null());
}
