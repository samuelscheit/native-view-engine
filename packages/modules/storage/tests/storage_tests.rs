use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nve_storage::{
    KvStore, MemoryStorage, NativeStorage, StorageBackend, StorageEntry, StorageError,
    StorageWriteOp, NVK2_MAGIC,
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
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nvk2_direct_kv_open_crash_recovery_torn_tail_checksum_memory_and_fuzz() {
    let root = unique_temp_dir("kv_crash");
    let log_path = root.join("wal.nvk2");

    {
        let mut store = KvStore::open(&log_path).unwrap();
        store.put(b"k1", b"alpha").unwrap();
        store.put(b"k2", b"beta").unwrap();
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
        let store = KvStore::open_file(2, &log_path).unwrap();
        assert!(store.open_info().recovered_torn_tail);
        assert_eq!(store.get(b"k3").unwrap(), None);
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

    // In-memory WAL recovery + large value round-trip via MemoryStorage.
    let mut mem_store = KvStore::in_memory();
    mem_store.put(b"m1", b"one").unwrap();
    let mut corrupted = mem_store.memory_log_bytes().unwrap().to_vec();
    let valid_len = corrupted.len();
    corrupted.extend_from_slice(&[1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 1, 2]);
    let recovered = KvStore::open_from_memory_log(5, corrupted).unwrap();
    assert!(recovered.open_info().recovered_torn_tail);
    assert_eq!(recovered.log_bytes() as usize, valid_len);

    let mem = MemoryStorage::new();
    mem.open("session").unwrap();
    let large_val = vec![0xABu8; 8192 + 123];
    mem.put("session", b"blob:large", &large_val).unwrap();
    assert_eq!(
        mem.get("session", b"blob:large").unwrap().as_deref(),
        Some(large_val.as_slice())
    );

    // Deterministic WAL replay fuzzing never panics on malformed/random inputs.
    let mut seed: u64 = 0x5704_A6E0_5EED_0008;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = KvStore::open_from_memory_log(99, buf);
    }

    let _ = fs::remove_dir_all(&root);
}
