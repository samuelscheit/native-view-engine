//! Comprehensive tests and thread-local `CountingAlloc` O(1)-allocation guards for
//! `Frame0File` (`frame0.nve2`) and atomic `StateFile` (`state.nve`).

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use std::path::Path;

use nve_components::tables::{
    Interaction, Layout, TextContent, Typography, Visual, core_realization_order,
    core_tables_schema_hash,
};
use nve_engine::{
    BackendApplyStats, BackendError, FRAME0_FOOTER_SIZE, Frame0File, Frame0Meta, Frame0Miss,
    PersistenceError, STATE_FILE_HEADER_SIZE, StateFile, StoreBackend, StoreWireReplica, Viewport,
    write_file_atomic,
};
use nve_store::wire::{MAGIC as NVE2_MAGIC, VERSION as NVE2_VERSION, WireDecoder, WireEncoder};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, RealizationOrder, Rgba};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[inline(always)]
fn bump() {
    let _ = THREAD_ALLOCS.try_with(|n| n.set(n.get().wrapping_add(1)));
}

// SAFETY: Delegates verbatim to `System` while bumping a thread-local counter.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: AllocLayout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc_zeroed` preconditions; delegated to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    (out, THREAD_ALLOCS.with(Cell::get).wrapping_sub(before))
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_file(name: &str) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nve_engine_persist_{}_{}_{}",
        std::process::id(),
        seq,
        name
    ))
}

struct CountingBackend {
    commits: usize,
    last_live: usize,
}

impl StoreBackend for CountingBackend {
    fn name(&self) -> &'static str {
        "counting-test"
    }

    fn set_viewport(&mut self, _surface_id: nve_abi::SurfaceId, _viewport: Viewport) {}

    fn commit(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError> {
        self.commits += 1;
        self.last_live = store.stats().live_nodes;
        Ok(BackendApplyStats {
            op_count: changes.created.len() as u32 + changes.removed.len() as u32,
            mutation_count: changes.created.len() as u32 + changes.removed.len() as u32,
            decode_us: 0,
            layout_us: 0,
            diff_us: 0,
            mount_us: 0,
        })
    }

    fn reset(&mut self) {
        self.last_live = 0;
    }
}

fn build_store(node_count: usize) -> NodeStore {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let mut kids = Vec::with_capacity(node_count);
    for i in 0..node_count {
        let mut nm = tx.spawn();
        nm.edit::<Layout>()
            .width(Length::points(200.0))
            .height(Length::points(32.0));
        nm.edit::<Visual>()
            .background(Rgba::rgb((i & 0xFF) as u8, 40, 60))
            .border_radius(8.0)
            .opacity(1.0);
        if i % 2 == 0 {
            nm.edit::<Typography>()
                .font_size(14.0)
                .color(Rgba::rgb(240, 244, 248));
            nm.edit::<TextContent>().text("Frame0 Label");
        }
        if i % 4 == 0 {
            nm.edit::<Interaction>().event_mask(1);
        }
        kids.push(nm.id());
    }
    tx.set_children(NodeId::ROOT, &kids);
    tx.commit_into(&mut ChangeSet::default());
    store
}

