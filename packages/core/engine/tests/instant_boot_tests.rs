use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nve_abi::{ColorRgba, FrameId, NodeId, SurfaceId, NVE_PRIMARY_SLOT_NONE};
use nve_components::style_patch::{LayoutSnapshot, VisualSnapshot};
use nve_engine::mmap_arena::ARENA_HEADER_SLOT_SIZE;
use nve_engine::{
    encode_scene_reloc, LayoutRect, NativeInstantBootStore, NativeStateStore, SceneRelocInput,
    SceneRelocNodeInput, StateStoreConfig, VerifyMode, Viewport, DEFAULT_STATE_ARENA_FILE,
    DEFAULT_USER_FRAME0_FILE,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve_instant_boot_{tag}_{}_{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp instant-boot test dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_sample_scene(schema_version: u32, title: &str) -> Vec<u8> {
    let nodes = [SceneRelocNodeInput {
        node_id: NodeId::ROOT,
        parent_index: -1,
        primary_slot: NVE_PRIMARY_SLOT_NONE,
        flags: 0,
        text_align: 0,
        image_resize_mode: 0,
        font_weight: 400,
        event_mask: 0,
        computed_rect: LayoutRect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        style: VisualSnapshot {
            background: ColorRgba::rgb(15, 23, 42),
            text_color: ColorRgba::WHITE,
            font_size: 20.0,
            ..Default::default()
        },
        layout: LayoutSnapshot::default(),
        text: title,
        tag: "root",
        image_src: "",
        box_shadow: "",
        draw_list: &[],
        children: &[],
    }];
    encode_scene_reloc(&SceneRelocInput {
        schema_version,
        surface_id: SurfaceId::ROOT,
        frame_id: FrameId(1),
        viewport: Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        },
        clear_color: ColorRgba::rgb(15, 23, 42),
        root_node_index: 0,
        app_tag: "test-app",
        nodes: &nodes,
    })
    .expect("encode scene reloc fixture")
    .as_slice()
    .to_vec()
}

#[test]
fn state_store_torn_header_slot_recovery_and_auto_compaction() {
    let dir = TempDir::new("torn_slot");
    let arena_path = dir.path().join(DEFAULT_STATE_ARENA_FILE);
    let cfg = StateStoreConfig {
        initial_capacity: 4096,
        compact_headroom: 4096,
        sync_on_save: true,
        auto_compact_every: 0,
    };

    let mut store = NativeStateStore::open(&arena_path, 2, cfg).unwrap();
    assert!(!store.recovery_stats().loaded_from_disk);
    assert_eq!(store.load_bytes().unwrap(), None);

    // Save #1 -> Slot B (gen=2), Save #2 -> Slot A (gen=3)
    let o1 = store.save(b"state-gen-2").unwrap();
    assert_eq!(o1.generation, 2);
    assert_eq!(o1.active_slot, 1);

    let o2 = store.save(b"state-gen-3-latest").unwrap();
    assert_eq!(o2.generation, 3);
    assert_eq!(o2.active_slot, 0);
    drop(store);

    // Simulate torn write of Slot A (corrupt bytes 0..64): Slot B (gen=2) MUST recover cleanly!
    {
        let mut f = OpenOptions::new().write(true).open(&arena_path).unwrap();
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(&[0xDEu8; ARENA_HEADER_SLOT_SIZE]).unwrap();
        f.sync_all().unwrap();
    }

    let mut recovered = NativeStateStore::open(&arena_path, 2, cfg).unwrap();
    assert!(recovered.recovery_stats().loaded_from_disk);
    assert!(!recovered.recovery_stats().recovered_from_corrupt_arena);
    assert_eq!(recovered.active_slot(), 1);
    assert_eq!(recovered.generation(), 2);
    assert_eq!(recovered.load_bytes().unwrap(), Some(b"state-gen-2".as_slice()));

    // Write payload larger than initial_capacity to prove automatic growth + compaction never fails.
    let big_payload = vec![0x5Au8; 12_000];
    let big_out = recovered.save(&big_payload).unwrap();
    assert!(big_out.compacted);
    assert_eq!(recovered.compaction_count(), 1);
    assert_eq!(recovered.load_bytes().unwrap(), Some(big_payload.as_slice()));
    drop(recovered);

    let reopened = NativeStateStore::open(&arena_path, 2, cfg).unwrap();
    assert_eq!(reopened.load_bytes().unwrap(), Some(big_payload.as_slice()));
}

#[test]
fn instant_boot_store_cold_bundled_returning_user_schema_mismatch_and_reset() {
    let dir = TempDir::new("instant_boot");
    let bundled_bytes = make_sample_scene(3, "Bundled Cold Frame 0");
    let user_bytes = make_sample_scene(3, "Returning User Frame 0");

    // 1. Cold first launch: no user state, bundled frame0 loads cleanly.
    let mut store = NativeInstantBootStore::open(dir.path(), 3, StateStoreConfig::default())
        .unwrap()
        .with_default_frame0_bytes(&bundled_bytes);
    assert_eq!(store.state_bytes().unwrap(), None);
    let f0 = store.load_frame0(VerifyMode::Full).expect("bundled frame0");
    assert!(!f0.is_user_cached());
    let sv = f0.scene_view(VerifyMode::Full).unwrap();
    assert_eq!(sv.text_of(&sv.nodes()[0]).unwrap(), "Bundled Cold Frame 0");

    // 2. Save returning-user checkpoint (user_frame0.nvereloc + state.nvemmap).
    store.save_checkpoint(&user_bytes, b"USER_STATE_V3").unwrap();
    assert!(dir.path().join(DEFAULT_USER_FRAME0_FILE).exists());
    assert!(dir.path().join(DEFAULT_STATE_ARENA_FILE).exists());
    drop(store);

    // 3. Returning user restart: prefers user_frame0.nvereloc and restores state.nvemmap.
    let mut returning = NativeInstantBootStore::open(dir.path(), 3, StateStoreConfig::default())
        .unwrap()
        .with_default_frame0_bytes(&bundled_bytes);
    assert!(returning.recovery_stats().loaded_from_disk);
    assert_eq!(returning.state_bytes().unwrap(), Some(b"USER_STATE_V3".as_slice()));
    let uf0 = returning.load_frame0(VerifyMode::Full).expect("user frame0");
    assert!(uf0.is_user_cached());
    let usv = uf0.scene_view(VerifyMode::Full).unwrap();
    assert_eq!(usv.text_of(&usv.nodes()[0]).unwrap(), "Returning User Frame 0");

    // 4. Reset clears user state and deletes user_frame0.nvereloc -> falls back to bundled frame0.
    returning.reset().unwrap();
    assert_eq!(returning.state_bytes().unwrap(), None);
    assert!(!dir.path().join(DEFAULT_USER_FRAME0_FILE).exists());
    let after_reset = returning.load_frame0(VerifyMode::Full).unwrap();
    assert!(!after_reset.is_user_cached());

    // Re-populate for schema mismatch test.
    returning.save_checkpoint(&user_bytes, b"USER_STATE_V3").unwrap();
    drop(returning);

    // 5. App upgrades to schema v4 -> auto-invalidates old v3 state and v3 user_frame0,
    //    falling back to bundled v4 frame0 cleanly.
    let bundled_v4 = make_sample_scene(4, "Bundled V4 Frame 0");
    let upgraded = NativeInstantBootStore::open(dir.path(), 4, StateStoreConfig::default())
        .unwrap()
        .with_default_frame0_bytes(&bundled_v4);
    assert!(upgraded.recovery_stats().schema_mismatch_reset);
    assert_eq!(upgraded.state_bytes().unwrap(), None);
    assert!(!dir.path().join(DEFAULT_USER_FRAME0_FILE).exists());
    let v4_f0 = upgraded.load_frame0(VerifyMode::Full).unwrap();
    assert!(!v4_f0.is_user_cached());
    assert_eq!(
        v4_f0
            .scene_view(VerifyMode::Full)
            .unwrap()
            .text_of(&v4_f0.scene_view(VerifyMode::Full).unwrap().nodes()[0])
            .unwrap(),
        "Bundled V4 Frame 0"
    );
    drop(upgraded);

    // 6. Total arena corruption recovers cleanly without panicking.
    std::fs::write(dir.path().join(DEFAULT_STATE_ARENA_FILE), b"GARBAGE_BYTES").unwrap();
    let recovered = NativeInstantBootStore::open(dir.path(), 4, StateStoreConfig::default()).unwrap();
    assert!(recovered.recovery_stats().recovered_from_corrupt_arena);
    assert_eq!(recovered.state_bytes().unwrap(), None);
}
