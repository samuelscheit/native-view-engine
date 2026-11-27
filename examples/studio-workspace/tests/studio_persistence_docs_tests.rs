mod common;

use common::{live_node_ids, register_studio_tables, unique_test_dir, MultiBackendHarness};
use nve_abi::xxh64;
use nve_abi::{
    pack_key_modifiers, EventMask, FrameId, NveDirectEvent, NveEventBatchRef, NveMapEntry,
    NvePropsRef, NveValue, SurfaceId, ROOT_NODE_ID,
};
use nve_canvas::{Canvas, CanvasOpRef, DrawList, DrawListDecoder, NVE_DRAW_MAGIC};
use nve_components::tables::{Image, ImageFit, ImageSrc, ImageSrcView, Interaction, TextInput};
use nve_engine::mmap_arena::ARENA_HEADER_SLOT_SIZE;
use nve_engine::NveApp;
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeStore};
use nve_studio_workspace::{
    AccentTheme, ActiveView, ChannelKind, CornerRadiusMode, DensityMode, DocBlockKind,
    PersistConfig, StudioWorkspaceModule, WorkbenchPreset, WorkspaceState, WorkspaceStore,
    HOST_SURFACES, STATE_ARENA_FILE_NAME, WORKSPACE_STATE_BIN_MAGIC, WORKSPACE_STATE_BIN_VERSION,
};

#[test]
fn command_buffer_emits_real_image_text_input_and_skia_canvas_with_nve_draw1_bytes() {
    let mut h = MultiBackendHarness::new(960.0, 640.0);
    let store = h.surface.store();

    let board_images: Vec<_> = live_node_ids(store)
        .into_iter()
        .filter_map(|id| store.try_row::<Image>(id).map(|row| (id, row)))
        .collect();
    assert!(
        board_images.len() >= 4,
        "expected PNG Image avatars on every issue card, got {}",
        board_images.len()
    );
    for (_, img) in &board_images {
        match store.resource::<ImageSrc>(img.src()) {
            Some(ImageSrcView::Url(src)) => {
                assert!(
                    src.starts_with("data:image/png;base64,"),
                    "expected PNG data URI src, got {src}"
                );
            }
            other => panic!("missing Image src resource: {other:?}"),
        }
        assert_eq!(img.fit(), ImageFit::Cover);
    }

    let board_inputs: Vec<_> = live_node_ids(store)
        .into_iter()
        .filter_map(|id| store.try_row::<TextInput>(id).map(|row| (id, row)))
        .collect();
    assert!(
        board_inputs.len() >= 2,
        "expected search + custom task TextInputs on Board, got {}",
        board_inputs.len()
    );
    for (id, inp) in &board_inputs {
        let mask = store
            .try_row::<Interaction>(*id)
            .map_or(0, |i| i.event_mask());
        assert_ne!(
            u64::from(mask) & EventMask::TEXT_CHANGE,
            0,
            "TextInput must subscribe to EventMask::TEXT_CHANGE"
        );
        assert!(!inp.placeholder().is_empty());
    }

    let html_board = h.html();
    assert!(html_board.contains("<img "));
    assert!(html_board.contains("<input "));

    h.press("nav-metrics");
    let canvas_id = h
        .find_by_test_id("metrics-sparkline-canvas")
        .expect("Canvas node in Metrics view");
    let canvas_row = h
        .surface
        .store()
        .try_row::<Canvas>(canvas_id)
        .expect("Canvas table row");
    let dl = h
        .surface
        .store()
        .resource::<DrawList>(canvas_row.draw_list())
        .expect("Canvas DrawList resource");
    assert!(
        dl.ops()
            .any(|op| matches!(op, CanvasOpRef::StrokePolyline { points, .. } if points.len() >= 8)),
        "expected telemetry StrokePolyline op in Canvas draw list, got {} ops",
        dl.len()
    );
    let raw_bytes = dl.to_owned().to_nve_draw1_bytes();
    assert!(raw_bytes.starts_with(&NVE_DRAW_MAGIC));
    assert!(DrawListDecoder::decode(&raw_bytes).expect("decode NVE_DRAW1").len() >= 7);
    assert!(h.html().contains("<canvas "));

    // Verify NVE2 wire roundtrip of the Canvas + DrawList resource through StudioWorkspaceModule.
    register_studio_tables();
    let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let metrics_entries = [NveMapEntry::new("view", NveValue::from_str("metrics"))];
    // SAFETY: `metrics_entries` outlives the synchronous `set_props` call.
    assert!(app.set_props(unsafe { NvePropsRef::from_slice(&metrics_entries) }));
    let wire_bytes = app.render(FrameId(1)).to_vec();

    let mut replica = NodeStore::replica();
    let mut decoder = WireDecoder::default();
    let mut changes = ChangeSet::default();
    let mut tx = replica.tx();
    decoder.apply(&mut tx, &wire_bytes).expect("decode metrics NVE2");
    tx.commit_into(&mut changes);

    let saw_canvas = live_node_ids(&replica).into_iter().any(|id| {
        replica
            .try_row::<Canvas>(id)
            .and_then(|c| replica.resource::<DrawList>(c.draw_list()))
            .is_some_and(|list| list.len() >= 7)
    });
    assert!(saw_canvas, "expected Canvas with >=7 DrawList ops in decoded NVE2 replica");
}

