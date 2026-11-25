//! Comprehensive edge-case & `NVE_IFACE_HOST = 5` round-trip tests for all Tier-2/3
//! `nve-sdk` zero-JSON native hardware & storage capabilities.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nve_abi::{
    NveHostEventBatch, NveHostStatus, SERVICE_CHANNEL, SERVICE_CLIPBOARD, SERVICE_FILESYSTEM,
    SERVICE_HAPTICS, SERVICE_LINKING, SERVICE_NOTIFICATIONS, SERVICE_SECURE_STORE,
    SERVICE_SYSTEM_ENV, SERVICE_WINDOW,
};
use nve_sdk::{
    audio, camera, fonts, sensors, share_web, AudioClient, AudioClipMeta, AudioEncoding,
    AudioEvent, AudioLevelEvent, AudioPlaybackEvent, AudioPlaybackState, AudioRecordConfig,
    AudioSource, BrowserPresentation, CameraCaptureOptions, CameraPreviewConfig, CapturedMedia,
    FfiHostServices, FlashMode, FontFormat, GeoFix, HostCallbackBridge, HostEvent,
    HostServiceProvider, InAppBrowserOptions, InvokeResult, KvStore, LocationAccuracy,
    LocationClient, LocationUpdateEvent, LocationWatchConfig, MediaFormat, MemoryHostServices,
    NativeHostServices, PermissionStatus, RecordedBrowserLaunch, RecordedShare, SdkClient,
    SdkError, SensorClient, SensorKind, SensorSample, SensorSampleSlice, ShareResult, StorageEntry,
    StorageWriteOp, WebAuthResult, FFI_STACK_BUF_SIZE, NVK1_MAGIC, SENSOR_SAMPLE_WIRE_LEN,
    SERVICE_AUDIO, SERVICE_CAMERA, SERVICE_FONTS, SERVICE_LOCATION, SERVICE_SENSORS, SERVICE_SHARE,
    SERVICE_STORAGE, SERVICE_WEB_AUTH,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn unique_temp_dir(label: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nve_sdk_tier2_{label}_{}_{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn sample_ttf(extra: u8) -> Vec<u8> {
    let mut b = vec![0x00, 0x01, 0x00, 0x00, 0, 1, 0, 16, 0, 0, 0, 0];
    b.extend_from_slice(&[extra; 24]);
    b
}

fn sample_apple_ttf(extra: u8) -> Vec<u8> {
    let mut b = b"true\x00\x01\x00\x10\x00\x00\x00\x00".to_vec();
    b.extend_from_slice(&[extra; 16]);
    b
}

fn sample_otf(extra: u8) -> Vec<u8> {
    let mut b = b"OTTO\x00\x01\x00\x10\x00\x00\x00\x00".to_vec();
    b.extend_from_slice(&[extra; 20]);
    b
}

fn sample_woff(extra: u8) -> Vec<u8> {
    let mut b = b"wOFF".to_vec();
    b.extend_from_slice(&[extra; 44]);
    b
}

fn sample_woff2(extra: u8) -> Vec<u8> {
    let mut b = b"wOF2".to_vec();
    b.extend_from_slice(&[extra; 48]);
    b
}

fn sample_ttc(extra: u8) -> Vec<u8> {
    let mut b = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x01".to_vec();
    b.extend_from_slice(&[extra; 12]);
    b
}

#[test]
fn all_builtin_service_ids_are_strictly_distinct() {
    let ids = [
        SERVICE_CHANNEL,
        SERVICE_WINDOW,
        SERVICE_CLIPBOARD,
        SERVICE_FILESYSTEM,
        SERVICE_SECURE_STORE,
        SERVICE_LINKING,
        SERVICE_SYSTEM_ENV,
        SERVICE_NOTIFICATIONS,
        SERVICE_HAPTICS,
        SERVICE_STORAGE,
        SERVICE_FONTS,
        SERVICE_AUDIO,
        SERVICE_CAMERA,
        SERVICE_LOCATION,
        SERVICE_SENSORS,
        SERVICE_SHARE,
        SERVICE_WEB_AUTH,
    ];
    let unique: BTreeSet<u32> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "service ID collision detected");
}

#[test]
fn nvk1_storage_persistence_atomic_batch_rollback_prefix_scan_and_compaction() {
    let root = unique_temp_dir("kv_persist");

    {
        let native = NativeHostServices::with_storage_root(&root);
        let sdk = SdkClient::new(&native);
        let store = sdk.storage().open("app_state").unwrap();
        assert_eq!(store.namespace(), "app_state");

        // Reject invalid namespaces and empty keys cleanly.
        assert!(matches!(
            sdk.storage().open("../escape"),
            Err(SdkError::InvalidArg(_))
        ));
        assert!(matches!(
            sdk.storage().open(""),
            Err(SdkError::InvalidArg(_))
        ));
        assert!(matches!(store.put(b"", b"v"), Err(SdkError::InvalidArg(_))));
        assert!(matches!(store.get(b""), Err(SdkError::InvalidArg(_))));

        // Put, overwrite, delete, and verify binary values.
        store.put(b"user:02", b"bob").unwrap();
        store.put(b"user:01", b"alice_v1").unwrap();
        store.put(b"user:01", b"alice_v2").unwrap();
        store.put(b"user:03", b"charlie").unwrap();
        store.put(b"meta:ver", &[0x00, 0xFF, 0x10, 0x20]).unwrap();
        assert!(store.delete(b"user:03").unwrap());
        assert!(!store.delete(b"user:03").unwrap());

        // Atomic batch commit.
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

        // Atomic batch rollback on invalid op (valid op followed by empty key).
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
        assert!(matches!(err, Err(SdkError::InvalidArg(_))));
        assert_eq!(store.get(b"user:99").unwrap(), None);
        assert_eq!(store.get(b"user:01").unwrap().as_deref(), Some(b"alice_v2".as_slice()));

        // Prefix scan ordering & limit.
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
        let limited = store.scan_prefix(b"user:", 2).unwrap();
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].key, b"user:01");
        assert_eq!(limited[1].key, b"user:04");
        assert!(store.scan_prefix(b"user:", 0).unwrap().is_empty());

        // Churn updates then compact -> must shrink file size & leave no leftover tmp file.
        for i in 0u32..40 {
            store
                .put(b"hot:counter", &i.to_le_bytes())
                .unwrap();
        }
        let stats = store.compact().unwrap();
        assert_eq!(stats.live_keys, 5);
        assert!(
            stats.bytes_after < stats.bytes_before,
            "expected compaction to shrink log: before={}, after={}",
            stats.bytes_before,
            stats.bytes_after
        );
        assert!(!root.join("app_state.nvk1.compact.tmp").exists());
    }

    // Reopen fresh NativeHostServices from disk and confirm full durability + magic.
    let log_path = root.join("app_state.nvk1");
    let raw = fs::read(&log_path).unwrap();
    assert_eq!(&raw[0..4], &NVK1_MAGIC);

    {
        let native = NativeHostServices::with_storage_root(&root);
        let sdk = SdkClient::new(&native);
        let (store, info) = sdk.storage().open_with_info("app_state").unwrap();
        assert_eq!(info.key_count, 5);
        assert!(!info.recovered_torn_tail);
        assert_eq!(store.get(b"user:01").unwrap().as_deref(), Some(b"alice_v2".as_slice()));
        assert_eq!(store.get(b"user:02").unwrap(), None);
        assert_eq!(
            store.get(b"meta:ver").unwrap().as_deref(),
            Some(&[0x00u8, 0xFF, 0x10, 0x20][..])
        );
        assert_eq!(
            store.get(b"hot:counter").unwrap().as_deref(),
            Some(&39u32.to_le_bytes()[..])
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nvk1_crash_recovery_torn_tail_and_corrupted_checksum_disk_and_memory() {
    let root = unique_temp_dir("kv_crash");
    let log_path = root.join("wal.nvk1");

    // 1. Write 3 committed records, then simulate a mid-write torn tail on disk.
    {
        let mut store = KvStore::open_file(1, &log_path).unwrap();
        store.put(b"k1", b"alpha").unwrap();
        store.put(b"k2", b"beta").unwrap();
        // Also test direct KvStore-level atomic batch rollback on invalid op.
        let bad = store.commit_batch(&[
            StorageWriteOp::Put {
                key: b"rolled_back".to_vec(),
                value: b"nope".to_vec(),
            },
            StorageWriteOp::Delete { key: Vec::new() },
        ]);
        assert!(matches!(bad, Err(SdkError::InvalidArg(_))));
        assert_eq!(store.get(b"rolled_back").unwrap(), None);
    }
    let clean_len = fs::metadata(&log_path).unwrap().len();
    {
        let mut f = OpenOptions::new().append(true).open(&log_path).unwrap();
        // Partial record header + half payload (simulates sudden power loss).
        f.write_all(&[1, 0, 0, 0, 99, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0xDE, 0xAD])
            .unwrap();
        f.sync_all().unwrap();
    }
    assert!(fs::metadata(&log_path).unwrap().len() > clean_len);

    // Reopen -> recovers cleanly, truncates torn tail back to `clean_len`, allows new writes.
    {
        let native = NativeHostServices::with_storage_root(&root);
        let sdk = SdkClient::new(&native);
        let (store, info) = sdk.storage().open_with_info("wal").unwrap();
        assert!(info.recovered_torn_tail);
        assert_eq!(info.key_count, 2);
        assert_eq!(info.log_bytes, clean_len);
        assert_eq!(store.get(b"k1").unwrap().as_deref(), Some(b"alpha".as_slice()));
        assert_eq!(store.get(b"k2").unwrap().as_deref(), Some(b"beta".as_slice()));
        store.put(b"k3", b"gamma").unwrap();
    }
    assert!(fs::metadata(&log_path).unwrap().len() > clean_len);

    // 2. Flip a single bit in the last record's payload (checksum corruption!).
    {
        let mut f = OpenOptions::new().read(true).write(true).open(&log_path).unwrap();
        let len = f.seek(SeekFrom::End(0)).unwrap();
        // Flip one byte inside the final record's payload (before the 12-byte trailer).
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
        assert_eq!(store.get(b"k1").unwrap().as_deref(), Some(b"alpha".as_slice()));
        assert_eq!(store.get(b"k2").unwrap().as_deref(), Some(b"beta".as_slice()));
        // Corrupted k3 record must be discarded rather than returning garbage.
        assert_eq!(store.get(b"k3").unwrap(), None);
    }

    // 3. Corrupt the 16-byte NVK1 header -> must fail with SdkError::Codec (never silently wipe).
    let bad_hdr_path = root.join("bad_hdr.nvk1");
    fs::write(&bad_hdr_path, b"NOPE\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();
    assert!(matches!(
        KvStore::open_file(3, &bad_hdr_path),
        Err(SdkError::Codec(_))
    ));

    // 4. In-memory NVK1 crash recovery works identically (WASM/headless path).
    let mut mem_store = KvStore::open_in_memory(4);
    mem_store.put(b"m1", b"one").unwrap();
    mem_store.put(b"m2", b"two").unwrap();
    let mut corrupted_buf = mem_store.memory_log_bytes().unwrap().to_vec();
    let valid_len = corrupted_buf.len();
    corrupted_buf.extend_from_slice(&[1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 1, 2]);
    let recovered = KvStore::open_from_memory_log(5, corrupted_buf).unwrap();
    assert!(recovered.open_info().recovered_torn_tail);
    assert_eq!(recovered.log_bytes() as usize, valid_len);
    assert_eq!(recovered.get(b"m1").unwrap().as_deref(), Some(b"one".as_slice()));
    assert_eq!(recovered.get(b"m2").unwrap().as_deref(), Some(b"two".as_slice()));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn font_loader_magic_validation_corrupt_rejection_deduplication_and_file_registration() {
    let dir = unique_temp_dir("fonts");
    let native = NativeHostServices::in_memory_storage();
    let sdk = SdkClient::new(&native);
    let fc = sdk.fonts();

    // Valid TTF, Apple TTF, OTF, WOFF, WOFF2, TTC.
    let h_ttf = fc.register_bytes("Inter", 400, false, &sample_ttf(1)).unwrap();
    let h_apple = fc
        .register_bytes("SF Pro", 500, false, &sample_apple_ttf(2))
        .unwrap();
    let h_otf = fc.register_bytes("Inter", 700, true, &sample_otf(3)).unwrap();
    let h_woff = fc.register_bytes("JetBrains Mono", 400, false, &sample_woff(4)).unwrap();
    let h_woff2 = fc
        .register_bytes("JetBrains Mono", 700, false, &sample_woff2(5))
        .unwrap();
    let h_ttc = fc.register_bytes("Noto Sans", 400, false, &sample_ttc(6)).unwrap();

    // Deduplication: identical (family, weight, italic, bytes) returns the exact same FontHandle.
    let h_ttf_dup = fc.register_bytes("Inter", 400, false, &sample_ttf(1)).unwrap();
    assert_eq!(h_ttf, h_ttf_dup);

    let list = fc.list_registered().unwrap();
    assert_eq!(list.len(), 6);
    assert_eq!(list.iter().find(|f| f.handle == h_ttf).unwrap().format, FontFormat::TrueType);
    assert_eq!(list.iter().find(|f| f.handle == h_apple).unwrap().format, FontFormat::TrueType);
    assert_eq!(list.iter().find(|f| f.handle == h_otf).unwrap().format, FontFormat::OpenType);
    assert_eq!(list.iter().find(|f| f.handle == h_woff).unwrap().format, FontFormat::Woff);
    assert_eq!(list.iter().find(|f| f.handle == h_woff2).unwrap().format, FontFormat::Woff2);
    assert_eq!(
        list.iter().find(|f| f.handle == h_ttc).unwrap().format,
        FontFormat::TrueTypeCollection
    );

    // Corrupt / empty / truncated payloads rejected cleanly.
    assert!(matches!(
        fc.register_bytes("Bad", 400, false, &[]),
        Err(SdkError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("Bad", 400, false, &[0, 1, 0, 0, 1, 2]),
        Err(SdkError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("Bad", 400, false, b"NOT_A_FONT_HEADER_BYTES"),
        Err(SdkError::InvalidArg(_))
    ));
    // WOFF requires >= 44 bytes; WOFF2 requires >= 48 bytes.
    assert!(matches!(
        fc.register_bytes("BadWoff", 400, false, b"wOFF0123456789012345"),
        Err(SdkError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("BadWoff2", 400, false, &[b'w', b'O', b'F', b'2', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        Err(SdkError::InvalidArg(_))
    ));
    // Invalid descriptors.
    assert!(matches!(
        fc.register_bytes("", 400, false, &sample_ttf(9)),
        Err(SdkError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("Inter", 0, false, &sample_ttf(9)),
        Err(SdkError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("Inter", 1001, false, &sample_ttf(9)),
        Err(SdkError::InvalidArg(_))
    ));

    // Disk file registration + missing file + corrupt file rejection.
    let good_file = dir.join("custom.otf");
    let bad_file = dir.join("corrupt.otf");
    fs::write(&good_file, sample_otf(42)).unwrap();
    fs::write(&bad_file, b"corrupt_font_bytes_12345").unwrap();

    let h_file = fc
        .register_file("CustomFile", 600, false, good_file.to_str().unwrap())
        .unwrap();
    assert_eq!(
        fc.register_file("CustomFile", 600, false, bad_file.to_str().unwrap()),
        Err(SdkError::InvalidArg("unrecognized font magic [63, 6F, 72, 72]".into()))
    );
    assert_eq!(
        fc.register_file("Missing", 400, false, dir.join("nope.ttf").to_str().unwrap()),
        Err(SdkError::NotFound)
    );

    // Unregister removes handle and releases dedup entry.
    assert!(fc.unregister(h_file).unwrap());
    assert!(!fc.unregister(h_file).unwrap());
    assert_eq!(fc.list_registered().unwrap().len(), 6);
    assert!(matches!(
        fonts::decode_handle(&[0u8; 8]),
        Err(SdkError::InvalidArg(_))
    ));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn full_nve_iface_host_5_binary_roundtrip_all_tier2_hardware_and_storage_services() {
    let mem = MemoryHostServices::new();
    mem.add_mock_font_file("/assets/ui.woff2", sample_woff2(77));
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let sdk = SdkClient::new(&ffi);

    // 1. Storage over NVE_IFACE_HOST = 5 including >4 KiB BufferTooSmall heap-retry path!
    let store = sdk.storage().open("session").unwrap();
    let large_val = vec![0xABu8; FFI_STACK_BUF_SIZE * 2 + 123];
    store.put(b"blob:large", &large_val).unwrap();
    assert_eq!(store.get(b"blob:large").unwrap().as_deref(), Some(large_val.as_slice()));
    let stats = store.compact().unwrap();
    assert_eq!(stats.live_keys, 1);

    // 2. Fonts over NVE_IFACE_HOST = 5
    let fh = sdk
        .fonts()
        .register_file("BrandUI", 500, false, "/assets/ui.woff2")
        .unwrap();
    let registered = sdk.fonts().list_registered().unwrap();
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].handle, fh);
    assert_eq!(registered[0].format, FontFormat::Woff2);

    // 3. Audio playback, recording, validation, and NVH1 push events
    let h_uri = sdk
        .audio()
        .load_source(&AudioSource::Uri("https://cdn.nve.dev/track.opus".into()))
        .unwrap();
    let h_bytes = sdk
        .audio()
        .load_source(&AudioSource::Bytes(vec![0x52, 0x49, 0x46, 0x46]))
        .unwrap();
    sdk.audio().play(h_uri).unwrap();
    sdk.audio().seek_ms(h_uri, 12_345).unwrap();
    sdk.audio().set_volume(h_uri, 0.75).unwrap();
    sdk.audio().set_looping(h_uri, true).unwrap();
    sdk.audio().pause(h_uri).unwrap();

    let track = bridge.provider().audio_track(h_uri).unwrap();
    assert_eq!(track.state, AudioPlaybackState::Paused);
    assert_eq!(track.position_ms, 12_345);
    assert_eq!(track.volume, 0.75);
    assert!(track.looping);

    assert!(matches!(
        sdk.audio().set_volume(h_uri, 1.5),
        Err(SdkError::InvalidArg(_))
    ));
    assert!(matches!(
        sdk.audio().set_volume(h_uri, f32::NAN),
        Err(SdkError::InvalidArg(_))
    ));
    sdk.audio().stop(h_uri).unwrap();
    sdk.audio().unload(h_bytes).unwrap();
    assert_eq!(sdk.audio().play(h_bytes), Err(SdkError::NotFound));

    // Recording lifecycle
    let rec_cfg = AudioRecordConfig {
        sample_rate_hz: 44_100,
        channels: 2,
        encoding: AudioEncoding::Opus,
        bit_rate_bps: Some(128_000),
        output_path: Some("/mem/tmp/note.opus".into()),
        max_duration_ms: Some(30_000),
    };
    sdk.audio().start_recording(&rec_cfg).unwrap();
    assert!(matches!(
        sdk.audio().start_recording(&rec_cfg),
        Err(SdkError::InvalidArg(_))
    ));
    let clip: AudioClipMeta = sdk.audio().stop_recording().unwrap();
    assert_eq!(clip.encoding, AudioEncoding::Opus);
    assert_eq!(clip.sample_rate_hz, 44_100);
    assert_eq!(clip.channels, 2);
    assert_eq!(clip.path.as_deref(), Some("/mem/tmp/note.opus"));
    assert_eq!(sdk.audio().stop_recording(), Err(SdkError::NotFound));

    // 4. Camera devices, permission, capture, preview, and denial
    let devices = sdk.camera().list_devices().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(sdk.camera().request_permission().unwrap(), PermissionStatus::Authorized);
    let photo: CapturedMedia = sdk
        .camera()
        .capture_photo(&CameraCaptureOptions {
            device_id: Some("cam-back-0".into()),
            flash: FlashMode::On,
            format: MediaFormat::Png,
            quality: 95,
            max_width: Some(640),
            max_height: Some(480),
        })
        .unwrap();
    assert_eq!(photo.format, MediaFormat::Png);
    assert_eq!(photo.width, 640);
    assert_eq!(photo.height, 480);

    let preview = sdk
        .camera()
        .start_preview(&CameraPreviewConfig {
            device_id: Some("cam-front-1".into()),
            width: 1280,
            height: 720,
            target_fps: 60,
            mirror: true,
        })
        .unwrap();
    assert_eq!(bridge.provider().active_camera_previews(), vec![preview]);
    sdk.camera().stop_preview(preview).unwrap();
    assert_eq!(sdk.camera().stop_preview(preview), Err(SdkError::NotFound));

    assert!(matches!(
        sdk.camera().capture_photo(&CameraCaptureOptions {
            quality: 101,
            ..CameraCaptureOptions::default()
        }),
        Err(SdkError::InvalidArg(_))
    ));
    bridge
        .provider()
        .set_camera_permission(PermissionStatus::Denied);
    assert_eq!(
        sdk.camera().capture_photo(&CameraCaptureOptions::default()),
        Err(SdkError::PermissionDenied)
    );

    // 5. Location fixes, watch lifecycle, and permission denial
    assert_eq!(
        sdk.location().request_permission().unwrap(),
        PermissionStatus::Authorized
    );
    let custom_fix = GeoFix {
        latitude: 48.1351,
        longitude: 11.5820,
        altitude_m: Some(519.0),
        accuracy_m: 3.5,
        speed_mps: Some(1.2),
        heading_deg: Some(180.0),
        timestamp_ms: 1_750_000_000_000,
    };
    bridge.provider().set_current_fix(custom_fix);
    assert_eq!(
        sdk.location()
            .current_position(LocationAccuracy::BestForNavigation)
            .unwrap(),
        custom_fix
    );
    let watch = sdk
        .location()
        .watch_position(&LocationWatchConfig {
            accuracy: LocationAccuracy::High,
            min_interval_ms: 500,
            min_distance_m: 2.5,
        })
        .unwrap();
    assert_eq!(bridge.provider().active_location_watches(), vec![watch]);
    sdk.location().stop_watch(watch).unwrap();
    assert_eq!(sdk.location().stop_watch(watch), Err(SdkError::NotFound));

    bridge
        .provider()
        .set_location_permission(PermissionStatus::Restricted);
    assert_eq!(
        sdk.location().current_position(LocationAccuracy::Balanced),
        Err(SdkError::PermissionDenied)
    );

    // 6. Sensors availability, subscribe/unsubscribe, and zero-copy fixed-layout SensorSample
    for kind in [
        SensorKind::Accelerometer,
        SensorKind::Gyroscope,
        SensorKind::Magnetometer,
        SensorKind::Barometer,
        SensorKind::AmbientLight,
    ] {
        assert!(sdk.sensors().is_available(kind).unwrap());
    }
    bridge
        .provider()
        .set_sensor_available(SensorKind::Barometer, false);
    assert!(!sdk.sensors().is_available(SensorKind::Barometer).unwrap());
    assert_eq!(
        sdk.sensors().subscribe(SensorKind::Barometer, 10),
        Err(SdkError::Unsupported)
    );
    assert!(matches!(
        sdk.sensors().subscribe(SensorKind::Accelerometer, 0),
        Err(SdkError::InvalidArg(_))
    ));
    let sub = sdk
        .sensors()
        .subscribe(SensorKind::Accelerometer, 120)
        .unwrap();
    assert_eq!(
        bridge.provider().active_sensor_subscriptions(),
        vec![(sub, SensorKind::Accelerometer, 120)]
    );
    sdk.sensors().unsubscribe(sub).unwrap();
    assert_eq!(sdk.sensors().unsubscribe(sub), Err(SdkError::NotFound));

    // 7. Share Sheet
    assert_eq!(
        sdk.share()
            .share_text("Check out NVE", Some("NVE Release"))
            .unwrap(),
        ShareResult::Completed
    );
    assert_eq!(
        sdk.share()
            .share_url("https://nve.dev/docs", Some("Docs"))
            .unwrap(),
        ShareResult::Completed
    );
    bridge.provider().set_share_result(ShareResult::Dismissed);
    assert_eq!(
        sdk.share()
            .share_files(&["/tmp/a.png", "/tmp/b.pdf"])
            .unwrap(),
        ShareResult::Dismissed
    );
    assert_eq!(
        bridge.provider().recorded_shares(),
        vec![
            RecordedShare::Text {
                text: "Check out NVE".into(),
                subject: Some("NVE Release".into()),
            },
            RecordedShare::Url {
                url: "https://nve.dev/docs".into(),
                title: Some("Docs".into()),
            },
            RecordedShare::Files(vec!["/tmp/a.png".into(), "/tmp/b.pdf".into()]),
        ]
    );
    assert!(matches!(
        sdk.share().share_files::<&str>(&[]),
        Err(SdkError::InvalidArg(_))
    ));

    // 8. In-App WebBrowser & OAuth WebAuth (sync + async NVH1 round-trip!)
    let browser_opts = InAppBrowserOptions {
        presentation: BrowserPresentation::FullScreen,
        toolbar_color_rgba: Some(0x1122_33FF),
        ephemeral_session: true,
        show_title: true,
    };
    sdk.web_auth()
        .open_in_app_browser("https://docs.nve.dev", &browser_opts)
        .unwrap();
    assert_eq!(
        bridge.provider().opened_browsers(),
        vec![RecordedBrowserLaunch {
            url: "https://docs.nve.dev".into(),
            options: browser_opts,
        }]
    );
    // Reject unsafe non-http(s) URL schemes and malformed callback schemes.
    assert!(matches!(
        sdk.web_auth()
            .open_in_app_browser("javascript:alert(1)", &InAppBrowserOptions::default()),
        Err(SdkError::InvalidArg(_))
    ));
    assert!(matches!(
        sdk.web_auth()
            .authenticate("https://auth.nve.dev/login", "bad:scheme"),
        Err(SdkError::InvalidArg(_))
    ));

    assert_eq!(
        sdk.web_auth()
            .authenticate("https://auth.nve.dev/authorize", "nveapp")
            .unwrap(),
        WebAuthResult::CallbackUri("nveapp://callback?code=mock_auth_code".into())
    );
    bridge
        .provider()
        .set_web_auth_result(Some(WebAuthResult::Cancelled));
    assert_eq!(
        sdk.web_auth()
            .authenticate("https://auth.nve.dev/authorize", "nveapp")
            .unwrap(),
        WebAuthResult::Cancelled
    );

    // Async Pending + NVH1 AsyncResponse + Push Events (Audio, Location, Sensors) in ONE batch!
    bridge
        .provider()
        .inject_async_pending(SERVICE_WEB_AUTH, share_web::METHOD_AUTHENTICATE);
    let auth_req =
        share_web::encode_authenticate_request("https://auth.nve.dev/oauth", "nveapp").unwrap();
    let req_id = match ffi
        .invoke_raw(SERVICE_WEB_AUTH, share_web::METHOD_AUTHENTICATE, &auth_req)
        .unwrap()
    {
        InvokeResult::Pending { request_id } => request_id,
        other => panic!("expected Pending, got {other:?}"),
    };

    let playback_ev = AudioPlaybackEvent {
        handle: h_uri,
        state: AudioPlaybackState::Playing,
        position_ms: 4200,
        duration_ms: 180_000,
    };
    let level_ev = AudioLevelEvent {
        handle: Some(h_uri),
        rms_db: -18.5,
        peak_db: -6.2,
        timestamp_ms: 99_001,
    };
    let loc_ev = LocationUpdateEvent {
        watch_id: watch,
        fix: custom_fix,
    };
    let samples = [
        SensorSample {
            x: 0.125,
            y: -9.81,
            z: 0.05,
            timestamp_ns: 1_000_000,
        },
        SensorSample {
            x: 0.25,
            y: -9.79,
            z: 0.02,
            timestamp_ns: 9_333_333,
        },
    ];
    let sensor_payload =
        sensors::encode_sensor_batch_event(sub, SensorKind::Accelerometer, &samples).unwrap();

    let mut batch = NveHostEventBatch::new();
    batch.push_async_response(
        req_id,
        NveHostStatus::Ok,
        share_web::encode_web_auth_result(&WebAuthResult::CallbackUri(
            "nveapp://cb?code=async_42".into(),
        ))
        .unwrap(),
    );
    batch.push_host_event(
        SERVICE_AUDIO,
        audio::EVENT_PLAYBACK_STATE,
        audio::encode_playback_event(&playback_ev).unwrap(),
    );
    batch.push_host_event(
        SERVICE_AUDIO,
        audio::EVENT_LEVEL_METER,
        audio::encode_level_event(&level_ev).unwrap(),
    );
    batch.push_host_event(
        SERVICE_LOCATION,
        sensors::EVENT_LOCATION_UPDATE,
        sensors::encode_location_update_event(&loc_ev).unwrap(),
    );
    batch.push_host_event(SERVICE_SENSORS, sensors::EVENT_SENSOR_BATCH, sensor_payload);

    assert_eq!(ffi.dispatch_host_events(&batch.encode()).unwrap(), 5);

    let async_bytes = ffi.take_async_response(req_id).unwrap().unwrap();
    assert_eq!(
        share_web::decode_web_auth_result(&async_bytes).unwrap(),
        WebAuthResult::CallbackUri("nveapp://cb?code=async_42".into())
    );

    let events = ffi.poll_host_events();
    assert_eq!(events.len(), 4);
    let HostEvent::Push(p0) = &events[0] else {
        panic!("expected push event");
    };
    assert_eq!(
        AudioClient::<FfiHostServices>::decode_push_event(p0).unwrap(),
        Some(AudioEvent::Playback(playback_ev))
    );
    let HostEvent::Push(p1) = &events[1] else {
        panic!("expected push event");
    };
    assert_eq!(
        AudioClient::<FfiHostServices>::decode_push_event(p1).unwrap(),
        Some(AudioEvent::Level(level_ev))
    );
    let HostEvent::Push(p2) = &events[2] else {
        panic!("expected push event");
    };
    assert_eq!(
        LocationClient::<FfiHostServices>::decode_push_event(p2).unwrap(),
        Some(loc_ev)
    );
    let HostEvent::Push(p3) = &events[3] else {
        panic!("expected push event");
    };
    let decoded_ref = SensorClient::<FfiHostServices>::decode_push_event_ref(p3)
        .unwrap()
        .unwrap();
    assert_eq!(decoded_ref.subscription_id, sub);
    assert_eq!(decoded_ref.kind, SensorKind::Accelerometer);
    assert_eq!(decoded_ref.samples.len(), 2);
    assert_eq!(decoded_ref.samples.get(0), Some(samples[0]));
    assert_eq!(decoded_ref.samples.get(1), Some(samples[1]));
    assert_eq!(decoded_ref.samples.get(2), None);
}

#[test]
fn sensor_sample_and_tier2_codecs_reject_truncated_corrupt_and_nan_inputs() {
    // Exact 20-byte fixed-layout SensorSample validation.
    let valid = SensorSample {
        x: 1.5,
        y: -2.25,
        z: 9.81,
        timestamp_ns: 42_000_000,
    };
    let wire = valid.encode().unwrap();
    assert_eq!(wire.len(), SENSOR_SAMPLE_WIRE_LEN);
    assert_eq!(SensorSample::decode(&wire).unwrap(), valid);

    // Every truncation fails cleanly.
    for len in 0..SENSOR_SAMPLE_WIRE_LEN {
        assert!(SensorSample::decode(&wire[..len]).is_err());
    }
    // Trailing byte fails cleanly.
    let mut over = wire.to_vec();
    over.push(0);
    assert!(SensorSample::decode(&over).is_err());

    // Non-finite floats (NaN / Inf) rejected on both encode and decode.
    assert!(SensorSample {
        x: f32::NAN,
        y: 0.0,
        z: 0.0,
        timestamp_ns: 1,
    }
    .encode()
    .is_err());
    let mut nan_wire = wire;
    nan_wire[0..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(SensorSample::decode(&nan_wire).is_err());

    // Misaligned / corrupted SensorSampleSlice rejected.
    assert!(SensorSampleSlice::from_bytes(&wire[..19]).is_err());
    let mut bad_batch = Vec::new();
    bad_batch.extend_from_slice(&wire);
    bad_batch.extend_from_slice(&nan_wire);
    assert!(SensorSampleSlice::from_bytes(&bad_batch).is_err());

    // Packed batch prefix truncation & declared count overflow (anti-OOM).
    let batch_wire = sensors::encode_sensor_batch_event(
        nve_sdk::SensorSubscriptionId(7),
        SensorKind::Gyroscope,
        &[valid],
    )
    .unwrap();
    for len in 0..batch_wire.len() {
        assert!(sensors::decode_sensor_batch_event_ref(&batch_wire[..len]).is_err());
    }
    let mut crafted_count = batch_wire.clone();
    crafted_count[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(sensors::decode_sensor_batch_event_ref(&crafted_count).is_err());

    // Audio level NaN rejection & GeoFix out-of-range rejection.
    assert!(audio::encode_level_event(&AudioLevelEvent {
        handle: None,
        rms_db: f32::NEG_INFINITY,
        peak_db: -1.0,
        timestamp_ms: 1,
    })
    .is_err());
    assert!(sensors::encode_geo_fix(&GeoFix {
        latitude: 95.0,
        longitude: 0.0,
        altitude_m: None,
        accuracy_m: 1.0,
        speed_mps: None,
        heading_deg: None,
        timestamp_ms: 1,
    })
    .is_err());

    // Camera invalid enum byte rejection.
    assert!(camera::decode_captured_media(&[99, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
}

#[test]
fn native_host_services_tier2_headless_safety() {
    let native = NativeHostServices::in_memory_storage();
    let sdk = SdkClient::new(&native);

    // Storage works in-memory without touching disk.
    let kv = sdk.storage().open("headless").unwrap();
    kv.put(b"ok", b"yes").unwrap();
    assert_eq!(kv.get(b"ok").unwrap().as_deref(), Some(b"yes".as_slice()));

    // Safe non-modal hardware queries never trigger OS permission modals.
    assert_eq!(sdk.camera().list_devices().unwrap(), Vec::new());
    assert_eq!(
        sdk.camera().request_permission(),
        Err(SdkError::Unsupported)
    );
    assert_eq!(
        sdk.location().request_permission(),
        Err(SdkError::Unsupported)
    );
    assert!(!sdk.sensors().is_available(SensorKind::Accelerometer).unwrap());
    assert_eq!(
        sdk.share().share_text("hi", None),
        Err(SdkError::Unsupported)
    );
    assert_eq!(
        sdk.web_auth()
            .authenticate("https://example.com/auth", "myapp"),
        Err(SdkError::Unsupported)
    );
    // Malformed arguments still return InvalidArg before Unsupported.
    assert!(matches!(
        sdk.share().share_text("", None),
        Err(SdkError::InvalidArg(_))
    ));
}

#[test]
fn fuzz_all_tier2_binary_codecs_never_panic_on_random_or_truncated_bytes() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };

    for i in 0..256 {
        let len = i % 64;
        let mut buf = Vec::with_capacity(len);
        for _ in 0..len {
            buf.push(next_u8());
        }
        let _ = nve_sdk::storage::decode_open_request(&buf);
        let _ = nve_sdk::storage::decode_open_response(&buf);
        let _ = nve_sdk::storage::decode_key_request(&buf);
        let _ = nve_sdk::storage::decode_put_request(&buf);
        let _ = nve_sdk::storage::decode_get_response(&buf);
        let _ = nve_sdk::storage::decode_scan_prefix_request(&buf);
        let _ = nve_sdk::storage::decode_scan_prefix_response(&buf);
        let _ = nve_sdk::storage::decode_commit_batch_request(&buf);
        let _ = nve_sdk::storage::decode_compact_response(&buf);
        let _ = KvStore::open_from_memory_log(99, buf.clone());
        let _ = fonts::validate_font_bytes(&buf);
        let _ = fonts::decode_register_bytes_request(&buf);
        let _ = fonts::decode_register_file_request(&buf);
        let _ = fonts::decode_list_registered_response(&buf);
        let _ = audio::decode_load_source_request(&buf);
        let _ = audio::decode_record_config(&buf);
        let _ = audio::decode_clip_meta(&buf);
        let _ = audio::decode_playback_event(&buf);
        let _ = audio::decode_level_event(&buf);
        let _ = camera::decode_devices_response(&buf);
        let _ = camera::decode_capture_options(&buf);
        let _ = camera::decode_captured_media(&buf);
        let _ = camera::decode_preview_config(&buf);
        let _ = sensors::decode_geo_fix(&buf);
        let _ = sensors::decode_watch_config(&buf);
        let _ = sensors::decode_location_update_event(&buf);
        let _ = sensors::decode_sensor_subscribe_request(&buf);
        let _ = sensors::decode_sensor_batch_event_ref(&buf);
        let _ = SensorSample::decode(&buf);
        let _ = SensorSampleSlice::from_bytes(&buf);
        let _ = share_web::decode_share_text_request(&buf);
        let _ = share_web::decode_share_url_request(&buf);
        let _ = share_web::decode_share_files_request(&buf);
        let _ = share_web::decode_open_browser_request(&buf);
        let _ = share_web::decode_authenticate_request(&buf);
        let _ = share_web::decode_web_auth_result(&buf);
    }
}