#[test]
fn state_file_atomic_roundtrip_schema_mismatch_corrupt_recovery_and_reset() {
    let path = temp_file("state.nve");
    let _ = std::fs::remove_file(&path);

    let mut sf = StateFile::open(&path, 7).expect("open fresh StateFile");
    assert_eq!(sf.generation(), 0);
    assert!(!sf.recovery_stats().loaded_from_disk);
    assert_eq!(sf.load().expect("load empty"), None);

    let out1 = sf.save(b"gen-1-payload").expect("save gen 1");
    assert_eq!(out1.generation, 1);
    assert_eq!(
        out1.byte_len,
        STATE_FILE_HEADER_SIZE + b"gen-1-payload".len()
    );
    let out2 = sf.save(b"gen-2-longer-payload").expect("save gen 2");
    assert_eq!(out2.generation, 2);
    assert_eq!(
        sf.load().expect("load gen 2").as_deref(),
        Some(b"gen-2-longer-payload".as_slice())
    );

    // Reopen with matching schema -> loaded_from_disk = true (single-read cached payload).
    let reopened = StateFile::open(&path, 7).expect("reopen matching schema");
    assert!(reopened.recovery_stats().loaded_from_disk);
    assert!(!reopened.recovery_stats().recovered_from_corrupt);
    assert_eq!(
        reopened.load().expect("load reopened").as_deref(),
        Some(b"gen-2-longer-payload".as_slice())
    );

    // Reopen expecting an OLDER schema (expected 6 < on-disk 7 = newer file on disk) ->
    // returns SchemaMismatch WITHOUT deleting or moving user data.
    assert!(matches!(
        StateFile::open(&path, 6),
        Err(PersistenceError::SchemaMismatch {
            expected: 6,
            actual: 7
        })
    ));
    assert!(
        path.exists(),
        "newer on-disk schema mismatch must preserve state file untouched"
    );
    let still_intact = StateFile::open(&path, 7).expect("original schema still readable");
    assert_eq!(
        still_intact.load().expect("load intact").as_deref(),
        Some(b"gen-2-longer-payload".as_slice())
    );

    // Reopen with a NEWER app schema (expected 8 > on-disk 7 = older file on disk) ->
    // rotates on-disk file to `<name>.v7` and keeps persistence active with fresh state.
    let rotated_backup = path.with_file_name(format!(
        "{}.v7",
        path.file_name().unwrap().to_str().unwrap()
    ));
    let _ = std::fs::remove_file(&rotated_backup);
    let mut upgraded = StateFile::open(&path, 8).expect("older schema rotates cleanly");
    assert_eq!(upgraded.recovery_stats().rotated_from_older_schema, Some(7));
    assert!(!upgraded.recovery_stats().loaded_from_disk);
    assert_eq!(upgraded.load().expect("fresh upgraded state"), None);
    assert!(
        rotated_backup.exists(),
        "older schema file must be rotated to .v7 backup"
    );
    upgraded
        .save(b"gen-v8-payload")
        .expect("save under upgraded v8");
    assert_eq!(
        upgraded.load().expect("load v8").as_deref(),
        Some(b"gen-v8-payload".as_slice())
    );
    let _ = std::fs::remove_file(&rotated_backup);
    // Restore schema 7 file for subsequent bit-flip/quarantine checks below.
    assert!(matches!(
        StateFile::open(&path, 7),
        Err(PersistenceError::SchemaMismatch {
            expected: 7,
            actual: 8
        })
    ));
    upgraded.reset().expect("reset before v7 save");
    let mut sf_v7 = StateFile::open(&path, 7).expect("reopen clean v7");
    sf_v7
        .save(b"gen-2-longer-payload")
        .expect("re-save v7 payload");

    // Single bit-flip in header (magic, schema_version, len), checksum, OR payload must report
    // Corrupt (never SchemaMismatch).
    let valid_raw = std::fs::read(&path).expect("read raw state");
    for pos in [0usize, 7, 8, 11, 12, 15, 16, 23, valid_raw.len() - 1] {
        let mut flipped = valid_raw.clone();
        flipped[pos] ^= 0x01;
        assert!(
            matches!(
                StateFile::decode_envelope(&flipped, 7),
                Err(PersistenceError::Corrupt(_))
            ),
            "bit flip at byte {pos} must fail with Corrupt, not SchemaMismatch"
        );
    }
    assert!(matches!(
        StateFile::decode_envelope(&valid_raw[..STATE_FILE_HEADER_SIZE - 1], 7),
        Err(PersistenceError::Corrupt(_))
    ));

    // Corrupt file on disk is quarantined to `<name>.corrupt` on reopen.
    let mut corrupt_raw = valid_raw;
    let last = corrupt_raw.len() - 1;
    corrupt_raw[last] ^= 0x01;
    std::fs::write(&path, &corrupt_raw).expect("write bit-flipped state");

    let backup_path = path.with_file_name(format!(
        "{}.corrupt",
        path.file_name().unwrap().to_str().unwrap()
    ));
    let _ = std::fs::remove_file(&backup_path);
    let recovered = StateFile::open(&path, 7).expect("recover from bit flip");
    assert!(recovered.recovery_stats().recovered_from_corrupt);
    assert_eq!(recovered.load().expect("load recovered"), None);
    assert!(!path.exists());
    assert!(
        backup_path.exists(),
        "corrupt state file must be moved to .corrupt backup instead of silently lost"
    );
    let _ = std::fs::remove_file(&backup_path);

    // I/O error (e.g. path is an existing directory) must return Err(Io) without removing it.
    let dir_path = temp_file("state_is_dir");
    std::fs::create_dir_all(&dir_path).expect("create dir");
    assert!(matches!(
        StateFile::open(&dir_path, 7),
        Err(PersistenceError::Io(_))
    ));
    assert!(dir_path.is_dir(), "I/O error must never delete path");
    let _ = std::fs::remove_dir(&dir_path);

    // write_file_atomic must reject paths with no file name.
    assert!(matches!(
        write_file_atomic(Path::new(""), b"abc", true),
        Err(PersistenceError::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput
    ));

    // Direct quarantine_corrupt() resets both generation() and recovery_stats().generation to 0.
    let q_path = temp_file("state_quarantine.nve");
    let q_backup = q_path.with_file_name(format!(
        "{}.corrupt",
        q_path.file_name().unwrap().to_str().unwrap()
    ));
    let _ = std::fs::remove_file(&q_path);
    let _ = std::fs::remove_file(&q_backup);
    {
        let mut sf_q = StateFile::open(&q_path, 7).expect("open quarantine target");
        sf_q.save(b"v7-payload-1").expect("save 1");
        sf_q.save(b"v7-payload-2").expect("save 2");
        assert_eq!(sf_q.generation(), 2);
    }
    let mut sf_q_reopen = StateFile::open(&q_path, 7).expect("reopen before quarantine");
    assert_eq!(sf_q_reopen.generation(), 1);
    assert_eq!(sf_q_reopen.recovery_stats().generation, 1);
    sf_q_reopen
        .quarantine_corrupt()
        .expect("quarantine_corrupt");
    assert_eq!(sf_q_reopen.generation(), 0);
    assert_eq!(sf_q_reopen.recovery_stats().generation, 0);
    assert!(!sf_q_reopen.recovery_stats().loaded_from_disk);
    assert!(sf_q_reopen.recovery_stats().recovered_from_corrupt);
    assert_eq!(sf_q_reopen.load().expect("load quarantined"), None);
    assert!(!q_path.exists());
    assert!(q_backup.exists());
    let _ = std::fs::remove_file(&q_backup);

    // Reset + in-memory mode.
    let mut mem = StateFile::new_in_memory(3);
    assert!(mem.path().is_none());
    mem.save(b"mem-state").expect("save mem");
    assert_eq!(
        mem.load().expect("load mem").as_deref(),
        Some(b"mem-state".as_slice())
    );
    mem.reset().expect("reset mem");
    assert_eq!(mem.load().expect("load reset mem"), None);
}

#[test]
fn frame0_file_roundtrip_mismatches_and_corrupt_auto_delete() {
    let store = build_store(32);
    let meta = Frame0Meta {
        table_schema_hash: core_tables_schema_hash(),
        app_build_id: 0xDEAD_BEEF_1234_5678,
        env_hash: 0xCAFE_BABE_0000_0001,
    };

    let path = temp_file("frame0.nve2");
    let _ = std::fs::remove_file(&path);

    // NotFound before save.
    assert_eq!(
        Frame0File::read_and_validate(&path, meta).unwrap_err(),
        Frame0Miss::NotFound
    );

    let saved_footer = Frame0File::save(&path, &store, meta).expect("save frame0");
    assert_eq!(saved_footer.node_count, store.stats().live_nodes as u32);
    assert_eq!(saved_footer.flags, 0);

    // Boot from disk into replica -> exact node count parity.
    let mut replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        RealizationOrder::default(),
    );
    let (footer, stats) =
        Frame0File::boot_into_replica(&path, meta, &mut replica).expect("boot frame0");
    assert_eq!(footer, saved_footer);
    assert!(stats.mutation_count >= 32);
    assert_eq!(replica.store().stats().live_nodes, store.stats().live_nodes);
    assert_eq!(replica.backend().commits, 1);

    // Footer metadata mismatches on disk must reject cleanly WITHOUT deleting a valid cache file.
    let raw = std::fs::read(&path).expect("read frame0 bytes");
    assert!(matches!(
        Frame0File::read_and_validate(
            &path,
            Frame0Meta {
                table_schema_hash: meta.table_schema_hash ^ 1,
                ..meta
            }
        ),
        Err(Frame0Miss::SchemaMismatch { .. })
    ));
    assert!(matches!(
        Frame0File::boot_into_replica(
            &path,
            Frame0Meta {
                app_build_id: meta.app_build_id ^ 1,
                ..meta
            },
            &mut replica
        ),
        Err(Frame0Miss::BuildIdMismatch { .. })
    ));
    assert!(matches!(
        Frame0File::read_and_validate(
            &path,
            Frame0Meta {
                env_hash: meta.env_hash ^ 1,
                ..meta
            }
        ),
        Err(Frame0Miss::EnvHashMismatch { .. })
    ));
    assert!(
        path.exists(),
        "metadata mismatch must not delete valid cache"
    );

    // Non-zero flags (re-signed with valid XXH64) -> FlagsMismatch.
    let mut flagged = raw.clone();
    let flag_off = flagged.len() - FRAME0_FOOTER_SIZE + 4;
    flagged[flag_off..flag_off + 4].copy_from_slice(&1u32.to_le_bytes());
    let hash_off = flagged.len() - 8;
    let rehash = nve_abi::xxh64(&flagged[..hash_off], 0x4E56_4546_524D_3032);
    flagged[hash_off..].copy_from_slice(&rehash.to_le_bytes());
    assert_eq!(
        Frame0File::validate_bytes(&flagged, meta).unwrap_err(),
        Frame0Miss::FlagsMismatch(1)
    );

    // 1. Truncated below footer -> Corrupt + auto-deleted.
    std::fs::write(&path, &raw[..FRAME0_FOOTER_SIZE - 1]).expect("write truncated");
    assert!(matches!(
        Frame0File::read_and_validate(&path, meta),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert!(!path.exists(), "truncated frame0 must be auto-deleted");

    // 2. Single bit-flip in payload -> Corrupt + auto-deleted.
    let mut flip_payload = raw.clone();
    flip_payload[12] ^= 0x01;
    std::fs::write(&path, &flip_payload).expect("write payload flip");
    assert!(matches!(
        Frame0File::read_and_validate(&path, meta),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert!(
        !path.exists(),
        "payload-corrupt frame0 must be auto-deleted"
    );

    // 3. Single bit-flip in footer metadata -> Corrupt + auto-deleted.
    let mut flip_meta = raw.clone();
    let meta_off = flip_meta.len() - FRAME0_FOOTER_SIZE + 10;
    flip_meta[meta_off] ^= 0x80;
    std::fs::write(&path, &flip_meta).expect("write footer meta flip");
    assert!(matches!(
        Frame0File::read_and_validate(&path, meta),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert!(
        !path.exists(),
        "footer-meta-corrupt frame0 must be auto-deleted"
    );

    // 4. Single bit-flip in footer checksum -> Corrupt + auto-deleted.
    let mut flip_hash = raw;
    let last = flip_hash.len() - 1;
    flip_hash[last] ^= 0x04;
    std::fs::write(&path, &flip_hash).expect("write checksum flip");
    assert!(matches!(
        Frame0File::read_and_validate(&path, meta),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert!(
        !path.exists(),
        "checksum-corrupt frame0 must be auto-deleted"
    );
}

#[test]
fn frame0_decode_into_presized_store_has_o1_allocations_1k_vs_5k() {
    // Positive probe verifying CountingAlloc actually observes allocations on this thread.
    let (_, probe_allocs) = count_allocs(|| {
        let mut v = Vec::<u8>::with_capacity(64);
        v.push(1);
        v
    });
    assert!(
        probe_allocs >= 1,
        "CountingAlloc probe must observe >= 1 allocation"
    );

    let store_1k = build_store(1_000);
    let store_5k = build_store(5_000);
    let meta = Frame0Meta {
        table_schema_hash: core_tables_schema_hash(),
        app_build_id: 42,
        env_hash: 99,
    };
    let bytes_1k = Frame0File::encode(&store_1k, meta).expect("encode 1k");
    let bytes_5k = Frame0File::encode(&store_5k, meta).expect("encode 5k");

    let decode_measure = |bytes: &[u8]| -> (usize, usize) {
        let mut target = NodeStore::replica();
        let mut decoder = WireDecoder::default();
        let mut changes = ChangeSet::default();
        let (live, allocs) = count_allocs(|| {
            let (footer, payload) =
                Frame0File::validate_bytes(bytes, meta).expect("valid frame0 bytes");
            target
                .reserve_for_frame0(footer.node_count as usize, payload.len())
                .expect("reserve store");
            changes
                .reserve_for_frame0(footer.node_count as usize, payload.len())
                .expect("reserve changes");
            let mut tx = target.tx();
            decoder
                .apply(&mut tx, payload)
                .expect("decode NVE2 into pre-sized NodeStore");
            tx.commit_into(&mut changes);
            target.stats().live_nodes
        });
        (live, allocs)
    };

    // Sanity-check encode_nve2 helper too.
    let mut raw_nve2 = Vec::new();
    WireEncoder::default().encode_full(&store_1k, &mut raw_nve2);
    let wrapped =
        Frame0File::encode_nve2(&raw_nve2, store_1k.stats().live_nodes as u32, meta).expect("wrap");
    assert_eq!(wrapped, bytes_1k);

    let (live_1k, allocs_1k) = decode_measure(&bytes_1k);
    let (live_5k, allocs_5k) = decode_measure(&bytes_5k);
    eprintln!("[persistence_tests] NVE2 decode allocs: 1k={allocs_1k}, 5k={allocs_5k}");
    assert_eq!(live_1k, 1_001);
    assert_eq!(live_5k, 5_001);
    assert!(allocs_1k >= 1);
    assert_eq!(
        allocs_5k, allocs_1k,
        "Frame-0 NVE2 decode allocations must be strictly constant after pre-sizing: 1k={allocs_1k}, 5k={allocs_5k}"
    );
}

fn sign_frame0_payload(payload: &[u8], node_count: u32, meta: Frame0Meta) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + FRAME0_FOOTER_SIZE);
    out.extend_from_slice(payload);
    let start = out.len();
    out.extend_from_slice(&node_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&meta.table_schema_hash.to_le_bytes());
    out.extend_from_slice(&meta.app_build_id.to_le_bytes());
    out.extend_from_slice(&meta.env_hash.to_le_bytes());
    let hash = nve_abi::xxh64(&out[..start + 32], 0x4E56_4546_524D_3032);
    out.extend_from_slice(&hash.to_le_bytes());
    out
}

fn minimal_nve2_header(frame: u64, op_count: u32, total_words: u32) -> Vec<u8> {
    let mut hdr = Vec::with_capacity(24);
    hdr.extend_from_slice(&NVE2_MAGIC.to_le_bytes());
    hdr.extend_from_slice(&NVE2_VERSION.to_le_bytes());
    hdr.extend_from_slice(&0u16.to_le_bytes());
    hdr.extend_from_slice(&frame.to_le_bytes());
    hdr.extend_from_slice(&op_count.to_le_bytes());
    hdr.extend_from_slice(&total_words.to_le_bytes());
    hdr
}

#[test]
fn hostile_frame0_u32_max_node_count_rejected_without_oom() {
    let meta = Frame0Meta {
        table_schema_hash: core_tables_schema_hash(),
        app_build_id: 1,
        env_hash: 0,
    };
    // 24-byte minimal valid NVE2 keyframe + 40-byte signed footer claiming u32::MAX nodes.
    let min_nve2 = minimal_nve2_header(1, 0, 6);
    let hostile = sign_frame0_payload(&min_nve2, u32::MAX, meta);
    assert_eq!(hostile.len(), 64);

    assert!(matches!(
        Frame0File::validate_bytes(&hostile, meta),
        Err(Frame0Miss::Corrupt(_))
    ));
    let mut replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        core_realization_order(),
    );
    assert!(matches!(
        Frame0File::boot_bytes_into_replica(&hostile, meta, &mut replica),
        Err(Frame0Miss::Corrupt(_))
    ));
    // Direct StoreWireReplica::submit_nve2_frame_with_hint must ALSO bound hostile hints cleanly.
    replica
        .submit_nve2_frame_with_hint(&min_nve2, u32::MAX as usize)
        .expect("bounded hint on empty NVE2 must succeed without OOM");
}

#[test]
fn frame0_rejects_delta_frames_and_node_count_mismatch() {
    let meta = Frame0Meta {
        table_schema_hash: core_tables_schema_hash(),
        app_build_id: 5,
        env_hash: 0,
    };
    let delta_hdr = minimal_nve2_header(2, 0, 6);
    assert!(matches!(
        Frame0File::encode_nve2(&delta_hdr, 0, meta),
        Err(PersistenceError::Corrupt(_))
    ));
    let signed_delta = sign_frame0_payload(&delta_hdr, 0, meta);
    assert!(matches!(
        Frame0File::validate_bytes(&signed_delta, meta),
        Err(Frame0Miss::Corrupt(_))
    ));

    // Valid 10-node keyframe whose footer falsely claims 5 nodes (passes payload.len()/4 check
    // but must fail post-decode live-node verification, reset replica, and delete disk file).
    let store = build_store(10);
    let mut raw_nve2 = Vec::new();
    WireEncoder::default().encode_full(&store, &mut raw_nve2);
    let lying_footer = sign_frame0_payload(&raw_nve2, 5, meta);
    let path = temp_file("lying_count.nve2");
    std::fs::write(&path, &lying_footer).expect("write lying frame0");

    let mut replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        core_realization_order(),
    );
    assert!(matches!(
        Frame0File::boot_into_replica(&path, meta, &mut replica),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert_eq!(
        replica.backend().commits,
        0,
        "lying footer node_count must fail BEFORE backend.commit runs"
    );
    assert_eq!(replica.store().stats().live_nodes, 0);
    assert!(replica.needs_keyframe());
    assert!(
        !path.exists(),
        "mismatched node_count frame0 must be deleted"
    );
}

#[test]
fn frame0_backend_commit_error_preserves_cache_while_corrupt_body_deletes_it() {
    struct FailingCommitBackend;
    impl StoreBackend for FailingCommitBackend {
        fn name(&self) -> &'static str {
            "failing-commit"
        }
        fn set_viewport(&mut self, _: nve_abi::SurfaceId, _: Viewport) {}
        fn commit(
            &mut self,
            _: &NodeStore,
            _: &ChangeSet,
        ) -> Result<BackendApplyStats, BackendError> {
            Err(BackendError::Internal("transient gpu surface lost".into()))
        }
        fn reset(&mut self) {}
    }

    let store = build_store(8);
    let meta = Frame0Meta {
        table_schema_hash: core_tables_schema_hash(),
        app_build_id: 9,
        env_hash: 0,
    };
    let path = temp_file("backend_err.nve2");
    Frame0File::save(&path, &store, meta).expect("save valid frame0");

    let mut failing = StoreWireReplica::new(FailingCommitBackend, core_realization_order());
    assert!(matches!(
        Frame0File::boot_into_replica(&path, meta, &mut failing),
        Err(Frame0Miss::Backend(ref msg)) if msg.contains("transient gpu surface lost")
    ));
    assert_eq!(
        failing.store().stats().live_nodes,
        0,
        "Backend::commit failure must reset replica so store and backend never diverge"
    );
    assert!(failing.needs_keyframe());
    assert!(
        path.exists(),
        "transient BackendError::commit failure must NOT delete a valid frame0.nve2"
    );

    // Corrupt NVE2 body (valid frame == 1 header claiming 1 op, truncated op payload, valid footer XXH64)
    // MUST return Corrupt and delete the corrupt file.
    let mut bad_body = minimal_nve2_header(1, 1, 7);
    bad_body.extend_from_slice(&(1u32 | (16u32 << 16)).to_le_bytes());
    let signed_bad_body = sign_frame0_payload(&bad_body, 0, meta);
    std::fs::write(&path, &signed_bad_body).expect("write corrupt body frame0");
    let mut ok_replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        core_realization_order(),
    );
    assert!(matches!(
        Frame0File::boot_into_replica(&path, meta, &mut ok_replica),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert!(
        !path.exists(),
        "corrupt NVE2 decode failure must auto-delete frame0.nve2"
    );
}

#[test]
fn hostile_padded_sparse_node_and_resource_frame0_rejected_without_oom() {
    let meta = Frame0Meta::default_core();
    // Pad payload with 16,384 Op::ReleaseResources entries (65 KiB -> session budget ~17 MiB) +
    // 1 Op::CreateNodes at NodeId(600_000) (19.2 MiB > 17 MiB) -> rejected before allocating.
    let pad_count: u32 = 16_384;
    let mut ops_body = Vec::with_capacity((pad_count as usize + 8) * 4);
    ops_body.extend_from_slice(&(5u32 | (pad_count << 16)).to_le_bytes());
    ops_body.extend_from_slice(&pad_count.to_le_bytes());
    ops_body.resize(ops_body.len() + (pad_count as usize) * 4, 0u8);
    ops_body.extend_from_slice(&(3u32 | (1u32 << 16)).to_le_bytes());
    ops_body.extend_from_slice(&1u32.to_le_bytes());
    ops_body.extend_from_slice(&600_000u32.to_le_bytes());

    let total_words = 6 + (ops_body.len() / 4) as u32;
    let mut nve2 = minimal_nve2_header(1, 2, total_words);
    nve2.extend_from_slice(&ops_body);

    // Direct bare-NodeStore WireDecoder check BEFORE any StoreWireReplica::reset():
    // proves rejection happens before growing `nodes` capacity.
    let mut bare_node = NodeStore::replica();
    let mut bare_dec = WireDecoder::default();
    assert_eq!(
        bare_dec.apply(&mut bare_node.tx(), &nve2),
        Err(nve_store::wire::WireError::Budget)
    );
    assert_eq!(
        bare_node.capacity(),
        1,
        "direct WireDecoder rejection must not grow bare NodeStore::nodes"
    );

    let hostile = sign_frame0_payload(&nve2, 1, meta);
    let path = temp_file("hostile_sparse_node.nve2");
    std::fs::write(&path, &hostile).expect("write hostile node frame0");
    let mut replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        core_realization_order(),
    );
    assert!(matches!(
        Frame0File::boot_into_replica(&path, meta, &mut replica),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert_eq!(replica.store().stats().live_nodes, 0);
    assert_eq!(replica.store().capacity(), 1);
    assert_eq!(replica.backend().commits, 0);
    assert!(
        !path.exists(),
        "hostile WireError::Budget frame0 must be auto-removed as Corrupt"
    );

    // Hostile padded ResourceId(400_000) (~22.4 MiB > 17 MiB) using a REAL registered codec
    // (`ImageSrc`) on disk via `Frame0File::boot_into_replica` -> rejected by SessionBudget
    // (non-vacuous: same codec/blob at ResourceId(1) succeeds!), slots stay 1, file deleted.
    use nve_components::tables::ImageSrc;
    use nve_store::Resource;
    nve_store::register_codec::<ImageSrc>();
    let mut valid_blob = Vec::new();
    ImageSrc::Url("https://example.invalid/a.png".into()).encode(&mut valid_blob);
    let build_res_nve2 = |rid: u32, with_pad: bool| {
        let mut def = Vec::new();
        def.extend_from_slice(&rid.to_le_bytes());
        def.extend_from_slice(&ImageSrc::KIND.lo().to_le_bytes());
        def.extend_from_slice(&ImageSrc::KIND.hi().to_le_bytes());
        def.extend_from_slice(&0u32.to_le_bytes());
        def.extend_from_slice(&(valid_blob.len() as u32).to_le_bytes());
        def.extend_from_slice(&valid_blob);
        while !def.len().is_multiple_of(4) {
            def.push(0);
        }
        let mut ops = Vec::new();
        let mut op_count = 0u32;
        if with_pad {
            ops.extend_from_slice(&(5u32 | (pad_count << 16)).to_le_bytes());
            ops.extend_from_slice(&pad_count.to_le_bytes());
            ops.resize(ops.len() + (pad_count as usize) * 4, 0u8);
            op_count += 1;
        }
        ops.extend_from_slice(&(4u32 | (1u32 << 16)).to_le_bytes());
        ops.extend_from_slice(&((def.len() / 4) as u32).to_le_bytes());
        ops.extend_from_slice(&def);
        op_count += 1;
        let mut nve2 = minimal_nve2_header(1, op_count, 6 + (ops.len() / 4) as u32);
        nve2.extend_from_slice(&ops);
        nve2
    };
    // Sanity check: identical registered ImageSrc payload at ResourceId(1) boots cleanly.
    let valid_res_frame0 = sign_frame0_payload(&build_res_nve2(1, false), 0, meta);
    Frame0File::boot_bytes_into_replica(&valid_res_frame0, meta, &mut replica)
        .expect("registered ImageSrc payload at ResourceId(1) must decode cleanly");

    // Direct bare-NodeStore WireDecoder check BEFORE any StoreWireReplica::reset():
    // proves neither `nodes` nor `resources` slots grew on the rejected hostile payload.
    let hostile_res_nve2 = build_res_nve2(400_000, true);
    let mut bare_res = NodeStore::replica();
    let mut bare_res_dec = WireDecoder::default();
    assert_eq!(
        bare_res_dec.apply(&mut bare_res.tx(), &hostile_res_nve2),
        Err(nve_store::wire::WireError::Budget)
    );
    assert_eq!(
        bare_res.capacity(),
        1,
        "direct WireDecoder resource rejection must keep bare NodeStore::nodes == 1"
    );
    assert_eq!(
        bare_res.resources().slots(),
        1,
        "direct WireDecoder resource rejection must keep bare resources().slots() == 1"
    );

    let hostile_res = sign_frame0_payload(&hostile_res_nve2, 0, meta);
    let res_path = temp_file("hostile_sparse_res.nve2");
    std::fs::write(&res_path, &hostile_res).expect("write hostile resource frame0");
    assert!(matches!(
        Frame0File::boot_into_replica(&res_path, meta, &mut replica),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert_eq!(
        replica.store().resources().slots(),
        1,
        "rejected resource frame0 must keep resources().slots() == 1"
    );
    assert!(
        !res_path.exists(),
        "hostile resource WireError::Budget frame0 must be auto-removed as Corrupt"
    );
}

#[test]
fn frame0_budget_overrun_maps_to_corrupt_and_removes_hostile_disk_file() {
    use nve_store::NveTable;

    fn encode_decl(desc: &nve_abi::table::TableDesc, wire_slot: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&desc.key.lo().to_le_bytes());
        b.extend_from_slice(&desc.key.hi().to_le_bytes());
        b.extend_from_slice(&(desc.schema_hash as u32).to_le_bytes());
        b.extend_from_slice(&((desc.schema_hash >> 32) as u32).to_le_bytes());
        let packed = (wire_slot as u32)
            | ((desc.storage as u32) << 8)
            | ((desc.flags as u32) << 16)
            | ((desc.defaults.len() as u32) << 24);
        b.extend_from_slice(&packed.to_le_bytes());
        b.extend_from_slice(&desc.str_words.to_le_bytes());
        b.extend_from_slice(&desc.resource_words.to_le_bytes());
        for &d in desc.defaults {
            b.extend_from_slice(&d.to_le_bytes());
        }
        b.extend_from_slice(desc.word_effects);
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
        b.extend_from_slice(&(desc.name.len() as u32).to_le_bytes());
        b.extend_from_slice(desc.name.as_bytes());
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
        b
    }

    let _ = (<Layout as NveTable>::slot(), <Visual as NveTable>::slot());
    let slot_bytes_per_node =
        (<Layout as NveTable>::DESC.words() + <Visual as NveTable>::DESC.words() + 4)
            * size_of::<u32>()
            + size_of::<nve_store::NodeRow>();
    assert!(slot_bytes_per_node > 64);
    let n_nodes: u32 = ((18 * 1024 * 1024) / (slot_bytes_per_node - 64)) as u32;

    let mut decls = encode_decl(&<Layout as NveTable>::DESC, 0);
    decls.extend_from_slice(&encode_decl(&<Visual as NveTable>::DESC, 1));

    let mut ops = Vec::new();
    // Op 1: DeclareTables (opcode=1, count=2)
    ops.extend_from_slice(&(1u32 | (2u32 << 16)).to_le_bytes());
    ops.extend_from_slice(&((decls.len() / 4) as u32).to_le_bytes());
    ops.extend_from_slice(&decls);
    // Op 2: CreateNodes (opcode=3, extended count=n_nodes)
    ops.extend_from_slice(&(3u32 | (0xFFFFu32 << 16)).to_le_bytes());
    ops.extend_from_slice(&(1u32 + n_nodes).to_le_bytes());
    ops.extend_from_slice(&n_nodes.to_le_bytes());
    for id in 1..=n_nodes {
        ops.extend_from_slice(&id.to_le_bytes());
    }
    // Op 3 & 4: Patch (opcode=6) on Layout (wire_slot=0) and Visual (wire_slot=1) for NodeId::ROOT.
    for wire_slot in [0u32, 1u32] {
        ops.extend_from_slice(&(6u32 | (wire_slot << 8) | (1u32 << 16)).to_le_bytes());
        ops.extend_from_slice(&2u32.to_le_bytes());
        ops.extend_from_slice(&NodeId::ROOT.raw().to_le_bytes());
        ops.extend_from_slice(&0u32.to_le_bytes());
    }

    let mut nve2 = minimal_nve2_header(1, 4, 6 + (ops.len() / 4) as u32);
    nve2.extend_from_slice(&ops);

    let meta = Frame0Meta::default_core();
    let envelope = sign_frame0_payload(&nve2, n_nodes, meta);
    let path = temp_file("budget_overrun.nve2");
    std::fs::write(&path, &envelope).expect("write budget overrun frame0");

    let mut replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        core_realization_order(),
    );
    assert!(matches!(
        Frame0File::boot_into_replica(&path, meta, &mut replica),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert_eq!(replica.store().stats().live_nodes, 0);
    assert_eq!(replica.backend().commits, 0);
    assert!(replica.needs_keyframe());
    assert!(
        !path.exists(),
        "deterministic WireError::Budget overrun must auto-delete hostile frame0.nve2"
    );
}

#[test]
fn frame0_churned_keyframe_roundtrips_cleanly() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut ids = Vec::with_capacity(2_500);
    {
        let mut tx = store.tx();
        for _ in 0..2_500 {
            ids.push(tx.create());
        }
        tx.commit_into(&mut changes);
    }
    let survivor = *ids.last().expect("survivor node");
    assert!(survivor.index() >= 2_500);
    {
        let mut tx = store.tx();
        for &id in &ids[..ids.len() - 1] {
            tx.remove(id);
        }
        tx.set_children(NodeId::ROOT, &[survivor]);
        tx.edit::<Visual>(survivor)
            .background(Rgba::rgb(12, 34, 56));
        tx.edit::<TextContent>(survivor).text("survived-churn");
        tx.commit_into(&mut changes);
    }

    let meta = Frame0Meta::default_core();
    let path = temp_file("churned_keyframe.nve2");
    let _ = std::fs::remove_file(&path);
    Frame0File::save(&path, &store, meta).expect("save churned frame0");

    let mut replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        core_realization_order(),
    );
    let (footer, _stats) =
        Frame0File::boot_into_replica(&path, meta, &mut replica).expect("boot churned frame0");
    assert_eq!(footer.node_count, 2);
    assert_eq!(replica.store().stats().live_nodes, 2);
    assert!(replica.store().is_live(survivor));
    assert_eq!(
        replica.store().row::<TextContent>(survivor).text(),
        "survived-churn"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn frame0_keyframe_resets_replica_holding_only_non_root_nodes() {
    let mut replica = StoreWireReplica::new(
        CountingBackend {
            commits: 0,
            last_live: 0,
        },
        core_realization_order(),
    );
    // Build a frame == 1 packet containing ONLY non-root NodeId(5) (no ROOT).
    let mut ops = Vec::new();
    ops.extend_from_slice(&(3u32 | (1u32 << 16)).to_le_bytes());
    ops.extend_from_slice(&1u32.to_le_bytes());
    ops.extend_from_slice(&5u32.to_le_bytes());
    let mut non_root_f1 = minimal_nve2_header(1, 1, 6 + (ops.len() / 4) as u32);
    non_root_f1.extend_from_slice(&ops);
    replica
        .submit_nve2_frame(&non_root_f1)
        .expect("submit non-root frame 1");
    assert!(replica.store().is_live(NodeId(5)));
    assert!(!replica.store().is_live(NodeId::ROOT));

    // Submitting a fresh frame == 1 keyframe with only ROOT must wipe non-root NodeId(5).
    let root_store = build_store(2);
    let mut root_f1 = Vec::new();
    WireEncoder::default().encode_full(&root_store, &mut root_f1);
    replica
        .submit_nve2_frame(&root_f1)
        .expect("fresh keyframe resets non-root live replica");
    assert!(replica.store().is_live(NodeId::ROOT));
    assert!(!replica.store().is_live(NodeId(5)));
}