#[test]
fn nws1_v2_xxh64_corruption_rejection_bitflips_truncation_and_v1_rejection() {
    let mut state = WorkspaceState::default();
    assert!(state.create_custom_task("Verify XXH64 persistence"));
    state.send_quick_reply(ChannelKind::Engineering, "NWS1 v2 integrity gate");
    state.select_view(ActiveView::Docs);
    assert!(state.toggle_doc_checklist(7));
    let added = state
        .add_doc_block(DocBlockKind::Callout, "XXH64 checksummed block")
        .expect("add doc block");
    assert!(state.update_doc_block_text(added, "Edited checksummed callout"));
    state.toggle_doc_markdown_preview();
    state.select_view(ActiveView::Workbench);
    state.set_workbench_preset(WorkbenchPreset::Tablet);
    state.toggle_workbench_chrome();
    state.select_workbench_host("flutter");
    state.set_accent(AccentTheme::Amber);
    state.set_radius(CornerRadiusMode::Pill);
    state.set_density(DensityMode::Compact);
    state.cycle_sync_transport();

    let encoded = state.to_bytes();
    assert!(encoded.starts_with(&WORKSPACE_STATE_BIN_MAGIC));
    assert_eq!(
        u16::from_le_bytes([encoded[4], encoded[5]]),
        WORKSPACE_STATE_BIN_VERSION
    );
    assert_eq!(WORKSPACE_STATE_BIN_VERSION, 2);

    let decoded = WorkspaceState::from_bytes(&encoded).expect("valid NWS1 v2 decode");
    assert_eq!(decoded, state);

    let len = encoded.len();
    let mut flip_positions = vec![0, 1, 3, 4, 5, 6, 16, len / 4, len / 2, (len * 3) / 4];
    for tail in (len - 8)..len {
        flip_positions.push(tail);
    }
    for pos in flip_positions {
        let mut mutated = encoded.clone();
        mutated[pos] ^= 0x80;
        assert!(
            WorkspaceState::from_bytes(&mutated).is_none(),
            "single-bit flip at byte {pos}/{len} must be rejected by XXH64"
        );
    }

    for cut in [0, 1, 4, 6, 13, 14, len / 2, len - 9, len - 8, len - 1] {
        assert!(
            WorkspaceState::from_bytes(&encoded[..cut]).is_none(),
            "truncated NWS1 v2 slice of len {cut} must be rejected"
        );
    }

    let mut with_extra = encoded.clone();
    with_extra.push(0x42);
    assert!(WorkspaceState::from_bytes(&with_extra).is_none());

    let mut forged_v1 = encoded.clone();
    forged_v1[4..6].copy_from_slice(&1u16.to_le_bytes());
    let crc = xxh64(&forged_v1[..len - 8], 0);
    forged_v1[len - 8..].copy_from_slice(&crc.to_le_bytes());
    assert!(
        WorkspaceState::from_bytes(&forged_v1).is_none(),
        "v1 format must be strictly rejected"
    );
}

