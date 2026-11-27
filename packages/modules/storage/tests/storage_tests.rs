use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nve_host_services::{
    FfiHostServices, HostCallbackBridge, MemoryHostServices, HostServiceRegistry, ServiceError,
    FFI_STACK_BUF_SIZE,
};
use nve_storage::{
    decode_commit_batch_request, decode_compact_response, decode_get_response, decode_key_request,
    decode_open_request, decode_open_response, decode_put_request, decode_scan_prefix_request,
    decode_scan_prefix_response, KvStore, MemoryStorage, NativeStorage, StorageClient,
    StorageEntry, StorageWriteOp, NVK2_MAGIC,
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
fn nvk1_storage_persistence_atomic_batch_rollback_prefix_scan_and_compaction() {
    let root = unique_temp_dir("kv_persist");

    {
        let native = HostServiceRegistry::new().with_handler(NativeStorage::with_root_dir(&root));
        let client = StorageClient::new(&native);
        let store = client.open("app_state").unwrap();
        assert_eq!(store.namespace(), "app_state");

        assert!(matches!(
            client.open("../escape"),
            Err(ServiceError::InvalidArg(_))
        ));
        assert!(matches!(client.open(""), Err(ServiceError::InvalidArg(_))));
        assert!(matches!(
            store.put(b"", b"v"),
            Err(ServiceError::InvalidArg(_))
        ));
        assert!(matches!(store.get(b""), Err(ServiceError::InvalidArg(_))));

        store.put(b"user:02", b"bob").unwrap();
        store.put(b"user:01", b"alice_v1").unwrap();
        store.put(b"user:01", b"alice_v2").unwrap();
        store.put(b"user:03", b"charlie").unwrap();
        store.put(b"meta:ver", &[0x00, 0xFF, 0x10, 0x20]).unwrap();
        assert!(store.delete(b"user:03").unwrap());
        assert!(!store.delete(b"user:03").unwrap());

        store
            .commit_batch(&[
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
            ])
            .unwrap();

        let err = store.commit_batch(&[
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
        ]);
        assert!(matches!(err, Err(ServiceError::InvalidArg(_))));
        assert_eq!(store.get(b"user:99").unwrap(), None);
        assert_eq!(
            store.get(b"user:01").unwrap().as_deref(),
            Some(b"alice_v2".as_slice())
        );

        let all_users = store.scan_prefix(b"user:", 100).unwrap();
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
        assert_eq!(store.scan_prefix(b"user:", 2).unwrap().len(), 2);
        assert!(store.scan_prefix(b"user:", 0).unwrap().is_empty());

        for i in 0u32..40 {
            store.put(b"hot:counter", &i.to_le_bytes()).unwrap();
        }
        let stats = store.compact().unwrap();
        assert_eq!(stats.live_keys, 5);
        assert!(stats.bytes_after < stats.bytes_before);
        assert!(!root.join("app_state.nvk1.compact.tmp").exists());
    }

    let raw = fs::read(root.join("app_state.nvk1")).unwrap();
    assert_eq!(&raw[0..4], &NVK2_MAGIC);

    {
        let native = HostServiceRegistry::new().with_handler(NativeStorage::with_root_dir(&root));
        let (store, info) = StorageClient::new(&native)
            .open_with_info("app_state")
            .unwrap();
        assert_eq!(info.key_count, 5);
        assert!(!info.recovered_torn_tail);
        assert_eq!(
            store.get(b"user:01").unwrap().as_deref(),
            Some(b"alice_v2".as_slice())
        );
        assert_eq!(store.get(b"user:02").unwrap(), None);
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nvk1_crash_recovery_torn_tail_checksum_corruption_c_abi_and_fuzz() {
    let root = unique_temp_dir("kv_crash");
    let log_path = root.join("wal.nvk1");

    {
        let mut store = KvStore::open_file(1, &log_path).unwrap();
        store.put(b"k1", b"alpha").unwrap();
        store.put(b"k2", b"beta").unwrap();
        let bad = store.commit_batch(&[
            StorageWriteOp::Put {
                key: b"rolled_back".to_vec(),
                value: b"nope".to_vec(),
            },
            StorageWriteOp::Delete { key: Vec::new() },
        ]);
        assert!(matches!(bad, Err(ServiceError::InvalidArg(_))));
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
        let native = HostServiceRegistry::new().with_handler(NativeStorage::with_root_dir(&root));
        let (store, info) = StorageClient::new(&native).open_with_info("wal").unwrap();
        assert!(info.recovered_torn_tail);
        assert_eq!(info.key_count, 2);
        assert_eq!(info.log_bytes, clean_len);
        store.put(b"k3", b"gamma").unwrap();
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

    let bad_hdr_path = root.join("bad_hdr.nvk1");
    fs::write(
        &bad_hdr_path,
        b"NOPE\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
    )
    .unwrap();
    assert!(matches!(
        KvStore::open_file(3, &bad_hdr_path),
        Err(ServiceError::Codec(_))
    ));

    // In-memory recovery + C-ABI >4 KiB retry.
    let mut mem_store = KvStore::open_in_memory(4);
    mem_store.put(b"m1", b"one").unwrap();
    let mut corrupted = mem_store.memory_log_bytes().unwrap().to_vec();
    let valid_len = corrupted.len();
    corrupted.extend_from_slice(&[1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 1, 2]);
    let recovered = KvStore::open_from_memory_log(5, corrupted).unwrap();
    assert!(recovered.open_info().recovered_torn_tail);
    assert_eq!(recovered.log_bytes() as usize, valid_len);

    let mem = MemoryHostServices::new().with_handler(MemoryStorage::new());
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let store = StorageClient::new(&ffi).open("session").unwrap();
    let large_val = vec![0xABu8; FFI_STACK_BUF_SIZE * 2 + 123];
    store.put(b"blob:large", &large_val).unwrap();
    assert_eq!(
        store.get(b"blob:large").unwrap().as_deref(),
        Some(large_val.as_slice())
    );

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
        let _ = decode_open_request(&buf);
        let _ = decode_open_response(&buf);
        let _ = decode_key_request(&buf);
        let _ = decode_put_request(&buf);
        let _ = decode_get_response(&buf);
        let _ = decode_scan_prefix_request(&buf);
        let _ = decode_scan_prefix_response(&buf);
        let _ = decode_commit_batch_request(&buf);
        let _ = decode_compact_response(&buf);
        let _ = KvStore::open_from_memory_log(99, buf);
    }

    let _ = fs::remove_dir_all(&root);
}
