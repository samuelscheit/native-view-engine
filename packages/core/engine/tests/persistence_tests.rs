//! Comprehensive tests and thread-local `CountingAlloc` O(1)-allocation guards for
//! `Frame0File` (`frame0.nve2`) and atomic `StateFile` (`state.nve`).

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nve_components::tables::{Interaction, Layout, TextContent, Typography, Visual};
use nve_engine::{
    BackendApplyStats, BackendError, Frame0File, Frame0Meta, Frame0Miss, PersistenceError,
    StateFile, StoreBackend, StoreWireReplica, Viewport, FRAME0_FOOTER_SIZE,
    STATE_FILE_HEADER_SIZE,
};
use nve_store::wire::{WireDecoder, WireEncoder};
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
    assert_eq!(out1.byte_len, STATE_FILE_HEADER_SIZE + b"gen-1-payload".len());
    let out2 = sf.save(b"gen-2-longer-payload").expect("save gen 2");
    assert_eq!(out2.generation, 2);
    assert_eq!(
        sf.load().expect("load gen 2").as_deref(),
        Some(b"gen-2-longer-payload".as_slice())
    );

    // Reopen with matching schema -> loaded_from_disk = true.
    let reopened = StateFile::open(&path, 7).expect("reopen matching schema");
    assert!(reopened.recovery_stats().loaded_from_disk);
    assert!(!reopened.recovery_stats().schema_mismatch_reset);
    assert_eq!(
        reopened.load().expect("load reopened").as_deref(),
        Some(b"gen-2-longer-payload".as_slice())
    );

    // Reopen with mismatched schema -> auto-reset and remove stale file.
    let mismatched = StateFile::open(&path, 8).expect("reopen mismatched schema");
    assert!(mismatched.recovery_stats().schema_mismatch_reset);
    assert!(!mismatched.recovery_stats().loaded_from_disk);
    assert_eq!(mismatched.load().expect("load after mismatch"), None);
    assert!(!path.exists());

    // Write valid state, then flip 1 bit in payload -> recovered_from_corrupt on reopen.
    let mut sf3 = StateFile::open(&path, 8).expect("reopen after reset");
    sf3.save(b"integrity-protected-state").expect("save");
    let mut raw = std::fs::read(&path).expect("read raw state");
    let last = raw.len() - 1;
    raw[last] ^= 0x01;
    std::fs::write(&path, &raw).expect("write bit-flipped state");

    assert!(matches!(
        StateFile::decode_envelope(&raw, 8),
        Err(PersistenceError::Corrupt(_))
    ));
    assert!(matches!(
        StateFile::decode_envelope(&raw[..STATE_FILE_HEADER_SIZE - 1], 8),
        Err(PersistenceError::Corrupt(_))
    ));

    let recovered = StateFile::open(&path, 8).expect("recover from bit flip");
    assert!(recovered.recovery_stats().recovered_from_corrupt);
    assert_eq!(recovered.load().expect("load recovered"), None);
    assert!(!path.exists());

    // Reset + in-memory mode.
    let mut mem = StateFile::new_in_memory(3);
    assert!(mem.path().is_none());
    mem.save(b"mem-state").expect("save mem");
    assert_eq!(mem.load().expect("load mem").as_deref(), Some(b"mem-state".as_slice()));
    mem.reset().expect("reset mem");
    assert_eq!(mem.load().expect("load reset mem"), None);
}

#[test]
fn frame0_file_roundtrip_mismatches_and_corrupt_auto_delete() {
    let store = build_store(32);
    let meta = Frame0Meta {
        table_schema_hash: nve_store::registered_tables_schema_hash(),
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

    // Footer metadata mismatches must reject cleanly WITHOUT deleting a valid file.
    let raw = std::fs::read(&path).expect("read frame0 bytes");
    assert!(matches!(
        Frame0File::validate_bytes(
            &raw,
            Frame0Meta {
                table_schema_hash: meta.table_schema_hash ^ 1,
                ..meta
            }
        ),
        Err(Frame0Miss::SchemaMismatch { .. })
    ));
    assert!(matches!(
        Frame0File::validate_bytes(
            &raw,
            Frame0Meta {
                app_build_id: meta.app_build_id ^ 1,
                ..meta
            }
        ),
        Err(Frame0Miss::BuildIdMismatch { .. })
    ));
    assert!(matches!(
        Frame0File::validate_bytes(
            &raw,
            Frame0Meta {
                env_hash: meta.env_hash ^ 1,
                ..meta
            }
        ),
        Err(Frame0Miss::EnvHashMismatch { .. })
    ));
    assert!(path.exists(), "metadata mismatch must not delete valid cache");

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
    assert!(!path.exists(), "payload-corrupt frame0 must be auto-deleted");

    // 3. Single bit-flip in footer metadata -> Corrupt + auto-deleted.
    let mut flip_meta = raw.clone();
    let meta_off = flip_meta.len() - FRAME0_FOOTER_SIZE + 10;
    flip_meta[meta_off] ^= 0x80;
    std::fs::write(&path, &flip_meta).expect("write footer meta flip");
    assert!(matches!(
        Frame0File::read_and_validate(&path, meta),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert!(!path.exists(), "footer-meta-corrupt frame0 must be auto-deleted");

    // 4. Single bit-flip in footer checksum -> Corrupt + auto-deleted.
    let mut flip_hash = raw;
    let last = flip_hash.len() - 1;
    flip_hash[last] ^= 0x04;
    std::fs::write(&path, &flip_hash).expect("write checksum flip");
    assert!(matches!(
        Frame0File::read_and_validate(&path, meta),
        Err(Frame0Miss::Corrupt(_))
    ));
    assert!(!path.exists(), "checksum-corrupt frame0 must be auto-deleted");
}

#[test]
fn frame0_decode_into_presized_store_has_o1_allocations_1k_vs_5k() {
    let store_1k = build_store(1_000);
    let store_5k = build_store(5_000);
    let meta = Frame0Meta {
        table_schema_hash: nve_store::registered_tables_schema_hash(),
        app_build_id: 42,
        env_hash: 99,
    };
    let bytes_1k = Frame0File::encode(&store_1k, meta);
    let bytes_5k = Frame0File::encode(&store_5k, meta);

    let decode_measure = |bytes: &[u8]| -> (usize, usize) {
        let mut target = NodeStore::replica();
        let mut decoder = WireDecoder::default();
        let mut changes = ChangeSet::default();
        let (live, allocs) = count_allocs(|| {
            let (footer, payload) =
                Frame0File::validate_bytes(bytes, meta).expect("valid frame0 bytes");
            target.reserve_for_frame0(footer.node_count as usize, payload.len());
            changes.reserve_for_frame0(footer.node_count as usize);
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
    assert!(
        allocs_5k <= allocs_1k + 8,
        "Frame-0 NVE2 decode allocations must be O(1) after pre-sizing: 1k={allocs_1k}, 5k={allocs_5k}"
    );
}
