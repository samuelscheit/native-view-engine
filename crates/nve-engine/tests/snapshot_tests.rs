//! Deep tests for Instant-Boot Frame-0 `CommandBuffer` freeze-dry + `mmap` zero-copy state snapshots.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use nve_abi::{
    ColorRgba, Command, CommandBufferBuilder, CommandDecoder, ComponentKind, Dimension, EdgeInsets,
    EventMask, FlexAlign, FlexDirection, FlexJustify, LayoutProps, RawPropValue, StyleProps,
};
use nve_engine::snapshot::{
    encode_snapshot, write_snapshot_atomic, MmapSnapshot, RelocatableStateArena,
    RelocatableTableView, RelocatableValue, SnapshotError, NVE_SNAP_FORMAT_VERSION, NVE_SNAP_MAGIC,
    SNAPSHOT_HEADER_SIZE,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(tag: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve-snapshot-test-{tag}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build_sample_frame0(surface_id: u32, frame_id: u64, item_count: u32) -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(surface_id, frame_id);
    b.create_node(1, ComponentKind::Root, None);
    b.create_node(2, ComponentKind::ScrollView, None);
    b.set_children(1, &[2]);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            flex_direction: FlexDirection::Column,
            justify_content: FlexJustify::FlexStart,
            align_items: FlexAlign::Stretch,
            gap: 8.0,
            padding: EdgeInsets::all(12.0),
            ..Default::default()
        },
    );

    let mut children = Vec::with_capacity(item_count as usize);
    for i in 0..item_count {
        let row_id = 10 + i * 2;
        let txt_id = row_id + 1;
        children.push(row_id);

        b.create_node(row_id, ComponentKind::Pressable, None);
        b.create_node(txt_id, ComponentKind::Text, None);
        b.set_children(row_id, &[txt_id]);
        b.set_text(txt_id, &format!("Instant-boot item #{i} — cached Frame-0"));
        b.set_style(
            row_id,
            &StyleProps {
                background: ColorRgba::rgba(15, 23, 42, 255),
                text_color: ColorRgba::WHITE,
                border_color: ColorRgba::rgb(56, 189, 248),
                border_width: 1.0,
                border_radius: 8.0,
                opacity: 1.0,
                font_size: 15.0,
                z_index: 0,
            },
        );
        b.set_event_mask(row_id, EventMask::PRESS);
        b.set_raw_prop(row_id, "rowIndex", &RawPropValue::I64(i as i64));
    }
    b.set_children(2, &children);
    b.commit(true);
    b.into_bytes()
}

fn assert_slice_is_inside_region(region: &[u8], inner: &[u8], label: &str) {
    let base = region.as_ptr() as usize;
    let end = base + region.len();
    let inner_start = inner.as_ptr() as usize;
    let inner_end = inner_start + inner.len();
    assert!(
        inner_start >= base && inner_end <= end,
        "{label} slice [{inner_start:#x}..{inner_end:#x}] is NOT inside mapped region [{base:#x}..{end:#x}]"
    );
}

// ============================================================================
// 1. Roundtrip atomic write -> mmap open -> zero-copy Frame-0 decode
// ============================================================================