#[test]
fn workspace_store_cold_save_torn_header_compaction_and_full_restart_recovery() {
    let dir = unique_test_dir("persist");
    let cfg = PersistConfig {
        initial_capacity: 128 * 1024,
        compact_headroom: 64 * 1024,
        auto_compact_every: 4,
        sync_on_save: true,
    };

    let mut store = WorkspaceStore::open_with_config(&dir, cfg).expect("cold open store");
    assert!(store.is_enabled());
    assert!(!store.recovery_stats().loaded_from_disk);
    assert_eq!(store.generation(), 1);
    assert!(dir.join(STATE_ARENA_FILE_NAME).exists());

    let mut state = store.initial_state().clone();
    assert!(state.create_custom_task("Ship local-first MmapOffsetArena persistence"));
    state.send_quick_reply(ChannelKind::Engineering, "Arena commit #1 verified");
    let out1 = store.save(&state).expect("arena commit 1");
    assert_eq!(out1.generation, 2);
    assert!(!out1.compacted);

    state.select_view(ActiveView::Docs);
    assert!(state.toggle_doc_checklist(7));
    let added_blk = state
        .add_doc_block(DocBlockKind::Code, "WorkspaceStore::open(dir)")
        .expect("add code block");
    assert!(state.update_doc_block_text(4, "Updated binary CommandBuffer invariant."));
    let out2 = store.save(&state).expect("arena commit 2");
    assert_eq!(out2.generation, 3);
    assert!(!out2.compacted);
    let state_at_gen3 = state.clone();

    state.set_accent(AccentTheme::Violet);
    state.set_radius(CornerRadiusMode::Pill);
    state.set_density(DensityMode::Compact);
    state.select_view(ActiveView::Workbench);
    state.set_workbench_preset(WorkbenchPreset::Mobile);
    state.select_workbench_host("skia-metal");
    let out3 = store.save(&state).expect("arena commit 3");
    assert_eq!(out3.generation, 4);
    assert!(!out3.compacted);
    let active_slot_gen4 = out3.active_slot;
    drop(store);

    {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join(STATE_ARENA_FILE_NAME))
            .expect("open state.nvemmap for torn-slot injection");
        let slot_off = (active_slot_gen4 * ARENA_HEADER_SLOT_SIZE) as u64;
        f.seek(SeekFrom::Start(slot_off)).unwrap();
        f.write_all(&[0xDEu8; ARENA_HEADER_SLOT_SIZE]).unwrap();
        f.sync_all().unwrap();
    }

    let mut store_reopened =
        WorkspaceStore::open_with_config(&dir, cfg).expect("recover torn header slot");
    let stats = store_reopened.recovery_stats();
    assert!(stats.loaded_from_disk);
    assert!(!stats.recovered_from_corrupt_arena);
    assert_eq!(stats.generation, 3);
    assert_eq!(store_reopened.initial_state(), &state_at_gen3);

    let mut state_after_compact = store_reopened.initial_state().clone();
    state_after_compact.set_accent(AccentTheme::Violet);
    state_after_compact.set_radius(CornerRadiusMode::Pill);
    state_after_compact.set_density(DensityMode::Compact);
    state_after_compact.send_quick_reply(ChannelKind::Design, "Post-compaction message");
    assert!(state_after_compact.toggle_doc_checklist(6));
    let mut saw_compact = false;
    for _ in 0..4 {
        let out = store_reopened
            .save(&state_after_compact)
            .expect("arena commit triggering compaction");
        saw_compact |= out.compacted;
    }
    assert!(saw_compact, "4th commit must trigger auto-compaction");
    assert!(store_reopened.compaction_count() >= 1);
    assert_eq!(
        store_reopened.load_state().expect("load_state after compact"),
        state_after_compact
    );
    drop(store_reopened);

    let store_cold =
        WorkspaceStore::open_with_config(&dir, cfg).expect("cold restart after compaction");
    let cold_stats = store_cold.recovery_stats();
    assert!(cold_stats.loaded_from_disk);
    let restored = store_cold.initial_state();
    assert_eq!(restored, &state_after_compact);
    assert!(restored
        .current
        .tasks
        .iter()
        .any(|t| t.title == "Ship local-first MmapOffsetArena persistence"));
    assert!(restored
        .current
        .eng_messages
        .iter()
        .any(|m| m.body == "Arena commit #1 verified"));
    assert!(restored
        .current
        .design_messages
        .iter()
        .any(|m| m.body == "Post-compaction message"));
    assert_eq!(restored.current.studio.accent, AccentTheme::Violet);
    assert_eq!(restored.current.studio.radius, CornerRadiusMode::Pill);
    assert_eq!(restored.current.studio.density, DensityMode::Compact);
    let doc1 = restored
        .current
        .docs
        .iter()
        .find(|d| d.id == 1)
        .expect("doc 1 preserved");
    assert!(doc1
        .blocks
        .iter()
        .any(|b| b.id == 7 && matches!(b.kind, DocBlockKind::Checklist { checked: true })));
    assert!(doc1
        .blocks
        .iter()
        .any(|b| b.id == 6 && matches!(b.kind, DocBlockKind::Checklist { checked: false })));
    assert!(doc1
        .blocks
        .iter()
        .any(|b| b.id == added_blk && b.text == "WorkspaceStore::open(dir)"));
    drop(store_cold);

    std::fs::write(dir.join(STATE_ARENA_FILE_NAME), b"CORRUPTED_ARENA_BYTES")
        .expect("corrupt arena file");
    let store_corrupt =
        WorkspaceStore::open_with_config(&dir, cfg).expect("fallback on corrupt arena");
    assert!(store_corrupt.recovery_stats().recovered_from_corrupt_arena);
    assert_eq!(store_corrupt.initial_state(), &WorkspaceState::default());
    drop(store_corrupt);

    let mod_dir = unique_test_dir("module-persist");
    {
        let mut app = StudioWorkspaceModule::create_with_persist_dir(SurfaceId::ROOT, &mod_dir)
            .expect("create module");
        let _ = app.render(FrameId(1));
        let docs_entries = [
            NveMapEntry::new("accent", NveValue::from_str("green")),
            NveMapEntry::new("view", NveValue::from_str("docs")),
        ];
        // SAFETY: `docs_entries` outlives the synchronous `set_props` call.
        assert!(app.set_props(unsafe { NvePropsRef::from_slice(&docs_entries) }));
        let _ = app.render(FrameId(2));
        assert!(app.persist_store().expect("persist store").generation() >= 2);
    }
    {
        let app2 = StudioWorkspaceModule::create_with_persist_dir(SurfaceId::ROOT, &mod_dir)
            .expect("reopen module");
        let snap = app2.state_snapshot();
        assert_eq!(snap.current.studio.accent, AccentTheme::Green);
        assert_eq!(snap.current.active_view, ActiveView::Docs);
    }

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&mod_dir);
}

