//! Cross-candidate correctness, persistence, alignment, bounds, and corruption tests
//! for `NVERELOC` (`RelocSlice<T>`) and `MmapOffsetArena` (`OffsetPtr<T>` / `OffsetSlice<T>`).

use std::mem::size_of;

use nve_engine::mmap_arena::{AnonReservation, MmapOffsetArena};

use super::reloc::{
    build_reloc_workspace, compute_reloc_checksum, write_reloc_workspace, MmapRelocWorkspace,
    RelocError, RelocWorkspaceView, VerifyMode,
};
use super::workspace_arena::{
    mutate_task_in_place, populate_workspace_arena, random_read_workspace_arena, workspace_root,
    workspace_tasks, workspace_tasks_mut, ArenaTask, WorkspaceRoot,
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
        populate_workspace_arena(guard.path(), 42, "studio-main", &tasks, 4096).expect("populate");
    let addr1 = arena1.base_addr();
    assert_eq!(
        random_read_workspace_arena(&arena1, &indices).unwrap(),
        expected_acc
    );

    // Mutate scalar fields AND allocate a brand-new title string inside the same live mmap.
    let mutated_off =
        mutate_task_in_place(&mut arena1, 77, 9, 3, 1_999_999_999_000).expect("mutate");
    let new_title = arena1
        .alloc_str("UPDATED-TASK-00078: live mmap string")
        .expect("alloc_str");
    workspace_tasks_mut(&mut arena1).unwrap()[77].title = new_title;
    let root_ptr = arena1.root_ptr::<WorkspaceRoot>();
    arena1.commit_root(root_ptr).expect("commit_root");
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
    assert_eq!(workspace_root(&arena_concurrent).unwrap().workspace_id, 42);
    assert_eq!(
        arena_concurrent
            .str_at(workspace_root(&arena_concurrent).unwrap().name)
            .unwrap(),
        "studio-main"
    );
    assert_eq!(workspace_tasks(&arena_concurrent).unwrap()[77].status, 9);
    assert_eq!(
        arena_concurrent
            .str_at(workspace_tasks(&arena_concurrent).unwrap()[77].title)
            .unwrap(),
        "UPDATED-TASK-00078: live mmap string"
    );

    // Drop `arena1`, reserve anonymous pages to shift VA layout, drop `arena_concurrent`, reopen.
    let cap = arena1.capacity_bytes();
    drop(arena1);
    let _va_shift = AnonReservation::new(cap).expect("anon reserve");
    drop(arena_concurrent);

    let arena2 = MmapOffsetArena::open(guard.path()).expect("reopen after drop");
    let reopened_tasks = workspace_tasks(&arena2).expect("tasks");
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