#[test]
fn atomic_write_mmap_open_zero_copy_frame0_decode_and_alignment() {
    let dir = TempDirGuard::new("roundtrip");
    let snap_path = dir.path().join("app.nvesnap");

    let frame0 = build_sample_frame0(7, 42, 25);
    // Deliberately choose unaligned section lengths to verify 8-byte alignment padding.
    let state = vec![0xABu8; 37];
    let meta = b"viewport=393x852@3x;gen=7";

    let written_header =
        write_snapshot_atomic(&snap_path, 3, 0x0001_0002, &frame0, &state, meta)
            .expect("atomic snapshot write");

    // Ensure no leftover .tmp files exist in the directory.
    let dir_entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(dir_entries, vec!["app.nvesnap"]);

    let snap = MmapSnapshot::open(&snap_path).expect("mmap open");
    assert_eq!(snap.header(), written_header);
    assert_eq!(snap.format_version(), NVE_SNAP_FORMAT_VERSION);
    assert_eq!(snap.app_schema_version(), 3);
    assert_eq!(snap.flags(), 0x0001_0002);

    // Every section offset must be 8-byte aligned and start at/after header.
    assert_eq!(snap.header().frame0_offset % 8, 0);
    assert_eq!(snap.header().state_offset % 8, 0);
    assert_eq!(snap.header().meta_offset % 8, 0);
    assert_eq!(snap.header().frame0_offset as usize, SNAPSHOT_HEADER_SIZE);

    // Prove true zero-copy: returned slices point directly inside the mmap'd address range.
    let raw = snap.as_bytes();
    let mapped_frame0 = snap.frame0_command_buffer();
    let mapped_state = snap.state_bytes();
    let mapped_meta = snap.meta_bytes();

    assert_slice_is_inside_region(raw, mapped_frame0, "frame0_command_buffer");
    assert_slice_is_inside_region(raw, mapped_state, "state_bytes");
    assert_slice_is_inside_region(raw, mapped_meta, "meta_bytes");

    assert_eq!(mapped_frame0, frame0.as_slice());
    assert_eq!(mapped_state, state.as_slice());
    assert_eq!(mapped_meta, meta);

    // Feed mapped Frame-0 bytes directly into `CommandDecoder::decode_all` with zero copies.
    let (cmd_header, commands) = CommandDecoder::new(mapped_frame0)
        .expect("decode Frame-0 header")
        .decode_all()
        .expect("decode all Frame-0 commands");
    assert_eq!(cmd_header.surface_id, 7);
    assert_eq!(cmd_header.frame_id, 42);
    assert_eq!(cmd_header.op_count as usize, commands.len());
    assert!(matches!(
        commands.last(),
        Some(Command::Commit { sync_mount: true })
    ));

    // Also verify open_unchecked_checksum + explicit deferred verify_checksum().
    let fast_snap =
        MmapSnapshot::open_unchecked_checksum(&snap_path).expect("open_unchecked_checksum");
    assert_eq!(fast_snap.frame0_command_buffer(), frame0.as_slice());
    fast_snap.verify_checksum().expect("deferred checksum");

    // Verify in-memory `from_bytes` produces identical results.
    let encoded = encode_snapshot(3, 0x0001_0002, &frame0, &state, meta);
    assert_eq!(encoded.as_slice(), raw);
    let mem_snap = MmapSnapshot::from_bytes(encoded).expect("from_bytes");
    assert_eq!(mem_snap.frame0_command_buffer(), frame0.as_slice());
    assert_eq!(mem_snap.state_bytes(), state.as_slice());
    assert_eq!(mem_snap.meta_bytes(), meta);
}

#[test]
fn empty_sections_roundtrip_cleanly() {
    let dir = TempDirGuard::new("empty-sections");
    let snap_path = dir.path().join("empty.nvesnap");

    write_snapshot_atomic(&snap_path, 0, 0, &[], &[], &[]).expect("write empty sections");
    let snap = MmapSnapshot::open(&snap_path).expect("open empty sections");
    assert_eq!(snap.as_bytes().len(), SNAPSHOT_HEADER_SIZE);
    assert!(snap.frame0_command_buffer().is_empty());
    assert!(snap.state_bytes().is_empty());
    assert!(snap.meta_bytes().is_empty());
}

// ============================================================================
// 2. RelocatableStateArena (1,000 entries) -> mmap -> zero-copy borrowed lookup
// ============================================================================

