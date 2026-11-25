//! Cross-format parity integration tests proving `serde_json`, `NVE_SNAP`, `rkyv`,
//! `musli-zerocopy`, `NVERELOC` (`RelocSlice<T>`), and `MmapOffsetArena` all agree 100%
//! field-for-field and checksum-for-checksum on identical Studio Workspace datasets.

#[path = "../benches/arena_and_mmap/crate_codecs.rs"]
mod crate_codecs;

use crate_codecs::{
    open_musli_mmap_full_verify, open_musli_mmap_lazy, open_rkyv_mmap_unchecked,
    open_rkyv_mmap_validated, random_read_musli_mmap, random_read_rkyv_mmap,
    random_read_serde_tasks, read_serde_json, to_rkyv_workspace, to_serde_tasks,
    write_musli_workspace, write_rkyv_workspace, write_serde_json,
};
use nve_bench::arena_mmap::baseline_codec::{read_tasks_nve_snap, write_tasks_nve_snap};
use nve_bench::arena_mmap::reloc::{write_reloc_workspace, MmapRelocWorkspace, VerifyMode};
use nve_bench::arena_mmap::workspace_arena::{
    populate_workspace_arena, random_read_workspace_arena,
};
use nve_bench::arena_mmap::{
    deterministic_read_indices, generate_workspace_tasks, random_read_owned_tasks, TempFileGuard,
};

#[test]
fn all_six_formats_produce_identical_checksums_and_expected_sizes() {
    let tasks = generate_workspace_tasks(1_000);
    let indices = deterministic_read_indices(tasks.len(), 1_000);
    let expected = random_read_owned_tasks(&tasks, &indices);

    // 1. serde_json
    let json_guard = TempFileGuard::new("parity_serde", "json");
    let serde_in = to_serde_tasks(&tasks);
    let json_bytes = write_serde_json(json_guard.path(), &serde_in);
    let serde_out = read_serde_json(json_guard.path());
    assert_eq!(serde_out.len(), tasks.len());
    assert_eq!(random_read_serde_tasks(&serde_out, &indices), expected);

    // 2. NVE_SNAP owned binary
    let snap_guard = TempFileGuard::new("parity_snap", "nvesnap");
    let snap_bytes = write_tasks_nve_snap(snap_guard.path(), &tasks).expect("write nve_snap");
    let snap_out = read_tasks_nve_snap(snap_guard.path()).expect("read nve_snap");
    assert_eq!(snap_out, tasks);
    assert_eq!(random_read_owned_tasks(&snap_out, &indices), expected);

    // 3. rkyv 0.8 (validated + unchecked)
    let rkyv_guard = TempFileGuard::new("parity_rkyv", "rkyv");
    let rkyv_ws = to_rkyv_workspace(&tasks);
    let rkyv_bytes = write_rkyv_workspace(rkyv_guard.path(), &rkyv_ws);
    let (rkyv_mmap, rkyv_len) = open_rkyv_mmap_validated(rkyv_guard.path());
    // SAFETY: Just written above.
    let (_rkyv_u, rkyv_ulen) = unsafe { open_rkyv_mmap_unchecked(rkyv_guard.path()) };
    assert_eq!((rkyv_len, rkyv_ulen), (tasks.len(), tasks.len()));
    assert_eq!(random_read_rkyv_mmap(&rkyv_mmap, &indices), expected);

    // 4. musli-zerocopy 0.1 (lazy + full verify)
    let musli_guard = TempFileGuard::new("parity_musli", "musli");
    let musli_bytes = write_musli_workspace(musli_guard.path(), &tasks);
    let (musli_mmap, musli_len) = open_musli_mmap_lazy(musli_guard.path());
    let (_musli_full, musli_flen) = open_musli_mmap_full_verify(musli_guard.path());
    assert_eq!((musli_len, musli_flen), (tasks.len(), tasks.len()));
    assert_eq!(random_read_musli_mmap(&musli_mmap, &indices), expected);

    // 5. NVERELOC / RelocSlice<T>
    let reloc_guard = TempFileGuard::new("parity_reloc", "nvereloc");
    let reloc_bytes = write_reloc_workspace(reloc_guard.path(), &tasks).expect("write reloc");
    let reloc_ws = MmapRelocWorkspace::open(reloc_guard.path(), VerifyMode::Full).expect("reloc");
    assert_eq!(
        reloc_ws.view().random_read_checksum(&indices).unwrap(),
        expected
    );

    // 6. MmapOffsetArena
    let arena_guard = TempFileGuard::new("parity_arena", "nvearena");
    let arena =
        populate_workspace_arena(arena_guard.path(), 1, "studio-parity", &tasks, 0)
            .expect("populate arena");
    let arena_bytes = arena.used_bytes();
    assert_eq!(
        random_read_workspace_arena(&arena, &indices).unwrap(),
        expected
    );

    // Binary/zero-copy layouts must all be strictly more compact than JSON text.
    assert!(snap_bytes < json_bytes);
    assert!(rkyv_bytes < json_bytes);
    assert!(musli_bytes < json_bytes);
    assert!(reloc_bytes < json_bytes);
    assert!(arena_bytes < json_bytes);

    eprintln!(
        "1k workspace footprint bytes: serde_json={json_bytes}, nve_snap={snap_bytes}, \
         rkyv={rkyv_bytes}, musli={musli_bytes}, nvereloc={reloc_bytes}, mmap_arena={arena_bytes}"
    );
}
