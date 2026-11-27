//! Allocation guard for the hybrid runtime: buffers handed to a null out-pointer are released,
//! never leaked; empty or impossible buffers and carrier decoding allocate nothing; and a hostile
//! declared length cannot make a decoder reserve memory in bulk.
#![cfg(feature = "wire")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_abi::BinWriter;
use nve_abi::hybrid::{
    NveHybridOwnedBytes, NveHybridStatus, decode_array, decode_bin_arg, decode_bool, fail,
    nve_hybrid_alloc_bytes, nve_hybrid_free_bytes, store_bytes,
};

/// Tracks this thread's successful allocations, live bytes, and largest single request, so
/// parallel tests never add noise.
struct Tracking;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

fn bytes(size: usize) -> isize {
    isize::try_from(size).unwrap_or(isize::MAX)
}

/// Records an allocation of `size` bytes once `System` has granted it.
fn granted(ptr: *mut u8, size: usize, freed: usize) -> *mut u8 {
    if !ptr.is_null() {
        let _ = LIVE.try_with(|n| n.set(n.get() + bytes(size) - bytes(freed)));
        let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
        let _ = LARGEST.try_with(|n| n.set(n.get().max(size)));
    }
    ptr
}

// SAFETY: Delegates verbatim to `System`, tracking only what it granted.
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        granted(unsafe { System.alloc(layout) }, layout.size(), 0)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Caller upholds `GlobalAlloc::alloc_zeroed` preconditions; delegated to `System`.
        granted(unsafe { System.alloc_zeroed(layout) }, layout.size(), 0)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) };
        let _ = LIVE.try_with(|n| n.set(n.get() - bytes(layout.size())));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        let grown = unsafe { System.realloc(ptr, layout, new_size) };
        granted(grown, new_size, layout.size())
    }
}

#[global_allocator]
static GLOBAL: Tracking = Tracking;

/// What `f` did on this thread: (live bytes left behind, allocations, largest allocation).
fn measure<R>(f: impl FnOnce() -> R) -> (isize, usize, usize, R) {
    let (live, allocs) = (LIVE.with(Cell::get), ALLOCS.with(Cell::get));
    LARGEST.with(|n| n.set(0));
    let out = f();
    (
        LIVE.with(Cell::get) - live,
        ALLOCS.with(Cell::get) - allocs,
        LARGEST.with(Cell::get),
        out,
    )
}

#[test]
fn null_out_pointers_release_owned_buffers() {
    let (leaked, ..) = measure(|| {
        // SAFETY: A null out-pointer is allowed; the message is dropped, not handed out.
        let status = unsafe {
            fail(
                std::ptr::null_mut(),
                NveHybridStatus::InvalidArg,
                "x".repeat(4096),
            )
        };
        assert_eq!(status, NveHybridStatus::InvalidArg as i32);
    });
    assert_eq!(leaked, 0);
    let (leaked, ..) = measure(|| {
        // SAFETY: A null out-pointer is allowed; the buffer comes from `from_vec`.
        unsafe {
            store_bytes(
                std::ptr::null_mut(),
                NveHybridOwnedBytes::from_vec(vec![7; 4096]),
            )
        };
    });
    assert_eq!(leaked, 0);
    let (leaked, ..) = measure(|| {
        let buf = NveHybridOwnedBytes::from_vec(Vec::with_capacity(256));
        assert!(buf.ptr.is_null(), "an empty vector crosses as `EMPTY`");
    });
    assert_eq!(leaked, 0, "its capacity is released, not handed out");
}

#[test]
fn empty_impossible_and_carrier_work_allocates_nothing() {
    let (leaked, allocs, _, ()) = measure(|| {
        for buf in [
            nve_hybrid_alloc_bytes(0),
            nve_hybrid_alloc_bytes(usize::MAX),
        ] {
            assert!(buf.ptr.is_null());
            // SAFETY: `EMPTY` is always releasable.
            unsafe { nve_hybrid_free_bytes(buf) };
        }
    });
    assert_eq!((leaked, allocs), (0, 0));

    let mut grid = [[1u8; 33]; 2];
    grid[0][7] = 0;
    let (_, allocs, _, decoded) =
        measure(|| decode_array(grid, |row| decode_array(row, decode_bool)));
    assert_eq!(allocs, 0);
    assert_eq!(decoded.map(|g| g[0][7]), Ok(false));

    let (leaked, allocs, _, ()) = measure(|| {
        let buf = nve_hybrid_alloc_bytes(1024);
        // SAFETY: Allocated by `nve_hybrid_alloc_bytes`, released once.
        unsafe { nve_hybrid_free_bytes(buf) };
    });
    assert_eq!((leaked, allocs), (0, 1));
}

#[test]
fn a_hostile_declared_length_cannot_reserve_in_bulk() {
    // 50 000 one-byte `Null`s: a 250 KB input claiming 50 000 elements of 4 KiB (200 MB).
    let mut w = BinWriter::new();
    w.array(|a| (0..50_000).for_each(|_| a.item().null()));
    let hostile = w.finish().expect("encodes");

    let (_, _, largest, decoded) = measure(|| decode_bin_arg::<Vec<[u8; 4096]>>(&hostile));
    assert!(decoded.is_err());
    assert!(largest <= 64 * 1024, "reserved {largest} bytes up front");

    let (_, _, largest, decoded) = measure(|| decode_bin_arg::<Vec<Vec<String>>>(&hostile));
    assert!(decoded.is_err());
    assert!(largest <= 64 * 1024, "reserved {largest} bytes up front");
}
