//! Heap-allocation ceilings for host ABI codegen, counted by a thread-local `CountingAlloc`.
//!
//! The view-models borrow the `nve-abi` descriptors and `TableDesc`s directly (no owned
//! `HybridFileSpec` bridge), and every template renders into one pre-sized buffer. What remains are
//! the small per-name `String`s of the spelling helpers: codegen is a cold build step, so those
//! are not worth a zero-allocation rewrite.
//!
//! Ceilings scale with the generated output (a minimum of output bytes per allocation, ~20% under
//! the measured density: C header ~30, tables ~37, all files ~32), so the ABI growing (new slots,
//! fields, value sets) never trips them, while reintroducing an owned intermediate spec or
//! per-field validation churn (more allocations for the same output) does. Absolute counts used
//! to fail on every ABI addition. Byte totals count every buffer growth (ceiling 5x the output).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_codegen::{generate_all_abi_files, generate_c_abi_header, generate_table_files};
use nve_components::tables::CORE_TABLE_DESCS;

struct CountingAlloc;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

fn record(size: usize) {
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
    let _ = BYTES.try_with(|c| c.set(c.get() + size));
}

// SAFETY: forwards every call unchanged to `System`; the counters are `const`-initialized
// thread-locals, so recording never allocates or re-enters the allocator.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// `(allocations, bytes)` made by `f` on this thread.
fn counted<T>(f: impl FnOnce() -> T) -> (usize, usize, T) {
    let (a0, b0) = (ALLOCS.with(Cell::get), BYTES.with(Cell::get));
    let out = f();
    (ALLOCS.with(Cell::get) - a0, BYTES.with(Cell::get) - b0, out)
}

/// At least `min_bytes_per_alloc` output bytes per heap allocation.
fn assert_density(what: &str, allocs: usize, out: usize, min_bytes_per_alloc: usize) {
    assert!(
        allocs * min_bytes_per_alloc <= out,
        "{what} took {allocs} allocations for {out} output bytes \
         ({} bytes/allocation, minimum {min_bytes_per_alloc})",
        out / allocs.max(1)
    );
}

#[test]
fn c_abi_header_allocation_ceiling() {
    let _ = generate_c_abi_header(); // warm one-time statics
    let (allocs, bytes, header) = counted(generate_c_abi_header);
    let header = header.expect("C header");
    eprintln!(
        "generate_c_abi_header: {allocs} allocations, {bytes} bytes for {} output bytes",
        header.len()
    );
    assert_density("C header", allocs, header.len(), 24);
    assert!(
        bytes <= 5 * header.len(),
        "C header allocated {bytes} bytes for {} output bytes",
        header.len()
    );
}

#[test]
fn table_files_allocation_ceiling() {
    let (allocs, bytes, files) = counted(|| generate_table_files(CORE_TABLE_DESCS));
    let out: usize = files.expect("tables").iter().map(|f| f.content.len()).sum();
    eprintln!(
        "generate_table_files(17 core): {allocs} allocations, {bytes} bytes for {out} output bytes"
    );
    // Java `NveTables` names every field value (`{FIELD}_{VALUE}`, ~650 constants).
    assert_density("tables", allocs, out, 29);
    assert!(
        bytes <= 5 * out,
        "tables allocated {bytes} bytes for {out} output bytes"
    );
}

#[test]
fn every_host_file_allocation_ceiling() {
    let (allocs, bytes, files) = counted(|| generate_all_abi_files(CORE_TABLE_DESCS));
    let out: usize = files
        .expect("all files")
        .iter()
        .map(|f| f.content.len())
        .sum();
    eprintln!("generate_all_abi_files: {allocs} allocations, {bytes} bytes for {out} output bytes");
    assert_density("all files", allocs, out, 26);
    assert!(
        bytes <= 5 * out,
        "all files allocated {bytes} bytes for {out} output bytes"
    );
}
