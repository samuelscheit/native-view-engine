use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nve_storage::{
    KvStore, MemoryStorage, NativeStorage, StorageBackend, StorageEntry, StorageError,
    StorageWriteOp, MAX_BATCH_OPS, MAX_KEY_LEN, MAX_NAMESPACE_LEN, MAX_VALUE_LEN, NVK2_MAGIC,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn unique_temp_dir(label: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nve_storage_test_{label}_{}_{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

#[test]
fn nvk2_storage_persistence_atomic_batch_rollback_prefix_scan_and_compaction() {
    let root = unique_temp_dir("kv_persist");

    {
        let native = NativeStorage::with_root_dir(&root);
        let backend: &dyn StorageBackend = &native;
        let info = backend.open("app_state").unwrap();
        assert_eq!(info.key_count, 0);

        assert!(matches!(
            backend.open("../escape"),
            Err(StorageError::InvalidArg(_))
        ));
        assert!(matches!(backend.open(""), Err(StorageError::InvalidArg(_))));
        assert!(matches!(
            backend.put("app_state", b"", b"v"),
            Err(StorageError::InvalidArg(_))
        ));
        assert!(matches!(
            backend.get("app_state", b""),
            Err(StorageError::InvalidArg(_))
        ));

        backend.put("app_state", b"user:02", b"bob").unwrap();
        backend.put("app_state", b"user:01", b"alice_v1").unwrap();
        backend.put("app_state", b"user:01", b"alice_v2").unwrap();
        backend.put("app_state", b"user:03", b"charlie").unwrap();
        backend
            .put("app_state", b"meta:ver", &[0x00, 0xFF, 0x10, 0x20])
            .unwrap();
        assert!(backend.delete("app_state", b"user:03").unwrap());
        assert!(!backend.delete("app_state", b"user:03").unwrap());

        // Empty commit_batch is a no-op and does not append to WAL.
        let before_empty = backend.open("app_state").unwrap().log_bytes;
        backend.commit_batch("app_state", &[]).unwrap();
        assert_eq!(backend.open("app_state").unwrap().log_bytes, before_empty);

        backend
            .commit_batch(
                "app_state",
                &[
                    StorageWriteOp::Put {
                        key: b"user:04".to_vec(),
                        value: b"diana".to_vec(),
                    },
                    StorageWriteOp::Put {
                        key: b"user:05".to_vec(),
                        value: b"eve".to_vec(),
                    },
                    StorageWriteOp::Delete {
                        key: b"user:02".to_vec(),
                    },
                ],
            )
            .unwrap();

        let err = backend.commit_batch(
            "app_state",
            &[
                StorageWriteOp::Put {
                    key: b"user:99".to_vec(),
                    value: b"should_not_persist".to_vec(),
                },
                StorageWriteOp::Delete {
                    key: b"user:01".to_vec(),
                },
                StorageWriteOp::Put {
                    key: Vec::new(),
                    value: b"bad".to_vec(),
                },
            ],
        );
        assert!(matches!(err, Err(StorageError::InvalidArg(_))));
        assert_eq!(backend.get("app_state", b"user:99").unwrap(), None);
        assert_eq!(
            backend.get("app_state", b"user:01").unwrap().as_deref(),
            Some(b"alice_v2".as_slice())
        );

        let all_users = backend.scan_prefix("app_state", b"user:", 100).unwrap();
        assert_eq!(
            all_users,
            vec![
                StorageEntry {
                    key: b"user:01".to_vec(),
                    value: b"alice_v2".to_vec(),
                },
                StorageEntry {
                    key: b"user:04".to_vec(),
                    value: b"diana".to_vec(),
                },
                StorageEntry {
                    key: b"user:05".to_vec(),
                    value: b"eve".to_vec(),
                },
            ]
        );
        assert_eq!(
            backend.scan_prefix("app_state", b"user:", 2).unwrap().len(),
            2
        );
        assert!(backend
            .scan_prefix("app_state", b"user:", 0)
            .unwrap()
            .is_empty());

        for i in 0u32..40 {
            backend
                .put("app_state", b"hot:counter", &i.to_le_bytes())
                .unwrap();
        }
        let stats = backend.compact("app_state").unwrap();
        assert_eq!(stats.live_keys, 5);
        assert!(stats.bytes_after < stats.bytes_before);
        assert!(!root.join("app_state.nvk2.compact.tmp").exists());

        // Write AFTER compaction and verify persistence on reopen.
        backend
            .put("app_state", b"post:compact", b"survived")
            .unwrap();
        backend.delete("app_state", b"user:05").unwrap();
    }

    let raw = fs::read(root.join("app_state.nvk2")).unwrap();
    assert_eq!(&raw[0..4], &NVK2_MAGIC);

    {
        let native = NativeStorage::with_root_dir(&root);
        let info = native.open("app_state").unwrap();
        assert_eq!(info.key_count, 5);
        assert!(!info.recovered_torn_tail);
        assert_eq!(
            native.get("app_state", b"user:01").unwrap().as_deref(),
            Some(b"alice_v2".as_slice())
        );
        assert_eq!(native.get("app_state", b"user:02").unwrap(), None);
        assert_eq!(native.get("app_state", b"user:05").unwrap(), None);
        assert_eq!(
            native.get("app_state", b"post:compact").unwrap().as_deref(),
            Some(b"survived".as_slice())
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nvk2_crash_recovery_torn_tail_mid_log_corruption_and_valid_wal_fuzz() {
    let root = unique_temp_dir("kv_crash");
    let log_path = root.join("wal.nvk2");

    {
        let mut store = KvStore::open(&log_path).unwrap();
        store.put(b"k1", b"alpha").unwrap();
        store.put(b"k2", b"beta").unwrap();
        assert_eq!(store.get_ref(b"k1").unwrap(), Some(b"alpha".as_slice()));
        assert_eq!(store.get_ref(b"missing").unwrap(), None);
        assert!(matches!(
            store.get_ref(b""),
            Err(StorageError::InvalidArg(_))
        ));
        let bad = store.commit_batch(&[
            StorageWriteOp::Put {
                key: b"rolled_back".to_vec(),
                value: b"nope".to_vec(),
            },
            StorageWriteOp::Delete { key: Vec::new() },
        ]);
        assert!(matches!(bad, Err(StorageError::InvalidArg(_))));
        assert_eq!(store.get(b"rolled_back").unwrap(), None);
    }
    let clean_len = fs::metadata(&log_path).unwrap().len();
    {
        let mut f = OpenOptions::new().append(true).open(&log_path).unwrap();
        f.write_all(&[1, 0, 0, 0, 99, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0xDE, 0xAD])
            .unwrap();
        f.sync_all().unwrap();
    }

    {
        let native = NativeStorage::with_root_dir(&root);
        let info = native.open("wal").unwrap();
        assert!(info.recovered_torn_tail);
        assert_eq!(info.key_count, 2);
        assert_eq!(info.log_bytes, clean_len);
        native.put("wal", b"k3", b"gamma").unwrap();
    }

    // Corrupting the FINAL record at EOF is recovered as a torn tail.
    {
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&log_path)
            .unwrap();
        let len = f.seek(SeekFrom::End(0)).unwrap();
        f.seek(SeekFrom::Start(len - 14)).unwrap();
        let mut byte = [0u8; 1];
        f.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x01;
        f.seek(SeekFrom::Start(len - 14)).unwrap();
        f.write_all(&byte).unwrap();
        f.sync_all().unwrap();
    }
    {
        let store = KvStore::open(&log_path).unwrap();
        assert!(store.open_info().recovered_torn_tail);
        assert_eq!(store.get(b"k3").unwrap(), None);
        assert_eq!(store.get_ref(b"k1").unwrap(), Some(b"alpha".as_slice()));
        assert_eq!(store.get_ref(b"k2").unwrap(), Some(b"beta".as_slice()));
    }

    // Mid-log corruption (bit flip in an earlier record when committed records follow) MUST
    // return Err(StorageError::Corrupt) rather than silently truncating committed data!
    {
        let mut mid_store = KvStore::in_memory();
        mid_store.put(b"r1", b"first").unwrap();
        mid_store.put(b"r2", b"second").unwrap();
        mid_store.put(b"r3", b"third").unwrap();
        let good = mid_store.memory_log_bytes().unwrap().to_vec();

        // 1. Corrupt payload byte in record 1 (after 16B header + 16B record header).
        let mut bad_mid_crc = good.clone();
        bad_mid_crc[16 + 16 + 4] ^= 0x01;
        assert!(
            matches!(
                KvStore::open_from_memory_log(bad_mid_crc),
                Err(StorageError::Corrupt(_))
            ),
            "mid-log checksum corruption must error instead of truncating committed records"
        );

        // 2. Corrupt payload_len in record 1 so it claims to extend past EOF before trailing NKCM.
        let mut bad_mid_len = good.clone();
        bad_mid_len[16 + 12..16 + 16].copy_from_slice(&999_999u32.to_le_bytes());
        assert!(
            matches!(
                KvStore::open_from_memory_log(bad_mid_len),
                Err(StorageError::Corrupt(_))
            ),
            "mid-log length corruption before committed tail must error"
        );

        // 3. Non-zero header reserved bytes (even with recomputed header CRC) must error.
        let mut bad_hdr_res = good.clone();
        bad_hdr_res[6] = 1;
        let crc = nve_hash::xxh64(&bad_hdr_res[0..8], 0);
        bad_hdr_res[8..16].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(
            KvStore::open_from_memory_log(bad_hdr_res),
            Err(StorageError::Corrupt(_))
        ));

        // 4. Non-zero record reserved byte with valid CRC + NKCM must error.
        let mut bad_rec_res = good;
        bad_rec_res[16 + 1] = 0x55;
        // Record 1: 16B hdr + (4+2 + 4+5 = 15B payload) => payload_end = 16 + 16 + 15 = 47.
        let rec1_crc = nve_hash::xxh64(&bad_rec_res[16..47], 0);
        bad_rec_res[47..55].copy_from_slice(&rec1_crc.to_le_bytes());
        assert!(matches!(
            KvStore::open_from_memory_log(bad_rec_res),
            Err(StorageError::Corrupt(_))
        ));
    }

    let bad_hdr_path = root.join("bad_hdr.nvk2");
    fs::write(
        &bad_hdr_path,
        b"NOPE\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
    )
    .unwrap();
    assert!(matches!(
        KvStore::open(&bad_hdr_path),
        Err(StorageError::Corrupt(_))
    ));

    // In-memory WAL recovery verifies BOTH log length AND recovered key/value contents.
    let mut mem_store = KvStore::in_memory();
    mem_store.put(b"m1", b"one").unwrap();
    let mut corrupted = mem_store.memory_log_bytes().unwrap().to_vec();
    let valid_len = corrupted.len();
    corrupted.extend_from_slice(&[1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 1, 2]);
    let recovered = KvStore::open_from_memory_log(corrupted).unwrap();
    assert!(recovered.open_info().recovered_torn_tail);
    assert_eq!(recovered.log_bytes() as usize, valid_len);
    assert_eq!(recovered.get(b"m1"), Ok(Some(b"one".to_vec())));
    assert_eq!(recovered.get_ref(b"m1"), Ok(Some(b"one".as_slice())));

    // Torn-tail recovery when torn final record's key/value contains `b"NKCM"` MUST recover cleanly
    // (both short <28B fragments and mid-payload truncations past the record header).
    {
        let mut nkcm_store = KvStore::in_memory();
        nkcm_store.put(b"committed", b"NKCM-inside-committed-val").unwrap();
        let committed_len = nkcm_store.log_bytes() as usize;
        nkcm_store
            .put(b"NKCM-key", b"prefix-NKCM-middle-NKCM-suffix")
            .unwrap();
        let full_log = nkcm_store.memory_log_bytes().unwrap().to_vec();

        // Cut every possible torn-tail point inside the second record (from +1 to full_len - 1),
        // including cuts that include `b"NKCM"` in both `< 28 B` and `>= 28 B` torn tails.
        for cut in (committed_len + 1)..full_log.len() {
            let torn = full_log[..cut].to_vec();
            let rec = KvStore::open_from_memory_log(torn).unwrap_or_else(|e| {
                panic!("torn final record containing NKCM cut at {cut} must recover, got {e:?}")
            });
            assert!(rec.open_info().recovered_torn_tail);
            assert_eq!(rec.log_bytes() as usize, committed_len);
            assert_eq!(
                rec.get_ref(b"committed").unwrap(),
                Some(b"NKCM-inside-committed-val".as_slice())
            );
            assert_eq!(rec.get_ref(b"NKCM-key").unwrap(), None);
        }

        // Also test a mid-log record whose NKCM marker is corrupted while followed by a genuinely
        // committed record -> MUST still return Err(StorageError::Corrupt).
        let mut mid_bad_marker = full_log;
        mid_bad_marker[committed_len - 1] ^= 0xFF;
        assert!(matches!(
            KvStore::open_from_memory_log(mid_bad_marker),
            Err(StorageError::Corrupt(_))
        ));
    }

    // Checksummed WAL record with an empty key (PUT, DELETE, and BATCH) MUST return
    // StorageError::Corrupt (never StorageError::InvalidArg).
    {
        let base_hdr = KvStore::in_memory().memory_log_bytes().unwrap().to_vec();
        let make_frame = |kind: u8, payload: &[u8]| -> Vec<u8> {
            let mut buf = base_hdr.clone();
            let start = buf.len();
            buf.push(kind);
            buf.extend_from_slice(&[0u8; 3]);
            buf.extend_from_slice(&1u64.to_le_bytes());
            buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            buf.extend_from_slice(payload);
            let crc = nve_hash::xxh64(&buf[start..], 0);
            buf.extend_from_slice(&crc.to_le_bytes());
            buf.extend_from_slice(&nve_storage::NVK2_COMMIT_MARKER);
            buf
        };
        // KIND_PUT with empty key (`len=0`, value=`b"v"`).
        let empty_put = make_frame(1, &[0, 0, 0, 0, 1, 0, 0, 0, b'v']);
        assert!(matches!(
            KvStore::open_from_memory_log(empty_put),
            Err(StorageError::Corrupt(_))
        ));
        // KIND_DELETE with empty key (`len=0`).
        let empty_del = make_frame(2, &[0, 0, 0, 0]);
        assert!(matches!(
            KvStore::open_from_memory_log(empty_del),
            Err(StorageError::Corrupt(_))
        ));
        // KIND_BATCH with 1 DELETE op having empty key (`count=1, op=2, key_len=0`).
        let empty_batch = make_frame(3, &[1, 0, 0, 0, 2, 0, 0, 0, 0]);
        assert!(matches!(
            KvStore::open_from_memory_log(empty_batch),
            Err(StorageError::Corrupt(_))
        ));
    }

    // Compaction failure (blocked temp path) leaves the live disk handle usable and uncorrupted.
    {
        let compact_dir = unique_temp_dir("kv_compact_fail");
        let db_path = compact_dir.join("db.nvk2");
        let tmp_blocker = compact_dir.join("db.nvk2.compact.tmp");
        let mut store = KvStore::open(&db_path).unwrap();
        store.put(b"a", b"1").unwrap();
        fs::create_dir_all(&tmp_blocker).unwrap();
        assert!(matches!(store.compact(), Err(StorageError::Io(_))));
        // Live store remains healthy and writable.
        store.put(b"b", b"2").unwrap();
        drop(store);
        let reopened = KvStore::open(&db_path).unwrap();
        assert_eq!(reopened.get_ref(b"a").unwrap(), Some(b"1".as_slice()));
        assert_eq!(reopened.get_ref(b"b").unwrap(), Some(b"2".as_slice()));
        let _ = fs::remove_dir_all(&compact_dir);
    }

    let mem = MemoryStorage::new();
    mem.open("session").unwrap();
    let large_val = vec![0xABu8; 8192 + 123];
    mem.put("session", b"blob:large", &large_val).unwrap();
    assert_eq!(
        mem.get("session", b"blob:large").unwrap().as_deref(),
        Some(large_val.as_slice())
    );

    // Deterministic WAL replay fuzzing mutating a REAL valid-NVK2-header WAL (PUT, DELETE, BATCH)
    // plus random truncations/bitflips never panics and preserves roundtrip consistency.
    let mut seed_store = KvStore::in_memory();
    seed_store.put(b"a", b"111").unwrap();
    seed_store.put(b"b", b"222").unwrap();
    seed_store.delete(b"a").unwrap();
    seed_store
        .commit_batch(&[
            StorageWriteOp::Put {
                key: b"c".to_vec(),
                value: b"333".to_vec(),
            },
            StorageWriteOp::Delete {
                key: b"b".to_vec(),
            },
        ])
        .unwrap();
    let valid_wal = seed_store.memory_log_bytes().unwrap().to_vec();

    let mut seed: u64 = 0x5704_A6E0_5EED_0008;
    let mut next_u64 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    // Every prefix truncation past the 16B header must replay cleanly without panic.
    for cut in 16..=valid_wal.len() {
        let mut slice = valid_wal[..cut].to_vec();
        let res = KvStore::open_from_memory_log(slice.clone());
        if let Ok(replayed) = res {
            let compacted = replayed.memory_log_bytes().unwrap().to_vec();
            let reopened = KvStore::open_from_memory_log(compacted).unwrap();
            assert_eq!(reopened.open_info().key_count, replayed.open_info().key_count);
        }
        // Single-byte flip at cut-1 inside valid header/records.
        if cut > 0 {
            slice[cut - 1] ^= 0xFF;
            let _ = KvStore::open_from_memory_log(slice);
        }
    }

    // 512 mutated valid-header WALs (records & batches) + random noise.
    for i in 0..512u32 {
        let buf = if i % 4 == 0 {
            let len = (next_u64() as usize) % 96;
            (0..len).map(|_| next_u64() as u8).collect()
        } else {
            let mut mutated = valid_wal.clone();
            let flips = 1 + ((next_u64() as usize) % 3);
            for _ in 0..flips {
                // Keep NVK2 header intact on most rounds so record/batch decoding is stressed.
                let start = if i % 4 == 1 { 0 } else { 16 };
                let idx = start + ((next_u64() as usize) % (mutated.len() - start));
                mutated[idx] ^= (next_u64() as u8) | 1;
            }
            if i % 4 == 3 {
                let cut = 16 + ((next_u64() as usize) % (mutated.len() - 15));
                mutated.truncate(cut);
            }
            mutated
        };
        let _ = KvStore::open_from_memory_log(buf);
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nvk2_limits_windows_reserved_namespaces_and_multithreaded_access() {
    let root = unique_temp_dir("kv_limits");
    let native = Arc::new(NativeStorage::with_root_dir(&root));
    let memory = Arc::new(MemoryStorage::new());

    // 1. Namespace length limit + trailing dot + Windows reserved names (case-insensitive + stems).
    let max_ns = "a".repeat(MAX_NAMESPACE_LEN);
    let over_ns = "a".repeat(MAX_NAMESPACE_LEN + 1);
    assert!(memory.open(&max_ns).is_ok());
    assert!(matches!(
        memory.open(&over_ns),
        Err(StorageError::InvalidArg(_))
    ));
    for bad in [
        "trailing.",
        "CON",
        "con",
        "Prn",
        "AUX",
        "nul",
        "con.db",
        "NUL.state",
        "COM1",
        "com9",
        "com5.log",
        "LPT1",
        "lpt9.dat",
    ] {
        assert!(
            matches!(memory.open(bad), Err(StorageError::InvalidArg(_))),
            "namespace `{bad}` must be rejected"
        );
    }
    for ok in ["console", "COM0", "LPT0", "nullable", "app.v1"] {
        assert!(memory.open(ok).is_ok(), "namespace `{ok}` must be accepted");
    }

    // 2. MAX_KEY_LEN, MAX_VALUE_LEN, MAX_BATCH_OPS boundaries.
    let mut kv = KvStore::in_memory();
    let max_key = vec![b'k'; MAX_KEY_LEN];
    let over_key = vec![b'k'; MAX_KEY_LEN + 1];
    kv.put(&max_key, b"ok").unwrap();
    assert_eq!(kv.get_ref(&max_key).unwrap(), Some(b"ok".as_slice()));
    assert!(matches!(
        kv.put(&over_key, b"nope"),
        Err(StorageError::InvalidArg(_))
    ));
    assert!(matches!(
        kv.get(&over_key),
        Err(StorageError::InvalidArg(_))
    ));
    assert!(matches!(
        memory.scan_prefix("app.v1", &over_key, 10),
        Err(StorageError::InvalidArg(_))
    ));

    let over_val = vec![0u8; MAX_VALUE_LEN + 1];
    assert!(matches!(
        kv.put(b"v", &over_val),
        Err(StorageError::InvalidArg(_))
    ));

    let over_batch: Vec<StorageWriteOp> = (0..=MAX_BATCH_OPS)
        .map(|i| StorageWriteOp::Delete {
            key: i.to_le_bytes().to_vec(),
        })
        .collect();
    assert!(matches!(
        kv.commit_batch(&over_batch),
        Err(StorageError::InvalidArg(_))
    ));

    // 3. Multi-threaded concurrent access on both NativeStorage and MemoryStorage.
    let mut handles = Vec::new();
    for tid in 0..4u32 {
        let nat = Arc::clone(&native);
        let mem = Arc::clone(&memory);
        handles.push(std::thread::spawn(move || {
            for i in 0..25u32 {
                let k = format!("t{tid}:k{i:02}");
                let v = format!("v{tid}:{i}");
                nat.put("shared", k.as_bytes(), v.as_bytes()).unwrap();
                mem.put("shared", k.as_bytes(), v.as_bytes()).unwrap();
                assert_eq!(
                    nat.get("shared", k.as_bytes()).unwrap().as_deref(),
                    Some(v.as_bytes())
                );
                assert_eq!(
                    mem.get("shared", k.as_bytes()).unwrap().as_deref(),
                    Some(v.as_bytes())
                );
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(native.scan_prefix("shared", b"t", 200).unwrap().len(), 100);
    assert_eq!(memory.scan_prefix("shared", b"t", 200).unwrap().len(), 100);
    let compact_stats = native.compact("shared").unwrap();
    assert_eq!(compact_stats.live_keys, 100);

    let _ = fs::remove_dir_all(&root);
}