#[test]
fn relocatable_state_arena_1000_entries_zero_copy_mmap_lookup() {
    let dir = TempDirGuard::new("relocatable-1000");
    let snap_path = dir.path().join("state1000.nvesnap");

    let mut arena = RelocatableStateArena::new();
    for i in 0..250usize {
        arena.insert_i64(format!("counter/{i:04}"), (i as i64) * 1_000_000 - 123_456);
        arena.insert_f64(format!("metric/{i:04}"), (i as f64) * 0.375 - 42.5);
        arena.insert_str(
            format!("title/{i:04}"),
            format!("Document #{i} — zero-copy 🚀"),
        );
        arena.insert_bytes(
            format!("blob/{i:04}"),
            vec![(i & 0xFF) as u8, ((i >> 4) & 0xFF) as u8, 0xCA, 0xFE],
        );
    }
    // Overwrite an existing key to verify last-write-wins deduplication.
    arena.insert_i64("counter/0000", i64::MIN);
    arena.insert_bool("flag/dark_mode", true);
    arena.insert_bool("flag/compact", false);

    assert_eq!(arena.len(), 1002);
    let state_blob = arena.finish();

    let frame0 = build_sample_frame0(1, 1, 10);
    write_snapshot_atomic(&snap_path, 11, 0, &frame0, &state_blob, b"meta-v11")
        .expect("write 1000-entry snapshot");

    let snap = MmapSnapshot::open(&snap_path).expect("mmap open");
    let table = snap.state_table().expect("open RelocatableTableView");
    assert_eq!(table.len(), 1002);
    assert!(!table.is_empty());
    table.validate_all().expect("full table validation");

    let mapped_region = snap.as_bytes();

    // Verify overwritten key + booleans.
    assert_eq!(table.get_i64("counter/0000"), Some(i64::MIN));
    assert_eq!(table.get_bool("flag/dark_mode"), Some(true));
    assert_eq!(table.get_bool("flag/compact"), Some(false));
    assert_eq!(table.get_str("counter/0000"), None); // wrong type returns None
    assert_eq!(table.get_i64("nonexistent/key"), None);

    // Verify all 1,000 entries and assert every borrowed &str and &[u8] points directly inside mmap!
    for i in 0..250usize {
        if i > 0 {
            let expected_i64 = (i as i64) * 1_000_000 - 123_456;
            assert_eq!(table.get_i64(&format!("counter/{i:04}")), Some(expected_i64));
        }

        let expected_f64 = (i as f64) * 0.375 - 42.5;
        assert_eq!(table.get_f64(&format!("metric/{i:04}")), Some(expected_f64));

        let expected_str = format!("Document #{i} — zero-copy 🚀");
        let borrowed_str = table
            .get_str(&format!("title/{i:04}"))
            .expect("borrowed str");
        assert_eq!(borrowed_str, expected_str);
        assert_slice_is_inside_region(mapped_region, borrowed_str.as_bytes(), "borrowed_str");

        let expected_blob = [(i & 0xFF) as u8, ((i >> 4) & 0xFF) as u8, 0xCA, 0xFE];
        let borrowed_bytes = table
            .get_bytes(&format!("blob/{i:04}"))
            .expect("borrowed bytes");
        assert_eq!(borrowed_bytes, &expected_blob);
        assert_slice_is_inside_region(mapped_region, borrowed_bytes, "borrowed_bytes");
    }

    // Verify iterator visits every entry without allocations.
    assert_eq!(table.iter().count(), 1002);
    assert!(matches!(
        table.get("flag/dark_mode"),
        Some(RelocatableValue::Bool(true))
    ));
}

