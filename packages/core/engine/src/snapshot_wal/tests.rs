use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::snapshot::{write_snapshot_atomic, MmapSnapshot, RelocatableStateArena};

use super::compact::WAL_TMP_COUNTER;
use super::*;

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let seq = WAL_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve_wal_test_{label}_{}_{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn append_and_replay_all_record_kinds_in_order() {
    let dir = TempDirGuard::new("append_replay");
    let wal_path = dir.path().join("app.nvewal");
    let base_checksum = 0x1234_5678_9abc_def0;

    {
        let mut wal = SnapshotWal::open_or_create_with_durability(
            &wal_path,
            42,
            base_checksum,
            WalDurability::SyncData,
        )
        .unwrap();
        assert_eq!(wal.record_count(), 0);
        assert_eq!(wal.next_seq(), 1);
        assert_eq!(wal.file_len(), WAL_HEADER_SIZE as u64);

        assert_eq!(
            wal.append_delta(WalRecordKind::StateDelta, b"delta-one")
                .unwrap(),
            1
        );
        assert_eq!(
            wal.append_delta(WalRecordKind::Frame0Delta, b"frame0-patch")
                .unwrap(),
            2
        );
        assert_eq!(
            wal.append_delta(WalRecordKind::Checkpoint, b"").unwrap(),
            3
        );
        assert_eq!(wal.record_count(), 3);
        assert_eq!(wal.next_seq(), 4);
    }

    // Reopen and verify exact replay order + payloads.
    let reopened = SnapshotWal::open_or_create(&wal_path, 42, base_checksum).unwrap();
    assert_eq!(reopened.record_count(), 3);
    assert_eq!(reopened.recovered_truncated_bytes(), 0);

    let mut seen = Vec::new();
    let count = reopened
        .replay_into(|rec| -> Result<(), std::convert::Infallible> {
            seen.push((rec.seq, rec.kind, rec.payload.to_vec()));
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 3);
    assert_eq!(
        seen,
        vec![
            (1, WalRecordKind::StateDelta, b"delta-one".to_vec()),
            (2, WalRecordKind::Frame0Delta, b"frame0-patch".to_vec()),
            (3, WalRecordKind::Checkpoint, Vec::new()),
        ]
    );

    // Mismatched base schema or checksum must be rejected.
    assert!(matches!(
        SnapshotWal::open_or_create(&wal_path, 99, base_checksum),
        Err(WalError::BaseSnapshotMismatch { .. })
    ));
    assert!(matches!(
        SnapshotWal::open_or_create(&wal_path, 42, base_checksum ^ 1),
        Err(WalError::BaseSnapshotMismatch { .. })
    ));
}

#[test]
fn torn_tail_partial_writes_recover_cleanly_at_every_cut_point() {
    let dir = TempDirGuard::new("torn_tail");
    let base_checksum = 0xfeed_face_cafe_babe;

    // Build a reference 4-record WAL.
    let ref_path = dir.path().join("ref.nvewal");
    let mut wal = SnapshotWal::open_or_create(&ref_path, 3, base_checksum).unwrap();
    wal.append_delta(WalRecordKind::StateDelta, b"rec-1-alpha")
        .unwrap();
    wal.append_delta(WalRecordKind::StateDelta, b"rec-2-beta-payload")
        .unwrap();
    wal.append_delta(WalRecordKind::Frame0Delta, b"rec-3-gamma")
        .unwrap();
    let boundary_after_3 = wal.file_len() as usize;
    wal.append_delta(WalRecordKind::StateDelta, b"rec-4-torn-tail-record")
        .unwrap();
    let full_bytes = fs::read(&ref_path).unwrap();
    drop(wal);

    // Test every possible byte-level truncation inside record 4 (header, payload, checksum).
    for cut in (boundary_after_3 + 1)..full_bytes.len() {
        let p = dir.path().join(format!("cut_{cut}.nvewal"));
        fs::write(&p, &full_bytes[..cut]).unwrap();

        let mut recovered = SnapshotWal::open_or_create(&p, 3, base_checksum).unwrap();
        assert_eq!(recovered.record_count(), 3, "cut={cut}");
        assert_eq!(recovered.next_seq(), 4, "cut={cut}");
        assert_eq!(recovered.file_len(), boundary_after_3 as u64, "cut={cut}");
        assert_eq!(
            recovered.recovered_truncated_bytes(),
            (cut - boundary_after_3) as u64,
            "cut={cut}"
        );

        // Subsequent appends continue cleanly at seq=4.
        assert_eq!(
            recovered
                .append_delta(WalRecordKind::StateDelta, b"rec-4-healed")
                .unwrap(),
            4
        );
        let recs = recovered.read_all_records().unwrap();
        assert_eq!(recs.len(), 4);
        assert_eq!(recs[3].seq, 4);
        assert_eq!(recs[3].payload, b"rec-4-healed");
    }

    // Also test a torn final record where OS extended file length with corrupt tail checksum.
    let bad_tail_path = dir.path().join("bad_tail_checksum.nvewal");
    let mut corrupted_tail = full_bytes.clone();
    let last_idx = corrupted_tail.len() - 1;
    corrupted_tail[last_idx] ^= 0x5A;
    fs::write(&bad_tail_path, &corrupted_tail).unwrap();

    let recovered = SnapshotWal::open_or_create(&bad_tail_path, 3, base_checksum).unwrap();
    assert_eq!(recovered.record_count(), 3);
    assert_eq!(
        recovered.recovered_truncated_bytes(),
        (full_bytes.len() - boundary_after_3) as u64
    );
}

#[test]
fn corrupted_middle_record_is_rejected() {
    let dir = TempDirGuard::new("corrupt_middle");
    let wal_path = dir.path().join("mid.nvewal");
    let base_checksum = 0x0bad_c0de_1122_3344;

    let mut wal = SnapshotWal::open_or_create(&wal_path, 1, base_checksum).unwrap();
    wal.append_delta(WalRecordKind::StateDelta, b"first-record")
        .unwrap();
    let rec2_offset = wal.file_len() as usize;
    wal.append_delta(WalRecordKind::StateDelta, b"second-record-payload")
        .unwrap();
    wal.append_delta(WalRecordKind::StateDelta, b"third-record")
        .unwrap();
    drop(wal);

    let original = fs::read(&wal_path).unwrap();

    // 1. Flip a byte inside record 2's payload -> CorruptRecordChecksum.
    let payload_corrupt = dir.path().join("mid_payload.nvewal");
    let mut bytes = original.clone();
    bytes[rec2_offset + WAL_RECORD_HEADER_SIZE + 2] ^= 0xFF;
    fs::write(&payload_corrupt, &bytes).unwrap();
    assert!(matches!(
        SnapshotWal::open_or_create(&payload_corrupt, 1, base_checksum),
        Err(WalError::CorruptRecordChecksum { seq: 2, .. })
    ));

    // 2. Corrupt record 2's kind byte -> InvalidRecordKind.
    let kind_corrupt = dir.path().join("mid_kind.nvewal");
    let mut bytes = original.clone();
    bytes[rec2_offset + 8] = 99;
    fs::write(&kind_corrupt, &bytes).unwrap();
    assert!(matches!(
        SnapshotWal::open_or_create(&kind_corrupt, 1, base_checksum),
        Err(WalError::InvalidRecordKind { raw_kind: 99, .. })
    ));

    // 3. Corrupt record 2's sequence number -> NonSequentialRecord.
    let seq_corrupt = dir.path().join("mid_seq.nvewal");
    let mut bytes = original.clone();
    bytes[rec2_offset..rec2_offset + 8].copy_from_slice(&77u64.to_le_bytes());
    fs::write(&seq_corrupt, &bytes).unwrap();
    assert!(matches!(
        SnapshotWal::open_or_create(&seq_corrupt, 1, base_checksum),
        Err(WalError::NonSequentialRecord {
            expected_seq: 2,
            actual_seq: 77,
            ..
        })
    ));
}

#[test]
fn threshold_compaction_roundtrips_mmap_snapshot_and_rotates_wal() {
    let dir = TempDirGuard::new("compact");
    let snap_path = dir.path().join("app.nvesnap");
    let wal_path = dir.path().join("app.nvewal");

    let mut arena0 = RelocatableStateArena::new();
    arena0.insert_i64("count", 10);
    let initial_snap = write_snapshot_atomic(
        &snap_path,
        5,
        0,
        b"FRAME0_V1",
        &arena0.finish(),
        b"meta-v1",
    )
    .unwrap();

    let mut wal =
        SnapshotWal::open_or_create(&wal_path, 5, initial_snap.checksum_xxh64).unwrap();
    assert!(!wal.should_compact(512, 4));

    for i in 1..=4i64 {
        wal.append_delta(WalRecordKind::StateDelta, &i.to_le_bytes())
            .unwrap();
    }
    assert!(wal.should_compact(0, 4));
    assert!(wal.should_compact(wal.file_len(), 0));

    // Fold deltas + compact atomically into a fresh MmapSnapshot.
    let mut folded_count = 10i64;
    wal.replay_into(|rec| -> Result<(), std::convert::Infallible> {
        if rec.kind == WalRecordKind::StateDelta {
            folded_count += i64::from_le_bytes(rec.payload.try_into().unwrap());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(folded_count, 20);

    let mut arena1 = RelocatableStateArena::new();
    arena1.insert_i64("count", folded_count);
    arena1.insert_str("status", "compacted");

    let (new_header, mut rotated_wal) = SnapshotWal::compact_into_snapshot(
        &snap_path,
        &wal_path,
        5,
        b"FRAME0_V2_COMPACTED",
        &arena1.finish(),
        b"meta-v2",
    )
    .unwrap();

    assert_ne!(new_header.checksum_xxh64, initial_snap.checksum_xxh64);
    assert_eq!(rotated_wal.record_count(), 0);
    assert_eq!(rotated_wal.next_seq(), 1);
    assert_eq!(rotated_wal.file_len(), WAL_HEADER_SIZE as u64);
    assert_eq!(
        rotated_wal.header().base_snapshot_checksum,
        new_header.checksum_xxh64
    );
    assert!(!rotated_wal.should_compact(512, 4));

    // MmapSnapshot::open must read the compacted snapshot with zero errors.
    let mapped = MmapSnapshot::open_with_schema(&snap_path, 5).unwrap();
    assert_eq!(mapped.checksum_xxh64(), new_header.checksum_xxh64);
    assert_eq!(mapped.frame0_command_buffer(), b"FRAME0_V2_COMPACTED");
    assert_eq!(mapped.meta_bytes(), b"meta-v2");
    let table = mapped.state_table().unwrap();
    assert_eq!(table.get_i64("count"), Some(20));
    assert_eq!(table.get_str("status"), Some("compacted"));

    // Further appends work seamlessly on the rotated WAL.
    assert_eq!(
        rotated_wal
            .append_delta(WalRecordKind::StateDelta, b"post-compact")
            .unwrap(),
        1
    );
    assert_eq!(rotated_wal.record_count(), 1);
}

#[test]
fn thousand_delta_append_replay_and_compact_performance() {
    let dir = TempDirGuard::new("perf_1000");
    let snap_path = dir.path().join("perf.nvesnap");
    let wal_path = dir.path().join("perf.nvewal");

    let initial_header =
        write_snapshot_atomic(&snap_path, 1, 0, b"FRAME0_INIT", b"state0", b"").unwrap();
    let mut wal =
        SnapshotWal::open_or_create(&wal_path, 1, initial_header.checksum_xxh64).unwrap();

    let t_append = Instant::now();
    for i in 0..1_000u64 {
        let mut payload = [0u8; 24];
        payload[0..8].copy_from_slice(&i.to_le_bytes());
        payload[8..16].copy_from_slice(&(i * 3).to_le_bytes());
        payload[16..24].copy_from_slice(&(i ^ 0xA5A5_5A5A).to_le_bytes());
        let seq = wal
            .append_delta(WalRecordKind::StateDelta, &payload)
            .unwrap();
        assert_eq!(seq, i + 1);
    }
    wal.sync_data().unwrap();
    let append_elapsed = t_append.elapsed();

    assert_eq!(wal.record_count(), 1_000);
    assert!(wal.should_compact(0, 1_000));

    let t_replay = Instant::now();
    let mut sum = 0u64;
    let replayed = wal
        .replay_into(|rec| -> Result<(), std::convert::Infallible> {
            let v = u64::from_le_bytes(rec.payload[0..8].try_into().unwrap());
            sum += v;
            Ok(())
        })
        .unwrap();
    let replay_elapsed = t_replay.elapsed();
    assert_eq!(replayed, 1_000);
    assert_eq!(sum, 999 * 1_000 / 2);

    let t_compact = Instant::now();
    let new_header = wal
        .compact(&snap_path, 1, b"FRAME0_1000", &sum.to_le_bytes(), b"")
        .unwrap();
    let compact_elapsed = t_compact.elapsed();

    assert_eq!(wal.record_count(), 0);
    assert_eq!(wal.file_len(), WAL_HEADER_SIZE as u64);
    assert_eq!(wal.header().base_snapshot_checksum, new_header.checksum_xxh64);

    let mapped = MmapSnapshot::open_with_schema(&snap_path, 1).unwrap();
    assert_eq!(mapped.state_bytes(), &sum.to_le_bytes());

    // Generous upper bounds that catch catastrophic regressions even on slow CI runners.
    assert!(
        append_elapsed.as_secs() < 5,
        "1,000 WAL appends took {append_elapsed:?}"
    );
    assert!(
        replay_elapsed.as_secs() < 5,
        "1,000 WAL replays took {replay_elapsed:?}"
    );
    assert!(
        compact_elapsed.as_secs() < 5,
        "WAL compaction took {compact_elapsed:?}"
    );
}
