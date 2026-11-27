//! Deterministic seeded property & fuzz-style tests for `nve-abi` `EventRing` and FFI helpers.
//!
//! Uses a zero-dependency `XorShift64` PRNG so every failure is 100% reproducible.

#![cfg(feature = "wire")]

use nve_abi::{
    decode_events, direct_event_to_native, encode_events, native_events_to_direct, EventPayload,
    NativeEvent, NativeEventKind, NodeId, NveBufferSlice, NveDirectEventRef, NveGestureDisposition,
    NveSizeConstraints, SurfaceId,
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
fn event_ring_and_direct_event_seeded_roundtrip_including_table_events() {
    let mut rng = XorShift64::new(0xC0DE_5EED_CAFE_BABE);
    let mut events = Vec::with_capacity(64);

    for i in 0..64u64 {
        let surface_id = SurfaceId::new((rng.next_u32() % 4) + 1, 0);
        let node_id = NodeId::new((rng.next_u32() % 1000) + 1, 0);
        let timestamp_us = i * 16_667;
        let ev = match i % 6 {
            0 => NativeEvent {
                surface_id,
                node_id,
                timestamp_us,
                kind: NativeEventKind::Press,
                payload: EventPayload::Empty,
            },
            1 => NativeEvent {
                surface_id,
                node_id,
                timestamp_us,
                kind: NativeEventKind::Scroll,
                payload: EventPayload::Scroll {
                    x: rng.next_f32(),
                    y: rng.next_f32(),
                    content_width: 1000.0,
                    content_height: 4000.0,
                },
            },
            2 => NativeEvent {
                surface_id,
                node_id,
                timestamp_us,
                kind: NativeEventKind::TextChange,
                payload: EventPayload::Text(rng.next_string(24)),
            },
            3 => NativeEvent {
                surface_id,
                node_id,
                timestamp_us,
                kind: NativeEventKind::Layout,
                payload: EventPayload::Layout {
                    x: rng.next_f32(),
                    y: rng.next_f32(),
                    width: 200.0,
                    height: 80.0,
                },
            },
            4 => NativeEvent {
                surface_id,
                node_id,
                timestamp_us,
                kind: NativeEventKind::Custom,
                payload: EventPayload::Custom {
                    name: rng.next_string(12),
                    data: rng.next_bytes(32),
                },
            },
            _ => NativeEvent {
                surface_id,
                node_id,
                timestamp_us,
                kind: NativeEventKind::TableEvent,
                payload: EventPayload::TableEvent {
                    table_key: rng.next_u64(),
                    event_id: rng.next_u32(),
                    payload: rng.next_bytes(48),
                },
            },
        };
        events.push(ev);
    }

    let encoded = encode_events(&events);
    let decoded = decode_events(&encoded).expect("decode_events");
    assert_eq!(decoded, events);

    // Zero-copy NveDirectEvent round-trip
    let mut direct = Vec::new();
    native_events_to_direct(&events, &mut direct);
    assert_eq!(direct.len(), events.len());
    for (d, expected) in direct.iter().zip(&events) {
        // SAFETY: `direct` borrows `events` which remain alive across this loop.
        let r = unsafe { NveDirectEventRef::from_raw(d) };
        assert_eq!(direct_event_to_native(r).as_ref(), Some(expected));
    }

    // Corrupt truncation fuzzing must never panic.
    for cut in 0..encoded.len().min(64) {
        let _ = decode_events(&encoded[..cut]);
    }
    let mut rng2 = XorShift64::new(0xDEAD_BEEF_1234_5678);
    for _ in 0..200 {
        let junk = rng2.next_bytes(64);
        let _ = decode_events(&junk);
    }
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
