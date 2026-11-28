//! Zero-allocation heap-guard suite for `nve-wire-core` hot paths.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::hint::black_box;
use std::sync::Mutex;

use nve_abi::{DurationMs, NodeId};
use nve_wire_core::{
    constant_time_eq, format_cert_hash_hex, parse_cert_hash_hex, NveNodePatch,
    NveNodePatchDeltaBuilder, NveNodePatchDeltaView, NveWireChannel, NveWireError, NveWireFrame,
    NveWireFrameRef, NveWireHeader, NveWireHelloAck, NveWireKind, NveWireSlotChunk,
    DEFAULT_MAX_WIRE_FRAME_BYTES, NVE_WIRE_FLAG_XXH64, NVE_WIRE_HEADER_SIZE,
};

struct CountingAlloc;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
}

#[inline]
fn thread_alloc_count() -> usize {
    ALLOC_COUNT.with(Cell::get)
}

#[inline]
fn measure_allocs<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = thread_alloc_count();
    let out = f();
    let after = thread_alloc_count();
    (out, after.wrapping_sub(before))
}

// SAFETY: Every allocation method forwards unchanged arguments to `System`, upholding `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn counting_alloc_positive_probe() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (nonzero, allocs) = measure_allocs(|| black_box(vec![1u8; 64]));
    drop(nonzero);
    assert!(
        allocs >= 1,
        "positive probe failed: Vec allocation must increment CountingAlloc (got {allocs})"
    );

    let (zeroed, zeroed_allocs) = measure_allocs(|| black_box(vec![0u8; 64]));
    drop(zeroed);
    assert!(
        zeroed_allocs >= 1,
        "positive probe failed: zeroed Vec allocation must increment CountingAlloc (got {zeroed_allocs})"
    );
}

#[test]
fn wire_header_and_frame_encode_decode_are_zero_alloc_in_steady_state() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let header = NveWireHeader {
        channel: NveWireChannel::UiScene,
        kind: NveWireKind::CommandDelta,
        flags: NVE_WIRE_FLAG_XXH64,
        sequence: 99,
        payload_len: 128,
    };
    let frame_checked = NveWireFrame::new(NveWireKind::CommandDelta, 42, vec![0x5Au8; 128]);
    let frame_unchecked =
        NveWireFrame::new(NveWireKind::Presence, 43, vec![0xA5u8; 64]).with_xxh64(false);

    let mut scratch = Vec::with_capacity(256);
    frame_checked.encode_into(&mut scratch).unwrap();
    let mut corrupted = scratch.clone();
    corrupted[NVE_WIRE_HEADER_SIZE + 5] ^= 0x01;

    let ((hdr_rt, dec_checked_len, dec_unchecked_len, bad_ck), allocs) = measure_allocs(|| {
        let raw_hdr = black_box(&header).to_bytes();
        let hdr_rt = NveWireHeader::decode(black_box(&raw_hdr), DEFAULT_MAX_WIRE_FRAME_BYTES)
            .expect("valid header");

        black_box(&frame_checked)
            .encode_into(&mut scratch)
            .expect("encode_into checked");
        let dec_checked = NveWireFrameRef::decode(black_box(&scratch), DEFAULT_MAX_WIRE_FRAME_BYTES)
            .expect("decode checked");
        let dec_checked_len = dec_checked.payload.len();

        black_box(&frame_unchecked)
            .encode_into(&mut scratch)
            .expect("encode_into unchecked");
        let dec_unchecked =
            NveWireFrameRef::decode(black_box(&scratch), DEFAULT_MAX_WIRE_FRAME_BYTES)
                .expect("decode unchecked");
        let dec_unchecked_len = dec_unchecked.payload.len();

        let bad_ck = matches!(
            NveWireFrameRef::decode(black_box(&corrupted), DEFAULT_MAX_WIRE_FRAME_BYTES),
            Err(NveWireError::ChecksumMismatch { .. })
        );

        (hdr_rt, dec_checked_len, dec_unchecked_len, bad_ck)
    });

    assert_eq!(
        allocs, 0,
        "NveWireHeader / NveWireFrame::encode_into / NveWireFrameRef::decode must perform 0 heap allocs, did {allocs}"
    );
    assert_eq!(hdr_rt, header);
    assert_eq!(dec_checked_len, 128);
    assert_eq!(dec_unchecked_len, 64);
    assert!(bad_ck);
}

