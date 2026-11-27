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
