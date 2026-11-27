//! Global-allocator verification proving `0` steady-state heap allocations for reused
//! `ByteWriter` + `ByteReader` + `#[derive(NveCodec)]` encode/decode round-trips.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_codec::{ByteReader, ByteWriter, NveCodec, NveDecode, NveEncode};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Delegates directly to `std::alloc::System` while counting allocations.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    let r = f();
    let after = ALLOC_COUNT.load(Ordering::SeqCst);
    (after - before, r)
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
fn steady_state_reused_codec_allocates_zero_bytes() {
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
        for i in 0..64u64 {
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

    assert_eq!(decoded.seq, 63);
    assert_eq!(allocs, 0, "steady-state ByteWriter/ByteReader roundtrip must allocate 0 bytes");
}

#[test]
fn adversarial_lying_lengths_allocate_zero_bytes_and_bound_zst_amplification() {
    // 1. Lying length with MIN_ENCODED_BYTES > 0 (Vec<u32>): 0 heap allocations on rejection.
    let lying_u32 = 65_536u32.to_le_bytes();
    let (allocs_u32, res_u32) = measure_allocs(|| Vec::<u32>::decode_exact(&lying_u32));
    assert!(matches!(res_u32, Err(nve_codec::CodecError::Truncated { .. })));
    assert_eq!(allocs_u32, 0, "lying Vec<u32> length must allocate 0 bytes");

    // 2. Lying length when caller passes min_elem_bytes == 0: 0 heap allocations on rejection.
    let (allocs_zero_min, res_zero_min) = measure_allocs(|| {
        let mut r = ByteReader::new(&lying_u32);
        r.read_vec(nve_codec::DEFAULT_MAX_ITEMS, 0, ByteReader::read_u32)
    });
    assert!(matches!(res_zero_min, Err(nve_codec::CodecError::Truncated { .. })));
    assert_eq!(allocs_zero_min, 0, "lying read_vec(min=0) must allocate 0 bytes");

    // 3. Lying Vec<()> claiming 65_536 ZST elements in 4 bytes: rejected in O(1) with 0 allocs.
    let (allocs_zst, res_zst) = measure_allocs(|| Vec::<()>::decode_exact(&lying_u32));
    assert!(matches!(res_zst, Err(nve_codec::CodecError::LenOverflow { .. })));
    assert_eq!(allocs_zst, 0, "lying Vec<()> must allocate 0 bytes");

    // 4. Nested Vec<Vec<()>> CPU-amplification payload (1024 inner vecs each claiming 65_535 items):
    // must be rejected cleanly in O(input_len) rather than amplifying to 67M+ iterations.
    let mut amp = ByteWriter::with_capacity(4 + 1024 * 4);
    amp.write_u32(1024);
    for _ in 0..1024 {
        amp.write_u32(65_535);
    }
    let res_amp = Vec::<Vec<()>>::decode_exact(amp.as_slice());
    assert!(matches!(res_amp, Err(nve_codec::CodecError::LenOverflow { .. })));
}
