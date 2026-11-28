//! Global-allocator verification proving `0` steady-state heap allocations for reused
//! `ByteWriter` + `ByteReader` + `#[derive(NveCodec)]` encode/decode round-trips, thread-local
//! allocator isolation under background allocation noise, and bounded zero-byte DoS budgets.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nve_codec::{ByteReader, ByteWriter, CodecError, NveCodec, NveDecode, NveEncode, DEFAULT_MAX_ITEMS};

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