#[test]
fn docs_view_interactive_blocks_textinput_markdown_export_and_time_travel() {
    let mut h = MultiBackendHarness::new(1280.0, 800.0);

    assert!(h.find_by_test_id("nav-docs").is_some());
    h.press("nav-docs");
    assert!(h.ascii().contains("Workspace Docs"));
    assert!(h.ascii().contains("RFC-042: Zero-Copy NveSceneView"));
    assert!(h.ascii().contains("Release Checklist: v1.0"));
    assert!(h.ascii().contains("1/2 checks"));
    h.assert_no_horizontal_overflow("docs-1280x800-initial");

    h.press("doc-tab-2");
    assert!(h.ascii().contains("Pre-Flight Gates"));
    assert!(h.ascii().contains("2/3 checks"));
    h.press("doc-tab-1");
    assert!(h.ascii().contains("Binary ABI Invariants"));

    h.press("doc-check-7");
    assert!(h.ascii().contains("2/2 checks"));

    h.press("doc-block-edit-4");
    assert!(h.find_by_test_id("doc-block-input-4").is_some());
    h.type_text("doc-block-input-4", "Flat zero-copy binary frames everywhere.");
    h.press("doc-block-save-4");
    assert!(h.ascii().contains("Flat zero-copy binary frames everywhere."));
    assert!(h.find_by_test_id("doc-block-input-4").is_none());

    h.press("doc-new-block-kind");
    h.type_text("doc-new-block-input", "Verify crash-safe .nvewal compaction");
    h.press("doc-new-block-submit");
    assert!(h.ascii().contains("Verify crash-safe .nvewal compaction"));
    assert!(h.ascii().contains("2/3 checks"));

    h.press("doc-block-kind-5");
    h.press("doc-block-up-5");

    assert!(h.find_by_test_id("doc-block-3").is_some());
    h.press("doc-block-del-3");
    assert!(h.find_by_test_id("doc-block-3").is_none());
    h.press("tt-undo");
    assert!(
        h.find_by_test_id("doc-block-3").is_some(),
        "Time-Travel Undo must restore deleted doc block"
    );
    h.press("tt-redo");
    assert!(
        h.find_by_test_id("doc-block-3").is_none(),
        "Time-Travel Redo must re-apply block deletion"
    );

    h.press("doc-export-md");
    assert!(h.find_by_test_id("doc-markdown-preview").is_some());
    let tree_md = h.ascii();
    assert!(tree_md.contains("# RFC-042: Zero-Copy NveSceneView"));
    assert!(tree_md.contains("- [x] Enable crash-safe NVERELOC + MmapOffsetArena local persistence in Studio"));
    assert!(tree_md.contains("- [ ] Verify crash-safe .nvewal compaction"));
    h.assert_no_horizontal_overflow("docs-1280x800-with-markdown");

    let default_state = WorkspaceState::default();
    let md = default_state.active_doc_markdown();
    assert!(md.starts_with("# RFC-042: Zero-Copy NveSceneView"));
    assert!(md.contains("\n\n> Status: Accepted"));
    assert!(md.contains("\n\n## Binary ABI Invariants\n\n"));
    assert!(md.contains("```\nnve_get_interface(NVE_IFACE_SURFACE) -> submit_frame(&[u8])\n```"));
    assert!(md.contains("- [x] Eliminate JSON scene descriptors"));
    assert!(md.contains("- [ ] Enable crash-safe NVERELOC"));

    let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let _ = app.render(FrameId(1));
    let cmd_k = [NveDirectEvent::key_down(
        SurfaceId::ROOT,
        ROOT_NODE_ID,
        1000,
        "k",
        "",
        pack_key_modifiers(true, false, false, false, false),
        0,
        false,
    )];
    // SAFETY: `cmd_k` outlives the synchronous `dispatch_events` call.
    assert!(app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&cmd_k) }));
    assert!(app.state_snapshot().palette.open);

    let mut h_mob = MultiBackendHarness::new(393.0, 852.0);
    h_mob.press("mobile-toggle-nav");
    h_mob.press("nav-docs");
    assert!(h_mob.ascii().contains("Workspace Docs"));
    h_mob.assert_no_horizontal_overflow("docs-393x852-mobile");
}