#[test]
fn relocatable_state_arena_empty_and_corruption_guards() {
    let empty_bytes = RelocatableStateArena::new().finish();
    let empty_view = RelocatableTableView::from_bytes(&empty_bytes).expect("empty table view");
    assert!(empty_view.is_empty());
    assert_eq!(empty_view.len(), 0);
    assert_eq!(empty_view.get_i64("anything"), None);
    empty_view.validate_all().expect("empty table valid");

    // Truncated header.
    assert!(matches!(
        RelocatableTableView::from_bytes(&[0u8; 8]),
        Err(SnapshotError::InvalidRelocatableArena(_))
    ));

    // Bad magic.
    let mut bad_magic = empty_bytes.clone();
    bad_magic[0] ^= 0xFF;
    assert!(matches!(
        RelocatableTableView::from_bytes(&bad_magic),
        Err(SnapshotError::InvalidRelocatableArena(_))
    ));

    // Unsupported version.
    let mut bad_ver = empty_bytes.clone();
    bad_ver[8..12].copy_from_slice(&99u32.to_le_bytes());
    assert!(matches!(
        RelocatableTableView::from_bytes(&bad_ver),
        Err(SnapshotError::InvalidRelocatableArena(_))
    ));

    // Entry count exceeds buffer.
    let mut huge_count = empty_bytes.clone();
    huge_count[12..16].copy_from_slice(&10_000u32.to_le_bytes());
    assert!(matches!(
        RelocatableTableView::from_bytes(&huge_count),
        Err(SnapshotError::InvalidRelocatableArena(_))
    ));

    // Corrupt slot key offset -> lookup returns None safely, validate_all catches it.
    let mut arena = RelocatableStateArena::new();
    arena.insert_str("hello", "world");
    let mut corrupt_slot = arena.finish();
    // Overwrite key_offset (bytes 16+8..16+12) to point past EOF.
    corrupt_slot[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
    let view = RelocatableTableView::from_bytes(&corrupt_slot).expect("header still valid");
    assert_eq!(view.get_str("hello"), None);
    assert!(matches!(
        view.validate_all(),
        Err(SnapshotError::InvalidRelocatableArena(_))
    ));
}

// ============================================================================
// 3. Cold `MmapSnapshot::open()` + Frame-0 decode latency benchmark test
// ============================================================================

#[test]
fn measure_cold_mmap_open_and_frame0_decode_sub_millisecond() {
    let dir = TempDirGuard::new("bench-cold-open");
    let snap_path = dir.path().join("frame0_bench.nvesnap");

    let frame0 = build_sample_frame0(1, 1, 100);
    let mut arena = RelocatableStateArena::new();
    for i in 0..200usize {
        arena.insert_str(format!("item/{i:04}"), format!("Cached row payload {i}"));
    }
    let state = arena.finish();
    write_snapshot_atomic(&snap_path, 1, 0, &frame0, &state, b"viewport=1440x900@2x")
        .expect("write bench snapshot");

    // Measure first (cold) open + verified Frame-0 decode + zero-copy state lookup.
    let t0 = Instant::now();
    let snap = MmapSnapshot::open(&snap_path).expect("cold open");
    let t_open = t0.elapsed();

    let (_hdr, cmds) = CommandDecoder::new(snap.frame0_command_buffer())
        .expect("decode hdr")
        .decode_all()
        .expect("decode all");
    let table = snap.state_table().expect("table view");
    let sample = table.get_str("item/0123").expect("lookup item/0123");
    let t_total = t0.elapsed();

    // Measure unchecked-checksum open + decode as well.
    let t1 = Instant::now();
    let fast = MmapSnapshot::open_unchecked_checksum(&snap_path).expect("fast open");
    let t_fast_open = t1.elapsed();
    let _ = CommandDecoder::new(fast.frame0_command_buffer())
        .unwrap()
        .decode_all()
        .unwrap();
    let t_fast_total = t1.elapsed();

    assert_eq!(sample, "Cached row payload 123");
    assert!(cmds.len() > 500);

    eprintln!(
        "[snapshot timing] size={} B | verified: open={} us, open+decode+lookup={} us | unchecked: open={} us, open+decode={} us",
        snap.as_bytes().len(),
        t_open.as_micros(),
        t_total.as_micros(),
        t_fast_open.as_micros(),
        t_fast_total.as_micros(),
    );

    // Generous CI ceiling while verifying truly fast restore (< 50 ms even under debug builds/heavy load).
    assert!(
        t_total.as_millis() < 50,
        "cold mmap open + Frame-0 decode unexpectedly slow: {t_total:?}"
    );
}

// ============================================================================
// 4. Exhaustive edge & corruption tests
// ============================================================================

#[test]
fn rejects_missing_empty_truncated_bad_magic_and_bad_version() {
    let dir = TempDirGuard::new("errors-basic");

    // Missing file.
    let missing = dir.path().join("does_not_exist.nvesnap");
    let err = MmapSnapshot::open(&missing).unwrap_err();
    assert!(
        matches!(err, SnapshotError::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
        "expected NotFound, got {err:?}"
    );

    // 0-byte file (must not invoke mmap(len=0) / EINVAL!).
    let zero_path = dir.path().join("zero.nvesnap");
    std::fs::write(&zero_path, b"").unwrap();
    let err = MmapSnapshot::open(&zero_path).unwrap_err();
    assert!(
        matches!(
            err,
            SnapshotError::Truncated {
                expected: SNAPSHOT_HEADER_SIZE,
                actual: 0
            }
        ),
        "expected Truncated(0), got {err:?}"
    );

    // Truncated header (40 bytes < 80 bytes).
    let trunc_path = dir.path().join("trunc.nvesnap");
    std::fs::write(&trunc_path, &vec![0u8; 40]).unwrap();
    let err = MmapSnapshot::open(&trunc_path).unwrap_err();
    assert!(
        matches!(
            err,
            SnapshotError::Truncated {
                expected: SNAPSHOT_HEADER_SIZE,
                actual: 40
            }
        ),
        "got {err:?}"
    );

    // Bad magic.
    let mut bad_magic = encode_snapshot(1, 0, b"frame", b"state", b"meta");
    bad_magic[0] = b'X';
    let err = MmapSnapshot::from_bytes(bad_magic).unwrap_err();
    assert!(matches!(err, SnapshotError::InvalidMagic(m) if m != NVE_SNAP_MAGIC));

    // Wrong format version.
    let mut bad_ver = encode_snapshot(1, 0, b"frame", b"state", b"meta");
    bad_ver[8..12].copy_from_slice(&2u32.to_le_bytes());
    let err = MmapSnapshot::from_bytes(bad_ver).unwrap_err();
    assert!(matches!(err, SnapshotError::UnsupportedFormatVersion(2)));

    // Non-zero reserved field.
    let mut bad_reserved = encode_snapshot(1, 0, b"frame", b"state", b"meta");
    bad_reserved[20..24].copy_from_slice(&1u32.to_le_bytes());
    let err = MmapSnapshot::from_bytes(bad_reserved).unwrap_err();
    assert!(matches!(err, SnapshotError::InvalidReservedHeader(1)));
}

#[test]
fn rejects_bit_flip_checksum_mismatch_in_every_region() {
    let dir = TempDirGuard::new("bitrot");
    let base = encode_snapshot(5, 7, b"frame0-payload-data", b"state-payload-data", b"meta");

    // Flip a bit in: schema version (byte 12), flags (byte 16), checksum (byte 24),
    // header-tail (byte 40), and payload (last non-zero byte).
    let probe_offsets = [12usize, 16, 24, 40, SNAPSHOT_HEADER_SIZE, base.len() - 1];
    for (idx, &offset) in probe_offsets.iter().enumerate() {
        let mut corrupted = base.clone();
        corrupted[offset] ^= 0x01;
        let path = dir.path().join(format!("corrupt_{idx}.nvesnap"));
        std::fs::write(&path, &corrupted).unwrap();

        let err = MmapSnapshot::open(&path).unwrap_err();
        assert!(
            matches!(
                err,
                SnapshotError::ChecksumMismatch { .. }
                    | SnapshotError::SectionOutOfBounds { .. }
                    | SnapshotError::OverlappingSections { .. }
            ),
            "bit flip at offset {offset} must be rejected, got {err:?}"
        );
    }

    // Specifically verify a payload bit-flip yields ChecksumMismatch.
    let mut payload_flip = base.clone();
    payload_flip[SNAPSHOT_HEADER_SIZE + 3] ^= 0x80;
    let err = MmapSnapshot::from_bytes(payload_flip).unwrap_err();
    assert!(
        matches!(err, SnapshotError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {err:?}"
    );
}

#[test]
fn rejects_out_of_bounds_unaligned_overflowing_and_overlapping_sections() {
    let base = encode_snapshot(1, 0, b"12345678", b"abcdefgh", b"xyz");

    // 1. Unaligned frame0 offset (81 is not divisible by 8).
    let mut unaligned = base.clone();
    unaligned[32..40].copy_from_slice(&81u64.to_le_bytes());
    assert!(matches!(
        MmapSnapshot::from_bytes_unchecked_checksum(unaligned),
        Err(SnapshotError::UnalignedSection {
            section: "frame0",
            offset: 81
        })
    ));

    // 2. Section offset inside header (< 80, aligned to 8 -> 16).
    let mut inside_hdr = base.clone();
    inside_hdr[32..40].copy_from_slice(&16u64.to_le_bytes());
    assert!(matches!(
        MmapSnapshot::from_bytes_unchecked_checksum(inside_hdr),
        Err(SnapshotError::SectionOutOfBounds {
            section: "frame0",
            ..
        })
    ));

    // 3. Out-of-bounds length past EOF.
    let mut past_eof = base.clone();
    past_eof[56..64].copy_from_slice(&999_999u64.to_le_bytes());
    assert!(matches!(
        MmapSnapshot::from_bytes_unchecked_checksum(past_eof),
        Err(SnapshotError::SectionOutOfBounds {
            section: "state",
            ..
        })
    ));

    // 4. Overflowing `offset + len` (`u64::MAX`).
    let mut overflow = base.clone();
    overflow[64..72].copy_from_slice(&(SNAPSHOT_HEADER_SIZE as u64).to_le_bytes());
    overflow[72..80].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(
        MmapSnapshot::from_bytes_unchecked_checksum(overflow),
        Err(SnapshotError::SectionOutOfBounds {
            section: "meta",
            len: u64::MAX,
            ..
        })
    ));

    // 5. Overlapping sections: point `state` at `frame0`'s offset.
    let mut overlap = base.clone();
    let frame0_off = u64::from_le_bytes(overlap[32..40].try_into().unwrap());
    overlap[48..56].copy_from_slice(&frame0_off.to_le_bytes());
    overlap[56..64].copy_from_slice(&8u64.to_le_bytes());
    assert!(matches!(
        MmapSnapshot::from_bytes_unchecked_checksum(overlap),
        Err(SnapshotError::OverlappingSections {
            first: "frame0",
            second: "state"
        })
    ));
}

#[test]
fn detects_schema_version_mismatch_and_handles_atomic_overwrite() {
    let dir = TempDirGuard::new("schema-and-overwrite");
    let snap_path = dir.path().join("live.nvesnap");

    // Write v1 snapshot and hold an open mmap handle to it.
    let f1 = build_sample_frame0(1, 1, 3);
    write_snapshot_atomic(&snap_path, 1, 0, &f1, b"state-v1", b"meta-v1")
        .expect("write v1");

    let snap_v1 = MmapSnapshot::open_with_schema(&snap_path, 1).expect("open v1");
    assert_eq!(snap_v1.state_bytes(), b"state-v1");

    // Opening v1 while expecting schema v2 must return SchemaVersionMismatch.
    let err = MmapSnapshot::open_with_schema(&snap_path, 2).unwrap_err();
    assert!(matches!(
        err,
        SnapshotError::SchemaVersionMismatch {
            expected: 2,
            actual: 1
        }
    ));

    // Atomically overwrite the snapshot with v2 while v1 is still mapped (on Unix, inode persists).
    let f2 = build_sample_frame0(1, 2, 6);
    write_snapshot_atomic(&snap_path, 2, 0xFF, &f2, b"state-v2-longer-payload", b"meta-v2")
        .expect("atomic overwrite v2");

    // New reader sees v2 cleanly.
    let snap_v2 = MmapSnapshot::open_with_schema(&snap_path, 2).expect("open v2");
    assert_eq!(snap_v2.app_schema_version(), 2);
    assert_eq!(snap_v2.flags(), 0xFF);
    assert_eq!(snap_v2.frame0_command_buffer(), f2.as_slice());
    assert_eq!(snap_v2.state_bytes(), b"state-v2-longer-payload");
    assert_eq!(snap_v2.meta_bytes(), b"meta-v2");

    // Previously mapped v1 handle still safely reads v1 without SIGBUS or corruption.
    assert_eq!(snap_v1.app_schema_version(), 1);
    assert_eq!(snap_v1.frame0_command_buffer(), f1.as_slice());
    assert_eq!(snap_v1.state_bytes(), b"state-v1");

    // No leftover temp files.
    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, vec!["live.nvesnap"]);
}
