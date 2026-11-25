//! Cross-candidate correctness, persistence, alignment, bounds, and corruption tests
//! for `NVERELOC` (`RelocSlice<T>`) and `MmapOffsetArena` (`OffsetPtr<T>` / `OffsetSlice<T>`).

use std::mem::{align_of, size_of};

use super::mmap_arena::{ArenaError, ArenaTask, MmapOffsetArena, OffsetPtr, OffsetSlice};
use super::os_mmap::AnonReservation;
use super::reloc::{
    build_reloc_workspace, compute_reloc_checksum, write_reloc_workspace, MmapRelocWorkspace,
    RelocError, RelocWorkspaceView, VerifyMode,
};
use super::{
    deterministic_read_indices, generate_workspace_tasks, random_read_owned_tasks, TempFileGuard,
};

#[test]
fn reloc_full_header_only_and_unchecked_parity() {
    let tasks = generate_workspace_tasks(1_000);
    let indices = deterministic_read_indices(tasks.len(), 1_000);
    let expected_acc = random_read_owned_tasks(&tasks, &indices);

    let guard = TempFileGuard::new("reloc_parity", "nvereloc");
    write_reloc_workspace(guard.path(), &tasks).expect("write reloc");

    let full = MmapRelocWorkspace::open(guard.path(), VerifyMode::Full).expect("full");
    let hdr = MmapRelocWorkspace::open(guard.path(), VerifyMode::HeaderOnly).expect("header only");
    // SAFETY: Just written by `write_reloc_workspace`.
    let unchk = unsafe { MmapRelocWorkspace::open_unchecked(guard.path()) }.expect("unchecked");

    assert!(!full.is_empty());
    assert_eq!(full.view().tasks().len(), 1_000);
    assert_eq!(hdr.view().tasks().len(), 1_000);
    assert_eq!(unchk.view().tasks().len(), 1_000);

    for (i, orig) in tasks.iter().enumerate() {
        let v = full.view();
        let pod = &v.tasks()[i];
        assert_eq!(pod.id, orig.id);
        assert_eq!(pod.created_at_ms, orig.created_at_ms);
        assert_eq!(pod.updated_at_ms, orig.updated_at_ms);
        assert_eq!(pod.priority, orig.priority);
        assert_eq!(pod.status, orig.status);
        assert_eq!(v.str_at(pod.title).unwrap(), orig.title);
        assert_eq!(v.str_at(pod.body).unwrap(), orig.body);
        assert_eq!(v.str_at(pod.tag).unwrap(), orig.tag);
    }

    assert_eq!(
        full.view().random_read_checksum(&indices).unwrap(),
        expected_acc
    );
    assert_eq!(
        hdr.view().random_read_checksum(&indices).unwrap(),
        expected_acc
    );
    assert_eq!(
        unchk.view().random_read_checksum(&indices).unwrap(),
        expected_acc
    );
}