#[test]
fn cert_hash_parse_constant_time_eq_and_handshake_views_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let expected_hash = [0xABu8; 32];
    let plain_hex = format_cert_hash_hex(&expected_hash);
    let colon_hex =
        "AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB";
    let ack = NveWireHelloAck {
        session_id: 77,
        schema_version: 2,
        server_capabilities: 0x0F,
    };
    let slot_bytes = NveWireSlotChunk {
        slot_id: 3,
        chunk_index: 1,
        total_chunks: 4,
        data: b"chunk-payload-bytes",
    }
    .encode();

    let ((h1, h2, bad_len, eq_ok, eq_neq, eq_len, ack_rt, slot_len), allocs) = measure_allocs(|| {
        let h1 = parse_cert_hash_hex(black_box(&plain_hex)).expect("plain hex");
        let h2 = parse_cert_hash_hex(black_box(colon_hex)).expect("colon hex");
        let bad_len = parse_cert_hash_hex(black_box("abcd")).is_err();

        let eq_ok = constant_time_eq(black_box(b"nve_secret_token_123"), black_box(b"nve_secret_token_123"));
        let eq_neq = constant_time_eq(black_box(b"nve_secret_token_123"), black_box(b"nve_secret_token_124"));
        let eq_len = constant_time_eq(black_box(b"short"), black_box(b"longer"));

        let ack_bytes = black_box(&ack).encode();
        let ack_rt = NveWireHelloAck::decode(black_box(&ack_bytes)).expect("decode ack");
        let slot_rt = NveWireSlotChunk::decode(black_box(&slot_bytes)).expect("decode slot chunk");

        (h1, h2, bad_len, eq_ok, eq_neq, eq_len, ack_rt, slot_rt.data.len())
    });

    assert_eq!(
        allocs, 0,
        "parse_cert_hash_hex / constant_time_eq / HelloAck / SlotChunk decode must perform 0 heap allocs, did {allocs}"
    );
    assert_eq!(h1, expected_hash);
    assert_eq!(h2, expected_hash);
    assert!(bad_len);
    assert!(eq_ok);
    assert!(!eq_neq);
    assert!(!eq_len);
    assert_eq!(ack_rt, ack);
    assert_eq!(slot_len, b"chunk-payload-bytes".len());
}

#[test]
fn node_patch_delta_encode_into_and_decode_into_are_zero_alloc_in_steady_state() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut builder = NveNodePatchDeltaBuilder::with_capacity(16, 128);
    builder
        .set_text(NodeId::new(101, 0), "LAL 108")
        .set_text(NodeId::new(102, 0), "BOS 104")
        .set_image_src(NodeId::new(103, 0), "https://cdn.example/badge.png")
        .set_bg_rgba(NodeId::new(104, 0), 0x16A3_4AFF)
        .set_opacity(NodeId::new(105, 0), 0.95, DurationMs(180))
        .set_rect_width(NodeId::new(106, 0), 284.5, DurationMs(240));

    let expected_bytes = builder.encode().unwrap();
    let expected_view = NveNodePatchDeltaView::decode(&expected_bytes).unwrap();

    let mut wire_buf = Vec::with_capacity(expected_bytes.len());
    let mut patch_buf: Vec<NveNodePatch> = Vec::with_capacity(16);

    // Warm both buffers once.
    builder.encode_into(&mut wire_buf).unwrap();
    let _ = NveNodePatchDeltaView::decode_into(&wire_buf, &mut patch_buf).unwrap();

    let ((count, first_text_len, third_src_len), allocs) = measure_allocs(|| {
        black_box(&builder)
            .encode_into(&mut wire_buf)
            .expect("warm encode_into");
        let string_table =
            NveNodePatchDeltaView::decode_into(black_box(&wire_buf), &mut patch_buf)
                .expect("warm decode_into");
        let s0 = NveNodePatchDeltaView::resolve_patch_str(string_table, &patch_buf[0])
            .expect("resolve text");
        let s2 = NveNodePatchDeltaView::resolve_patch_str(string_table, &patch_buf[2])
            .expect("resolve image_src");
        (patch_buf.len(), s0.len(), s2.len())
    });

    assert_eq!(
        allocs, 0,
        "Warm NveNodePatchDeltaBuilder::encode_into and NveNodePatchDeltaView::decode_into must perform 0 heap allocs, did {allocs}"
    );
    assert_eq!(wire_buf, expected_bytes);
    assert_eq!(patch_buf.as_slice(), expected_view.patches());
    assert_eq!(count, 6);
    assert_eq!(first_text_len, "LAL 108".len());
    assert_eq!(third_src_len, "https://cdn.example/badge.png".len());
}
