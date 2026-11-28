//! Global-allocator verification proving `0` steady-state heap allocations for reused
//! `ByteWriter` + `ByteReader` + `#[derive(NveCodec)]` encode/decode round-trips, thread-local
//! allocator isolation under background allocation noise, and bounded zero-byte DoS budgets.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nve_codec::{
    call_sync_decode, encode_f32_stack, try_with_stack_writer, with_stack_writer, ByteReader,
    ByteTransport, ByteWriter, CodecError, NveCodec, NveDecode, NveEncode, TransportError,
    DEFAULT_MAX_ITEMS,
};

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System` while counting allocations per thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let r = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (after.wrapping_sub(before), r)
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveCodec)]
enum Kind {
    Ping = 1,
    Pong = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, NveCodec)]
struct Header {
    kind: Kind,
    seq: u64,
    active: bool,
    scale: f32,
}

#[test]
fn steady_state_reused_codec_allocates_zero_bytes_under_background_alloc_noise() {
    // Spawn a noisy background thread that continuously allocates heap memory to prove our
    // thread-local CountingAlloc never races with parallel test threads.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = Arc::clone(&stop);
    let bg = std::thread::spawn(move || {
        let mut total = 0usize;
        while !stop_bg.load(Ordering::Relaxed) {
            let v: Vec<u8> = Vec::with_capacity(64);
            total = total.wrapping_add(v.capacity());
            std::hint::black_box(v);
        }
        total
    });

    let mut writer = ByteWriter::with_capacity(256);
    let hdr = Header {
        kind: Kind::Pong,
        seq: 42,
        active: true,
        scale: 2.0,
    };

    // Warm buffer once.
    writer.clear();
    hdr.encode(&mut writer).unwrap();
    writer.write_str("nve-steady-state").unwrap();

    let (allocs, decoded) = measure_allocs(|| {
        let mut last = hdr;
        for i in 0..128u64 {
            writer.clear();
            let cur = Header { seq: i, ..hdr };
            cur.encode(&mut writer).unwrap();
            writer.write_str("nve-steady-state").unwrap();

            let mut r = ByteReader::new(writer.as_slice());
            last = Header::decode(&mut r).unwrap();
            let label = r.read_str(64).unwrap();
            assert_eq!(label, "nve-steady-state");
            r.finish().unwrap();
        }
        last
    });

    stop.store(true, Ordering::Relaxed);
    let _ = bg.join().unwrap();

    assert_eq!(decoded.seq, 127);
    assert_eq!(
        allocs, 0,
        "steady-state ByteWriter/ByteReader roundtrip must allocate 0 bytes"
    );
}

#[test]
fn adversarial_lying_lengths_allocate_zero_bytes_and_bound_zst_amplification() {
    // 1. Lying length with MIN_ENCODED_BYTES > 0 (Vec<u32>): 0 heap allocations on rejection.
    let lying_u32 = 65_536u32.to_le_bytes();
    let (allocs_u32, res_u32) = measure_allocs(|| Vec::<u32>::decode_exact(&lying_u32));
    assert!(matches!(res_u32, Err(CodecError::Truncated { .. })));
    assert_eq!(allocs_u32, 0, "lying Vec<u32> length must allocate 0 bytes");

    // 2. Lying length when caller passes min_elem_bytes == 0: 0 heap allocations on rejection.
    let (allocs_zero_min, res_zero_min) = measure_allocs(|| {
        let mut r = ByteReader::new(&lying_u32);
        r.read_vec(DEFAULT_MAX_ITEMS, 0, ByteReader::read_u32)
    });
    assert!(matches!(res_zero_min, Err(CodecError::Truncated { .. })));
    assert_eq!(allocs_zero_min, 0, "lying read_vec(min=0) must allocate 0 bytes");

    // 3. Lying Vec<()> claiming 65_536 ZST elements in 4 bytes: rejected in O(1) with 0 allocs.
    let (allocs_zst, res_zst) = measure_allocs(|| Vec::<()>::decode_exact(&lying_u32));
    assert!(matches!(res_zst, Err(CodecError::LenOverflow { .. })));
    assert_eq!(allocs_zst, 0, "lying Vec<()> must allocate 0 bytes");

    // 4. Running-total zero_byte_budget test: 1024 inner Vec<()> each claiming 4000 items.
    // Input is 4100 bytes (budget = 4164), so inner #0 (4000 <= 4164) drains budget to 164 and
    // inner #1 (4000 > 164) MUST trigger `LenOverflow`, exercising `self.zero_byte_budget -= 1`.
    let mut amp = ByteWriter::with_capacity(4 + 1024 * 4);
    amp.write_u32(1024);
    for _ in 0..1024 {
        amp.write_u32(4_000);
    }
    let res_amp = Vec::<Vec<()>>::decode_exact(amp.as_slice());
    assert!(matches!(res_amp, Err(CodecError::LenOverflow { .. })));

    // 5. Direct read_vec(..., 0, ...) counting invocations across multiple inner vectors:
    // total zero-byte invocations must never exceed input_len + 64.
    let input = amp.as_slice();
    let mut r = ByteReader::new(input);
    let outer = r.read_u32().unwrap();
    let mut invocations = 0usize;
    let mut hit_overflow = false;
    for _ in 0..outer {
        match r.read_vec(DEFAULT_MAX_ITEMS, 0, |_| {
            invocations += 1;
            Ok(())
        }) {
            Ok(_) => {}
            Err(CodecError::LenOverflow { .. }) => {
                hit_overflow = true;
                break;
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert!(hit_overflow, "running zero_byte_budget must stop multi-vec ZST amplification");
    assert!(
        invocations <= input.len() + 64 + 1,
        "zero-byte callback invocations ({invocations}) exceeded input_len + 64 ({})",
        input.len() + 64
    );
}

#[test]
fn counting_alloc_positive_probe_and_stack_writer_byte_transport_zero_alloc() {
    // Positive control probe: prove CountingAlloc catches real heap allocations.
    let (probe_allocs, _) = measure_allocs(|| {
        let v = std::hint::black_box(vec![1u8; 64]);
        v.len()
    });
    assert!(
        probe_allocs >= 1,
        "positive control probe must observe >= 1 heap allocation"
    );

    struct InlineEcho;
    impl ByteTransport for InlineEcho {
        fn call_sync(
            &self,
            _service_id: u32,
            _method_id: u32,
            req: &[u8],
            on_reply: &mut dyn FnMut(&[u8]),
        ) -> Result<(), TransportError> {
            on_reply(req);
            Ok(())
        }

        fn call_async(
            &self,
            _service_id: u32,
            _method_id: u32,
            req: &[u8],
            on_done: nve_codec::TransportDoneFn,
        ) {
            on_done(Ok(req));
        }
    }

    let transport = InlineEcho;

    let (fast_allocs, sum) = measure_allocs(|| {
        let mut acc = 0u64;
        for i in 0..128u32 {
            let decoded: u64 = try_with_stack_writer::<512, _, TransportError>(
                |w| {
                    w.write_u32(i)?;
                    w.write_str("clipboard.read")?;
                    w.write_bytes(b"zero-alloc-payload")?;
                    w.write_bool(true)
                },
                |req| {
                    call_sync_decode(&transport, 1, 1, req, |reply| {
                        let mut r = ByteReader::new(reply);
                        let seq = r.read_u32()?;
                        let name = r.read_str(64)?;
                        let body = r.read_bytes(64)?;
                        let flag = r.read_bool()?;
                        r.finish()?;
                        assert_eq!(name, "clipboard.read");
                        assert_eq!(body, b"zero-alloc-payload");
                        assert!(flag);
                        Ok(u64::from(seq))
                    })
                },
            )
            .unwrap();
            acc += decoded;

            // Single primitive + canonical float stack round-trip.
            let pod_req = encode_f32_stack(i as f32);
            let pod_out = call_sync_decode(&transport, 1, 1, &pod_req, |reply| {
                let mut r = ByteReader::new(reply);
                let v = r.read_f32()?;
                r.finish()?;
                Ok(v)
            })
            .unwrap();
            assert_eq!(pod_out, i as f32);
        }
        acc
    });
    assert_eq!(sum, (0..128u64).sum::<u64>());
    assert_eq!(
        fast_allocs, 0,
        "with_stack_writer::<512> + ByteTransport::call_sync fast path must allocate 0 bytes"
    );

    // Overflow spills cleanly to Vec<u8> (verified by observing >= 1 allocation).
    let (spill_allocs, spilled_len) = measure_allocs(|| {
        with_stack_writer::<16, _>(
            |w| {
                w.write_str("this-payload-exceeds-sixteen-bytes-and-spills-to-heap")
            },
            |bytes| bytes.len(),
        )
        .unwrap()
    });
    assert!(spilled_len > 16);
    assert!(
        spill_allocs >= 1,
        "overflowing StackWriter must spill to heap Vec<u8>"
    );
}