#[test]
fn reloc_handles_empty_workspace_and_rejects_corruption() {
    let empty = build_reloc_workspace(&[]).expect("build empty");
    let empty_view = RelocWorkspaceView::from_bytes(&empty, VerifyMode::Full).expect("empty view");
    assert!(empty_view.tasks().is_empty());

    let tasks = generate_workspace_tasks(32);
    let valid = build_reloc_workspace(&tasks).expect("build");

    // Truncated buffer rejected.
    assert!(matches!(
        RelocWorkspaceView::from_bytes(&valid[..16], VerifyMode::HeaderOnly),
        Err(RelocError::Truncated)
    ));

    // Misaligned slice (offset by 1 byte inside an 8-byte aligned vec).
    let mut padded = vec![0u8; valid.len() + 16];
    let base_off = (8 - (padded.as_ptr() as usize % 8)) % 8;
    padded[base_off + 1..base_off + 1 + valid.len()].copy_from_slice(&valid);
    let err = RelocWorkspaceView::from_bytes(
        &padded[base_off + 1..base_off + 1 + valid.len()],
        VerifyMode::HeaderOnly,
    )
    .err()
    .expect("must reject misaligned buffer");
    assert!(matches!(err, RelocError::Misaligned { .. }));

    // Bad magic and unsupported version.
    let mut bad_magic = valid.clone();
    bad_magic[0] ^= 0xFF;
    assert!(matches!(
        RelocWorkspaceView::from_bytes(&bad_magic, VerifyMode::HeaderOnly),
        Err(RelocError::InvalidMagic)
    ));

    let mut bad_ver = valid.clone();
    bad_ver[8..12].copy_from_slice(&99u32.to_le_bytes());
    assert!(matches!(
        RelocWorkspaceView::from_bytes(&bad_ver, VerifyMode::HeaderOnly),
        Err(RelocError::UnsupportedVersion(99))
    ));

    // Payload bit flip caught by Full checksum, skipped by HeaderOnly.
    let mut flipped = valid.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0x5A;
    assert!(RelocWorkspaceView::from_bytes(&flipped, VerifyMode::HeaderOnly).is_ok());
    assert!(matches!(
        RelocWorkspaceView::from_bytes(&flipped, VerifyMode::Full),
        Err(RelocError::ChecksumMismatch { .. })
    ));

    // Invalid UTF-8 inside string pool with recomputed checksum is still caught by Full.
    let mut bad_utf8 = valid.clone();
    let pool_off = u32::from_le_bytes(bad_utf8[40..44].try_into().unwrap()) as usize;
    bad_utf8[pool_off] = 0xFF;
    bad_utf8[pool_off + 1] = 0xFE;
    let new_ck = compute_reloc_checksum(&bad_utf8);
    bad_utf8[24..32].copy_from_slice(&new_ck.to_le_bytes());
    assert!(matches!(
        RelocWorkspaceView::from_bytes(&bad_utf8, VerifyMode::Full),
        Err(RelocError::InvalidUtf8)
    ));
}

#[test]
fn mmap_arena_persists_across_drop_and_different_virtual_address() {
    let tasks = generate_workspace_tasks(1_000);
    let indices = deterministic_read_indices(tasks.len(), 1_000);
    let expected_acc = random_read_owned_tasks(&tasks, &indices);

    let guard = TempFileGuard::new("arena_persist", "nvearena");
    let mut arena1 =
        MmapOffsetArena::populate_workspace(guard.path(), 42, "studio-main", &tasks, 4096)
            .expect("populate");
    let addr1 = arena1.base_addr();
    assert_eq!(arena1.random_read_checksum(&indices).unwrap(), expected_acc);

    // Mutate scalar fields AND allocate a brand-new title string inside the same live mmap.
    let mutated_off = arena1
        .mutate_task_in_place(77, 9, 3, 1_999_999_999_000)
        .expect("mutate");
    let new_title = arena1.alloc_str("UPDATED-TASK-00078: live mmap string").expect("alloc_str");
    arena1.tasks_mut().unwrap()[77].title = new_title;
    arena1
        .sync_range(mutated_off, size_of::<ArenaTask>())
        .expect("msync range");
    arena1.sync_all().expect("msync all");

    // Keep `arena1` mapped while opening `arena_concurrent`: OS guarantees distinct VAs.
    let arena_concurrent = MmapOffsetArena::open(guard.path()).expect("concurrent open");
    let addr_concurrent = arena_concurrent.base_addr();
    assert_ne!(
        addr1, addr_concurrent,
        "concurrent mmap must land at a different virtual address"
    );
    assert_eq!(arena_concurrent.root().unwrap().workspace_id, 42);
    assert_eq!(
        arena_concurrent
            .str_at(arena_concurrent.root().unwrap().name)
            .unwrap(),
        "studio-main"
    );
    assert_eq!(arena_concurrent.tasks().unwrap()[77].status, 9);
    assert_eq!(
        arena_concurrent
            .str_at(arena_concurrent.tasks().unwrap()[77].title)
            .unwrap(),
        "UPDATED-TASK-00078: live mmap string"
    );

    // Drop `arena1`, reserve anonymous pages to shift VA layout, drop `arena_concurrent`, reopen.
    let cap = arena1.capacity_bytes();
    drop(arena1);
    let _va_shift = AnonReservation::new(cap).expect("anon reserve");
    drop(arena_concurrent);

    let arena2 = MmapOffsetArena::open(guard.path()).expect("reopen after drop");
    let reopened_tasks = arena2.tasks().expect("tasks");
    assert_eq!(reopened_tasks.len(), tasks.len());
    assert_eq!(reopened_tasks[77].status, 9);
    assert_eq!(reopened_tasks[77].priority, 3);
    assert_eq!(reopened_tasks[77].updated_at_ms, 1_999_999_999_000);
    assert_eq!(
        arena2.str_at(reopened_tasks[77].title).unwrap(),
        "UPDATED-TASK-00078: live mmap string"
    );
    for (i, orig) in tasks.iter().enumerate() {
        assert_eq!(reopened_tasks[i].id, orig.id);
        if i != 77 {
            assert_eq!(arena2.str_at(reopened_tasks[i].title).unwrap(), orig.title);
        }
        assert_eq!(arena2.str_at(reopened_tasks[i].body).unwrap(), orig.body);
        assert_eq!(arena2.str_at(reopened_tasks[i].tag).unwrap(), orig.tag);
    }
}