#[test]
fn workbench_view_multi_viewport_presets_live_telemetry_and_no_overflow() {
    let mut h = MultiBackendHarness::new(1280.0, 800.0);
    assert!(h.find_by_test_id("nav-workbench").is_some());
    h.press("nav-workbench");
    assert!(h.ascii().contains("Meta Workbench"));
    assert!(h.ascii().contains("CONNECTED HOST SURFACES (9)"));

    let desk_id = h.find_by_test_id("wb-frame-desktop").expect("desktop frame");
    let tab_id = h.find_by_test_id("wb-frame-tablet").expect("tablet frame");
    let mob_id = h.find_by_test_id("wb-frame-mobile").expect("mobile frame");
    let r_desk = h.headless.layout_of(desk_id).expect("desktop rect");
    let r_tab = h.headless.layout_of(tab_id).expect("tablet rect");
    let r_mob = h.headless.layout_of(mob_id).expect("mobile rect");
    assert!(
        r_desk.x < r_tab.x && r_tab.x < r_mob.x,
        "Desktop, Tablet, Mobile frames must sit side-by-side at 1280x800: desk={r_desk:?}, tab={r_tab:?}, mob={r_mob:?}"
    );
    assert!(h.ascii().contains("1280x800"));
    assert!(h.ascii().contains("820x1180"));
    assert!(h.ascii().contains("390x844"));
    h.assert_no_horizontal_overflow("workbench-1280x800-all");

    assert_eq!(HOST_SURFACES.len(), 9);
    for spec in HOST_SURFACES {
        let tid = format!("wb-host-{}", spec.slug);
        assert!(
            h.find_by_test_id(&tid).is_some(),
            "missing connected host surface card '{tid}'"
        );
        assert!(
            h.ascii().contains(spec.name),
            "workbench missing host surface name '{}'",
            spec.name
        );
    }
    assert!(h.ascii().contains("LAN QUIC"));
    assert!(h.ascii().contains("3.8 KB/s"));
    assert!(h.ascii().contains("6.4x"));

    h.press("accent-violet");
    h.press("radius-pill");
    h.press("layout-yoga");
    let tree_tokens = h.ascii();
    assert!(tree_tokens.contains("Violet | 18px | Normal | Yoga | 4 issues | 2 docs"));

    h.press("wb-sync-cycle");
    let tree_sync = h.ascii();
    assert!(tree_sync.contains("USB Mux"));
    assert!(tree_sync.contains("2.4 KB/s"));
    assert!(tree_sync.contains("7.2x"));

    h.press("wb-preset-desktop");
    assert!(h.find_by_test_id("wb-frame-desktop").is_some());
    assert!(h.find_by_test_id("wb-frame-tablet").is_none());
    assert!(h.find_by_test_id("wb-frame-mobile").is_none());
    h.assert_no_horizontal_overflow("workbench-1280x800-desktop");

    h.press("wb-preset-tablet");
    assert!(h.find_by_test_id("wb-frame-desktop").is_none());
    assert!(h.find_by_test_id("wb-frame-tablet").is_some());
    assert!(h.find_by_test_id("wb-frame-mobile").is_none());
    h.assert_no_horizontal_overflow("workbench-1280x800-tablet");

    h.press("wb-preset-mobile");
    assert!(h.find_by_test_id("wb-frame-desktop").is_none());
    assert!(h.find_by_test_id("wb-frame-tablet").is_none());
    assert!(h.find_by_test_id("wb-frame-mobile").is_some());
    h.assert_no_horizontal_overflow("workbench-1280x800-mobile");

    h.press("wb-preset-all");
    assert!(h.find_by_test_id("wb-frame-desktop").is_some());
    assert!(h.find_by_test_id("wb-frame-tablet").is_some());
    assert!(h.find_by_test_id("wb-frame-mobile").is_some());

    h.press("wb-toggle-chrome");
    assert!(!h.ascii().contains("1280x800"));
    h.press("wb-toggle-chrome");
    assert!(h.ascii().contains("1280x800"));

    h.press("wb-host-skia-metal");
    assert!(h.ascii().contains("Host -> Skia Metal"));

    let mut h_mob = MultiBackendHarness::new(393.0, 852.0);
    h_mob.press("mobile-toggle-nav");
    h_mob.press("nav-workbench");
    assert!(h_mob.ascii().contains("Meta Workbench"));
    assert!(h_mob.find_by_test_id("wb-frame-desktop").is_some());
    assert!(h_mob.find_by_test_id("wb-frame-tablet").is_some());
    assert!(h_mob.find_by_test_id("wb-frame-mobile").is_some());
    h_mob.assert_no_horizontal_overflow("workbench-393x852-all");

    h_mob.press("wb-preset-mobile");
    h_mob.assert_no_horizontal_overflow("workbench-393x852-mobile");
}
