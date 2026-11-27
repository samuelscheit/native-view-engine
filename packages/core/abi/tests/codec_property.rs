//! Deterministic seeded property & fuzz-style tests for `nve-abi` 64-byte `NveEvent`,
//! `EventBatchBuf`, `ValidatedEvents`, `NVEV` v2 wire codec, and FFI helpers.

use nve_abi::{
    EventBatchBuf, EventFlags, EventId, InteractionEvent, Key64, NodeId, NveBufferSlice,
    NveGestureDisposition, NveSizeConstraints, SurfaceId, TimestampUs, ValidatedEvents,
};

struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        assert_ne!(seed, 0);
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u8(&mut self) -> u8 {
        self.next_u64() as u8
    }

    fn next_usize(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound
    }

    fn next_f32(&mut self) -> f32 {
        let raw = (self.next_u32() % 200_000) as i32 - 100_000;
        raw as f32 / 100.0
    }

    fn next_string(&mut self, max_len: usize) -> String {
        const ALPHABET: &[char] = &[
            'a', 'Z', '0', '9', '-', '_', ' ', '/', 'é', 'ü', '中', '日', '🚀', '✨',
        ];
        let len = if max_len == 0 {
            0
        } else {
            self.next_usize(max_len + 1)
        };
        (0..len)
            .map(|_| ALPHABET[self.next_usize(ALPHABET.len())])
            .collect()
    }

    fn next_bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = if max_len == 0 {
            0
        } else {
            self.next_usize(max_len + 1)
        };
        (0..len).map(|_| self.next_u8()).collect()
    }
}

#[test]
fn event_batch_wire_and_validated_ffi_seeded_roundtrip() {
    let mut rng = XorShift64::new(0xC0DE_5EED_CAFE_BABE);
    let mut buf = EventBatchBuf::with_capacity(64, 1024);

    for i in 0..64u64 {
        let surface = SurfaceId::new((rng.next_u32() % 4) + 1, 0);
        let node = NodeId::new((rng.next_u32() % 1000) + 1, 0);
        let ts = TimestampUs(i * 16_667);
        match i % 7 {
            0 => buf.push_press(surface, node, ts, rng.next_f32(), rng.next_f32()),
            1 => buf.push_scroll(surface, node, ts, rng.next_f32(), rng.next_f32(), 1000.0, 4000.0),
            2 => buf.push_text_change(surface, node, ts, &rng.next_string(24)),
            3 => buf.push_layout(surface, node, ts, rng.next_f32(), rng.next_f32(), 200.0, 80.0),
            4 => buf.push_custom(surface, node, ts, &rng.next_string(12), &rng.next_bytes(32)),
            5 => buf.push_key_down(
                surface,
                node,
                ts,
                &rng.next_string(24),
                &rng.next_string(12),
                EventFlags::from_modifiers(i % 2 == 0, i % 3 == 0, false, true, false),
                rng.next_u32(),
            ),
            _ => buf.push_table_event(
                Key64(rng.next_u64()),
                surface,
                node,
                ts,
                EventId(rng.next_u32() % 32),
                EventFlags::EMPTY,
                [rng.next_u32(), rng.next_u32(), 0, 0, 0, 0],
                &rng.next_bytes(48),
            ),
        }
    }

    let encoded = buf.to_wire_bytes();
    let decoded = EventBatchBuf::decode_wire(&encoded).expect("decode_wire");
    assert_eq!(decoded, buf);

    // Zero-copy raw FFI ingress round-trip
    // SAFETY: `buf` stays alive for the lifetime of `view`.
    let view = unsafe { ValidatedEvents::try_from_raw(buf.as_raw_batch()) }.expect("try_from_raw");
    assert_eq!(view.len(), buf.len());
    for (a, b) in view.iter().zip(decoded.as_validated().iter()) {
        assert_eq!(a.raw(), b.raw());
        assert_eq!(a.bytes(), b.bytes());
        assert_eq!(a.text(), b.text());
        assert_eq!(a.key(), b.key());
        assert_eq!(a.code(), b.code());
        assert_eq!(a.custom_name(), b.custom_name());
        assert_eq!(a.custom_data(), b.custom_data());
    }

    // Every strict prefix of a valid wire frame MUST return `Err(...)` (never silently succeed).
    for cut in 0..encoded.len() {
        assert!(
            EventBatchBuf::decode_wire(&encoded[..cut]).is_err(),
            "strict prefix cut={cut}/{} must fail decode_wire",
            encoded.len()
        );
    }
    let mut rng2 = XorShift64::new(0xDEAD_BEEF_1234_5678);
    for _ in 0..200 {
        let junk = rng2.next_bytes(96);
        if let Ok(decoded) = EventBatchBuf::decode_wire(&junk) {
            assert_eq!(decoded.to_wire_bytes(), junk);
        }
    }

    assert_eq!(InteractionEvent::Press.event_id(), EventId(0));
}

#[test]
fn ffi_value_helpers_sanity() {
    let bytes = b"hello";
    let slice = NveBufferSlice::from_slice(bytes);
    // SAFETY: `bytes` is a static slice and `EMPTY` has length 0.
    unsafe {
        assert_eq!(slice.as_slice(), bytes);
        assert!(NveBufferSlice::EMPTY.as_slice().is_empty());
    }

    let c = NveSizeConstraints::loose(320.0, 640.0);
    assert_eq!(c.min_w, 0.0);
    assert_eq!(c.max_w, 320.0);
    assert_eq!(NveGestureDisposition::Claim as u8, 1);
    assert_eq!(NveGestureDisposition::PassThrough as u8, 2);
    assert_eq!(NveGestureDisposition::DeferToParent as u8, 3);
}