#[test]
fn mmap_arena_bounds_alignment_and_corruption_detection() {
    let guard = TempFileGuard::new("arena_checks", "nvearena");
    let mut arena = MmapOffsetArena::create(guard.path(), 512).expect("create");

    // Mixed-alignment allocations must auto-align to 8 bytes for `u64`.
    let s = arena.alloc_str("abc").expect("str 3 bytes");
    assert_eq!(arena.str_at(s).unwrap(), "abc");
    let num = arena.alloc_val(0xDEAD_BEEF_CAFE_BABEu64).expect("u64");
    assert_eq!(num.offset as usize % align_of::<u64>(), 0);
    assert_eq!(*arena.get(num).unwrap(), 0xDEAD_BEEF_CAFE_BABEu64);

    // Null, misaligned, OOB, and overflowing handles rejected.
    assert!(matches!(
        arena.get(OffsetPtr::<u64>::NULL),
        Err(ArenaError::NullPointer)
    ));
    assert!(matches!(
        arena.get(OffsetPtr::<u64>::new(num.offset + 1)),
        Err(ArenaError::Misaligned { .. })
    ));
    assert!(matches!(
        arena.get(OffsetPtr::<u64>::new(4096)),
        Err(ArenaError::OutOfBounds { .. })
    ));
    assert!(matches!(
        arena.slice(OffsetSlice::<u64>::new(num.offset, u32::MAX)),
        Err(ArenaError::OutOfBounds { .. } | ArenaError::Overflow)
    ));

    // Capacity exhaustion rejected without advancing cursor.
    let before_used = arena.used_bytes();
    let huge = vec![0u8; 2048];
    assert!(matches!(
        arena.alloc_slice_copy(&huge),
        Err(ArenaError::OutOfCapacity { .. })
    ));
    assert_eq!(arena.used_bytes(), before_used);
    drop(arena);

    // Corrupt header checksum -> rejected on reopen.
    let mut bytes = std::fs::read(guard.path()).unwrap();
    bytes[24] ^= 0x01;
    std::fs::write(guard.path(), &bytes).unwrap();
    assert!(matches!(
        MmapOffsetArena::open(guard.path()),
        Err(ArenaError::HeaderChecksumMismatch { .. })
    ));

    // Corrupt magic -> rejected on reopen.
    bytes[0] = b'X';
    std::fs::write(guard.path(), &bytes).unwrap();
    assert!(matches!(
        MmapOffsetArena::open(guard.path()),
        Err(ArenaError::InvalidMagic)
    ));

    // Truncated file -> rejected on reopen.
    std::fs::write(guard.path(), &bytes[..16]).unwrap();
    assert!(matches!(
        MmapOffsetArena::open(guard.path()),
        Err(ArenaError::Truncated)
    ));
}
