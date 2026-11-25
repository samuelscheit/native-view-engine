use std::fs;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempArenaDir(PathBuf);

impl TempArenaDir {
    fn new(tag: &str) -> Self {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve_mmap_arena_test_{tag}_{}_{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempArenaDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DemoRecord {
    id: u64,
    value: i64,
    title: TrustedUtf8Str,
    tag: TrustedUtf8Str,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DemoRoot {
    version: u64,
    name: TrustedUtf8Str,
    records: OffsetSlice<DemoRecord>,
}

// SAFETY: `#[repr(C)]` zero-padding POD structs composed solely of `ArenaPod` fields.
unsafe impl ArenaPod for DemoRecord {}
unsafe impl ArenaPod for DemoRoot {}

#[test]
fn distinct_va_reopen_persistence_and_trusted_utf8_parity() {
    let dir = TempArenaDir::new("distinct_va");
    let path = dir.path().join("workspace.nvearena");

    let mut arena1 = MmapOffsetArena::create(&path, 32_768).expect("create");
    assert_eq!(arena1.active_slot(), 0);
    assert_eq!(arena1.generation(), 1);
    assert!(arena1.is_utf8_trusted());

    let name = arena1.alloc_str("Aether Studio 🚀").expect("alloc name");
    let mut recs = Vec::with_capacity(64);
    for i in 0..64u64 {
        let title = arena1
            .alloc_str(&format!("Record #{i} — zero-copy UTF-8"))
            .expect("title");
        let tag = arena1.alloc_str("engine/mmap").expect("tag");
        recs.push(DemoRecord {
            id: 1000 + i,
            value: (i as i64) * -7,
            title,
            tag,
        });
    }
    let slice = arena1.alloc_slice_copy(&recs).expect("slice");
    let root_ptr = arena1
        .alloc_val(DemoRoot {
            version: 1,
            name,
            records: slice,
        })
        .expect("root");
    let gen2 = arena1.commit_root(root_ptr).expect("commit");
    assert_eq!(gen2, 2);
    assert_eq!(arena1.active_slot(), 1);
    assert_eq!(arena1.string_count(), 1 + 64 * 2);

    // Mutate in place + append new string, then commit into Slot A (generation 3).
    let updated_title = arena1.alloc_str("Updated Record #17").expect("new title");
    {
        let root = *arena1.get(root_ptr).unwrap();
        let list = arena1.slice_mut(root.records).unwrap();
        list[17].value = 999_999;
        list[17].title = updated_title;
    }
    let gen3 = arena1.commit_root(root_ptr).expect("commit gen3");
    assert_eq!(gen3, 3);
    assert_eq!(arena1.active_slot(), 0);

    // Concurrent mapping guarantees a distinct virtual address.
    let addr1 = arena1.base_addr();
    let concurrent = MmapOffsetArena::open(&path).expect("concurrent open");
    assert_ne!(addr1, concurrent.base_addr());
    assert_eq!(concurrent.generation(), 3);
    assert_eq!(concurrent.active_slot(), 0);

    // Drop `arena1`, reserve anonymous VA pages, drop `concurrent`, reopen.
    let cap = arena1.capacity_bytes();
    drop(arena1);
    let _va_shift = AnonReservation::new(cap).expect("anon reserve");
    drop(concurrent);

    let arena2 = MmapOffsetArena::open(&path).expect("reopen");
    assert!(arena2.is_utf8_trusted());
    let root2 = arena2.get(arena2.root_ptr::<DemoRoot>()).expect("root2");
    assert_eq!(arena2.str_at(root2.name).unwrap(), "Aether Studio 🚀");
    assert_eq!(
        arena2.str_at_checked(root2.name).unwrap(),
        arena2.str_at(root2.name).unwrap()
    );
    let list2 = arena2.slice(root2.records).unwrap();
    assert_eq!(list2.len(), 64);
    assert_eq!(list2[17].value, 999_999);
    assert_eq!(arena2.str_at(list2[17].title).unwrap(), "Updated Record #17");
    for (i, rec) in list2.iter().enumerate() {
        assert_eq!(rec.id, 1000 + i as u64);
        assert_eq!(
            arena2.str_at(rec.title).unwrap(),
            arena2.str_at_checked(rec.title).unwrap()
        );
        assert_eq!(arena2.str_at(rec.tag).unwrap(), "engine/mmap");
    }
}

#[test]
fn simulated_torn_slot_a_and_torn_slot_b_recover_to_previous_generation() {
    let dir = TempArenaDir::new("torn_slots");
    let path = dir.path().join("crash_recovery.nvearena");

    // Step 1: Commit gen 2 into Slot B, then gen 3 into Slot A.
    let (root_v2, root_v3);
    {
        let mut arena = MmapOffsetArena::create(&path, 8192).expect("create");
        let s2 = arena.alloc_str("gen-2-root").unwrap();
        root_v2 = arena
            .alloc_val(DemoRoot {
                version: 2,
                name: s2,
                records: OffsetSlice::EMPTY,
            })
            .unwrap();
        assert_eq!(arena.commit_root(root_v2).unwrap(), 2);
        assert_eq!(arena.active_slot(), 1); // Slot B holds gen 2

        let s3 = arena.alloc_str("gen-3-root").unwrap();
        root_v3 = arena
            .alloc_val(DemoRoot {
                version: 3,
                name: s3,
                records: OffsetSlice::EMPTY,
            })
            .unwrap();
        assert_eq!(arena.commit_root(root_v3).unwrap(), 3);
        assert_eq!(arena.active_slot(), 0); // Slot A holds gen 3
    }

    let clean_bytes = fs::read(&path).unwrap();

    // Case A: Tear Slot A (bytes 0..64, gen 3) mid-write -> open MUST recover Slot B (gen 2)!
    {
        let mut torn_a = clean_bytes.clone();
        // Flip bytes in the middle of Slot A simulating a partial write.
        for b in &mut torn_a[20..44] {
            *b ^= 0xA5;
        }
        let path_a = dir.path().join("torn_a.nvearena");
        fs::write(&path_a, &torn_a).unwrap();

        let mut recovered = MmapOffsetArena::open(&path_a).expect("recover from torn Slot A");
        assert_eq!(recovered.active_slot(), 1);
        assert_eq!(recovered.generation(), 2);
        let root = recovered.get(recovered.root_ptr::<DemoRoot>()).unwrap();
        assert_eq!(root.version, 2);
        assert_eq!(recovered.str_at(root.name).unwrap(), "gen-2-root");

        // Next commit heals Slot A cleanly with generation 3.
        let healed_name = recovered.alloc_str("healed-after-torn-a").unwrap();
        let healed_root = recovered
            .alloc_val(DemoRoot {
                version: 99,
                name: healed_name,
                records: OffsetSlice::EMPTY,
            })
            .unwrap();
        assert_eq!(recovered.commit_root(healed_root).unwrap(), 3);
        assert_eq!(recovered.active_slot(), 0);
        drop(recovered);

        let reopened = MmapOffsetArena::open(&path_a).unwrap();
        assert_eq!(reopened.generation(), 3);
        assert_eq!(reopened.active_slot(), 0);
        assert_eq!(
            reopened
                .str_at(reopened.get(reopened.root_ptr::<DemoRoot>()).unwrap().name)
                .unwrap(),
            "healed-after-torn-a"
        );
    }

    // Case B: Advance to gen 4 in Slot B, then tear Slot B -> open MUST recover Slot A (gen 3)!
    {
        let path_b = dir.path().join("torn_b.nvearena");
        fs::write(&path_b, &clean_bytes).unwrap();
        {
            let mut arena = MmapOffsetArena::open(&path_b).unwrap();
            let s4 = arena.alloc_str("gen-4-root").unwrap();
            let root_v4 = arena
                .alloc_val(DemoRoot {
                    version: 4,
                    name: s4,
                    records: OffsetSlice::EMPTY,
                })
                .unwrap();
            assert_eq!(arena.commit_root(root_v4).unwrap(), 4);
            assert_eq!(arena.active_slot(), 1); // Slot B holds gen 4, Slot A holds gen 3
        }

        let mut torn_b = fs::read(&path_b).unwrap();
        // Zero out second half of Slot B (bytes 96..128) simulating torn cache line.
        torn_b[96..128].fill(0xFF);
        fs::write(&path_b, &torn_b).unwrap();

        let recovered = MmapOffsetArena::open(&path_b).expect("recover from torn Slot B");
        assert_eq!(recovered.active_slot(), 0);
        assert_eq!(recovered.generation(), 3);
        let root = recovered.get(recovered.root_ptr::<DemoRoot>()).unwrap();
        assert_eq!(root.version, 3);
        assert_eq!(recovered.str_at(root.name).unwrap(), "gen-3-root");
        assert_eq!(recovered.root_ptr::<DemoRoot>(), root_v3);
    }
    let _ = root_v2;
}

#[test]
fn compaction_reclaims_dead_string_slack_and_preserves_live_data() {
    let dir = TempArenaDir::new("compaction");
    let path = dir.path().join("live.nvearena");

    // Populate 32 records, then overwrite their strings 25 times to accumulate dead slack space.
    let mut arena = MmapOffsetArena::create(&path, 256 * 1024).expect("create");
    let name = arena.alloc_str("compaction-ws").unwrap();
    let mut initial = Vec::new();
    for i in 0..32u64 {
        let title = arena.alloc_str(&format!("init-title-{i}")).unwrap();
        let tag = arena.alloc_str("v0").unwrap();
        initial.push(DemoRecord {
            id: i,
            value: i as i64,
            title,
            tag,
        });
    }
    let recs_slice = arena.alloc_slice_copy(&initial).unwrap();
    let root_ptr = arena
        .alloc_val(DemoRoot {
            version: 1,
            name,
            records: recs_slice,
        })
        .unwrap();
    arena.commit_root(root_ptr).unwrap();

    for round in 1..=25u32 {
        for i in 0..32usize {
            let new_title = arena
                .alloc_str(&format!(
                    "round-{round:02}-record-{i:02}-padding-text-that-becomes-garbage-on-next-round"
                ))
                .unwrap();
            let root = *arena.get(root_ptr).unwrap();
            arena.slice_mut(root.records).unwrap()[i].title = new_title;
            arena.slice_mut(root.records).unwrap()[i].value = (round as i64) * 100 + i as i64;
        }
    }
    let gen_before = arena.commit_root(root_ptr).unwrap();
    let used_before = arena.used_bytes();
    let file_len_before = fs::metadata(&path).unwrap().len() as usize;
    assert!(used_before > 50_000, "expected substantial dead slack");

    // Compact in-place with 1024 bytes of headroom.
    let (reported_before, used_after) = arena
        .compact_in_place(1024, |old, new| {
            let old_root = *old.get(old.root_ptr::<DemoRoot>())?;
            let old_recs = old.slice(old_root.records)?;
            let new_name = new.alloc_str(old.str_at(old_root.name)?)?;
            let mut new_recs = Vec::with_capacity(old_recs.len());
            for r in old_recs {
                new_recs.push(DemoRecord {
                    id: r.id,
                    value: r.value,
                    title: new.alloc_str(old.str_at(r.title)?)?,
                    tag: new.alloc_str(old.str_at(r.tag)?)?,
                });
            }
            let slice = new.alloc_slice_copy(&new_recs)?;
            new.alloc_val(DemoRoot {
                version: old_root.version,
                name: new_name,
                records: slice,
            })
        })
        .expect("compact_in_place");

    assert_eq!(reported_before, used_before);
    assert!(
        used_after * 4 < used_before,
        "expected >75% slack reclamation: before={used_before}, after={used_after}"
    );
    let file_len_after = fs::metadata(&path).unwrap().len() as usize;
    assert!(
        file_len_after < file_len_before / 4,
        "expected on-disk file shrink: before={file_len_before}, after={file_len_after}"
    );
    assert!(arena.generation() > gen_before);
    assert_eq!(arena.string_count(), 1 + 32 * 2);

    // Ensure no orphan `.compact.` temp files remain.
    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["live.nvearena"]);

    // Reopen from disk and verify 100% live record & string fidelity.
    drop(arena);
    let reopened = MmapOffsetArena::open(&path).expect("reopen compacted");
    let root = reopened.get(reopened.root_ptr::<DemoRoot>()).unwrap();
    assert_eq!(reopened.str_at(root.name).unwrap(), "compaction-ws");
    let recs = reopened.slice(root.records).unwrap();
    assert_eq!(recs.len(), 32);
    for (i, r) in recs.iter().enumerate() {
        assert_eq!(r.id, i as u64);
        assert_eq!(r.value, 2500 + i as i64);
        assert_eq!(
            reopened.str_at(r.title).unwrap(),
            format!("round-25-record-{i:02}-padding-text-that-becomes-garbage-on-next-round")
        );
        assert_eq!(reopened.str_at(r.tag).unwrap(), "v0");
    }
}

#[test]
fn bounds_alignment_overflow_corrupt_headers_and_untrusted_utf8_rejection() {
    let dir = TempArenaDir::new("guards");
    let path = dir.path().join("guards.nvearena");
    let mut arena = MmapOffsetArena::create(&path, 512).expect("create");

    // Odd string followed by u64 auto-aligns to 8 bytes.
    let s = arena.alloc_str("abc").expect("str");
    assert_eq!(arena.str_at(s).unwrap(), "abc");
    let num = arena.alloc_val(0xDEAD_BEEF_CAFE_BABEu64).expect("u64");
    assert_eq!((num.offset as usize) % align_of::<u64>(), 0);
    assert_eq!(*arena.get(num).unwrap(), 0xDEAD_BEEF_CAFE_BABEu64);

    // Null, misaligned, OOB, header-overlap, and overflowing handles rejected.
    assert!(matches!(
        arena.get(OffsetPtr::<u64>::NULL),
        Err(ArenaError::NullPointer)
    ));
    assert!(matches!(
        arena.get(OffsetPtr::<u64>::new(num.offset + 1)),
        Err(ArenaError::Misaligned { .. })
    ));
    assert!(matches!(
        arena.get(OffsetPtr::<u64>::new(16)),
        Err(ArenaError::OutOfBounds { .. })
    ));
    assert!(matches!(
        arena.get(OffsetPtr::<u64>::new(4096)),
        Err(ArenaError::OutOfBounds { .. })
    ));
    assert!(matches!(
        arena.slice(OffsetSlice::<u64>::new(num.offset, u32::MAX)),
        Err(ArenaError::OutOfBounds { .. } | ArenaError::Overflow)
    ));
    assert!(matches!(
        arena.str_at(TrustedUtf8Str::new(num.offset, u32::MAX)),
        Err(ArenaError::Overflow | ArenaError::OutOfBounds { .. })
    ));

    // Capacity exhaustion does not advance `used_bytes`.
    let before = arena.used_bytes();
    assert!(matches!(
        arena.alloc_slice_copy(&vec![0u8; 2048]),
        Err(ArenaError::OutOfCapacity { .. })
    ));
    assert_eq!(arena.used_bytes(), before);

    arena.commit_root(num).unwrap();
    drop(arena);

    // Corrupt stored string bytes on disk -> `open_untrusted` catches invalid UTF-8!
    let mut bad_utf8 = fs::read(&path).unwrap();
    bad_utf8[s.offset as usize] = 0xFF;
    bad_utf8[s.offset as usize + 1] = 0xFE;
    let bad_utf8_path = dir.path().join("bad_utf8.nvearena");
    fs::write(&bad_utf8_path, &bad_utf8).unwrap();
    let untrusted = MmapOffsetArena::open_untrusted(&bad_utf8_path).unwrap();
    assert!(!untrusted.is_utf8_trusted());
    assert!(matches!(untrusted.str_at(s), Err(ArenaError::InvalidUtf8)));
    assert!(matches!(
        MmapOffsetArena::open_and_verify(&bad_utf8_path, |a| {
            let _ = a.str_at_checked(s)?;
            Ok(())
        }),
        Err(ArenaError::InvalidUtf8)
    ));

    // Corrupt BOTH header slots -> rejected with specific errors.
    let valid = fs::read(&path).unwrap();

    // Checksum mismatch in both slots.
    let mut bad_ck = valid.clone();
    bad_ck[32] ^= 0x01;
    bad_ck[64 + 32] ^= 0x01;
    fs::write(&path, &bad_ck).unwrap();
    assert!(matches!(
        MmapOffsetArena::open(&path),
        Err(ArenaError::HeaderChecksumMismatch { .. })
    ));

    // Unsupported version in both slots.
    let mut bad_ver = valid.clone();
    bad_ver[8..12].copy_from_slice(&99u32.to_le_bytes());
    bad_ver[64 + 8..64 + 12].copy_from_slice(&99u32.to_le_bytes());
    fs::write(&path, &bad_ver).unwrap();
    assert!(matches!(
        MmapOffsetArena::open(&path),
        Err(ArenaError::UnsupportedVersion(99))
    ));

    // Bad magic in both slots.
    let mut bad_magic = valid.clone();
    bad_magic[0] = b'X';
    bad_magic[64] = b'X';
    fs::write(&path, &bad_magic).unwrap();
    assert!(matches!(
        MmapOffsetArena::open(&path),
        Err(ArenaError::InvalidMagic)
    ));

    // Truncated file (< 128 bytes).
    fs::write(&path, &valid[..64]).unwrap();
    assert!(matches!(
        MmapOffsetArena::open(&path),
        Err(ArenaError::Truncated)
    ));
}

#[test]
fn in_memory_arena_supports_all_core_operations() {
    let mut arena = MmapOffsetArena::new_in_memory(4096).expect("in memory");
    let s = arena.alloc_str("wasm-safe-string").unwrap();
    let r = arena
        .alloc_val(DemoRoot {
            version: 7,
            name: s,
            records: OffsetSlice::EMPTY,
        })
        .unwrap();
    assert_eq!(arena.commit_root(r).unwrap(), 2);
    assert_eq!(
        arena.str_at(arena.get(arena.root_ptr::<DemoRoot>()).unwrap().name).unwrap(),
        "wasm-safe-string"
    );
    assert_eq!(size_of::<DemoRecord>(), 32);
    assert!(matches!(
        arena.compact_in_place::<DemoRoot, _>(64, |_, _| Ok(OffsetPtr::NULL)),
        Err(ArenaError::NotFileBacked)
    ));
}
