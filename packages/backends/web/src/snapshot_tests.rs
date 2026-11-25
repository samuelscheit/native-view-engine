//! Unit tests for 0ms Web Instant Snapshot & State Restore (`WebSnapshotPayload`,
//! `WebSnapshotStore`, quota/size guards, schema invalidation, `?reset=1`, inline bootstrap
//! `<script data-nve-snapshot-boot>` size budget, and cold SSR shell injection).

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, LayoutProps, RawPropValue, StyleProps,
    ROOT_NODE_ID,
};
use nve_engine::RenderBackend;

use crate::snapshot::{
    decode_state_envelope, encode_state_envelope, query_has_reset_param,
    snapshot_and_replay_script_tags, snapshot_bootstrap_script_tag, snapshot_storage_key,
    state_storage_key, MemorySnapshotStorage, SnapshotError, SnapshotStorage, WebSnapshotPayload,
    WebSnapshotStore, SNAPSHOT_BOOTSTRAP_MAX_BYTES, WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG,
};
use crate::ssg::{inject_ssg_into_shell_html, WebSsgDocumentOptions};
use crate::WebBackend;

fn sample_ssr_backend() -> WebBackend {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Text, None);
    b.set_raw_prop(2, "accessibilityRole", &RawPropValue::String("heading".into()));
    b.set_raw_prop(2, "ariaLevel", &RawPropValue::I64(1));
    b.set_style(
        2,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_text(2, "Aether · Studio ⚡ — \"quoted\" & <safe>");
    b.create_node(3, ComponentKind::ScrollView, None);
    b.set_layout(3, &LayoutProps::default());
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();
    web
}

#[test]
fn snapshot_bootstrap_scripts_enforce_strict_900_byte_budget() {
    assert!(
        WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.len() < SNAPSHOT_BOOTSTRAP_MAX_BYTES,
        "WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG ({} B) must be < {SNAPSHOT_BOOTSTRAP_MAX_BYTES} B",
        WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.len()
    );
    assert!(WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.starts_with("<script data-nve-snapshot-boot>"));
    assert!(WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.ends_with("</script>"));
    assert!(WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.contains("reset"));
    assert!(WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.contains("nveRestored"));
    assert!(WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.contains("nveHashes"));

    let single_app = snapshot_bootstrap_script_tag("nve-root", "studio", 2);
    assert!(
        single_app.len() < SNAPSHOT_BOOTSTRAP_MAX_BYTES,
        "single-app snapshot bootstrap ({} B) must be < {SNAPSHOT_BOOTSTRAP_MAX_BYTES} B",
        single_app.len()
    );
    assert!(single_app.contains("getElementById('nve-root')"));
    assert!(single_app.contains(":studio:v2"));

    let paired = snapshot_and_replay_script_tags("app", "studio", 2);
    assert!(paired.contains("<script data-nve-snapshot-boot>"));
    assert!(paired.contains("<script data-nve-replay>"));
    assert!(paired.contains("__nveReplayDrain"));
}

#[test]
fn snapshot_payload_roundtrip_handles_unicode_emojis_scrolls_and_rejects_corruption() {
    let web = sample_ssr_backend();
    let ssr = web.render_ssr();
    let payload = WebSnapshotPayload::from_ssr(&ssr, 2, vec![(0, 240), (2, 1080)]);

    let encoded = payload.encode_envelope();
    assert!(encoded.starts_with("NVWS1|2|"));
    let decoded = WebSnapshotPayload::decode_envelope(&encoded, 2)
        .expect("valid Unicode/emoji NVWS1 envelope must decode cleanly");
    assert_eq!(decoded, payload);

    // Schema version mismatch must be rejected cleanly.
    assert!(WebSnapshotPayload::decode_envelope(&encoded, 3).is_none());
    assert!(WebSnapshotPayload::decode_envelope(&encoded, 1).is_none());

    // Truncated envelope must be rejected cleanly.
    let truncated = &encoded[..encoded.len() - 5];
    assert!(WebSnapshotPayload::decode_envelope(truncated, 2).is_none());

    // Trailing garbage must be rejected cleanly.
    let trailing = format!("{encoded}EXTRA");
    assert!(WebSnapshotPayload::decode_envelope(&trailing, 2).is_none());

    // Garbage / wrong magic / empty inputs must never panic.
    for bad in ["", "{}", "NVWS1|", "NVWS1|2|bad\n", "NVWS2|2|0|0|0|0|0\nx"] {
        assert!(WebSnapshotPayload::decode_envelope(bad, 2).is_none());
    }
}

#[test]
fn binary_state_envelope_roundtrips_arbitrary_bytes_and_rejects_corrupt_or_wrong_version() {
    let sample: Vec<u8> = (0u8..=255).collect();
    let encoded = encode_state_envelope(2, &sample);
    assert!(encoded.starts_with("NVB1|2|"));
    assert_eq!(decode_state_envelope(&encoded, 2), Some(sample));

    // Empty and 1/2/3-byte padding edge cases.
    for bytes in [
        &b""[..],
        &b"A"[..],
        &b"AB"[..],
        &b"ABC"[..],
        &b"NWS1\x02\x00\xff\x00"[..],
    ] {
        let env = encode_state_envelope(1, bytes);
        assert_eq!(decode_state_envelope(&env, 1).as_deref(), Some(bytes));
    }

    // Version mismatch and corrupt base64 fail cleanly.
    assert!(decode_state_envelope(&encoded, 3).is_none());
    assert!(decode_state_envelope("NVB1|2|!!!not-base64!!!", 2).is_none());
    assert!(decode_state_envelope("NVB1|2|abc", 2).is_none());
    assert!(decode_state_envelope("bad-header", 2).is_none());
}

#[test]
fn snapshot_store_save_load_schema_invalidation_reset_and_quota_guards() {
    let web = sample_ssr_backend();
    let ssr = web.render_ssr();
    let payload = WebSnapshotPayload::from_ssr(&ssr, 2, vec![(0, 64)]);
    let state_bytes = b"NWS1\x02\x00hello-binary-state".to_vec();

    let mut store = WebSnapshotStore::new(MemorySnapshotStorage::new(), "studio", 2);
    let (frame_len, state_len) = store
        .save_all(&payload, &state_bytes)
        .expect("save_all must succeed within default 512 KiB limits");
    assert!(frame_len > 0 && state_len > 0);
    assert_eq!(store.frame_key(), snapshot_storage_key("studio", 2));
    assert_eq!(store.state_key(), state_storage_key("studio", 2));

    assert_eq!(store.load_frame(), Some(payload.clone()));
    assert_eq!(store.load_state(), Some(state_bytes.clone()));

    // Schema bump invalidates previous version cleanly without panic.
    let v3_store = WebSnapshotStore::new(store.storage().clone(), "studio", 3);
    assert!(v3_store.load_frame().is_none());
    assert!(v3_store.load_state().is_none());

    // `?reset=1` clears ALL versions for this app_slug while preserving other apps.
    let mut multi = store.storage().clone();
    multi.set(&snapshot_storage_key("studio", 1), "stale-v1").unwrap();
    multi.set(&snapshot_storage_key("counter", 1), "keep-counter").unwrap();
    let mut studio_store = WebSnapshotStore::new(multi, "studio", 2);
    assert!(!studio_store.handle_url_query("?app=studio&ws=off"));
    assert!(studio_store.load_state().is_some());
    assert!(query_has_reset_param("?app=studio&reset=1&ws=off"));
    assert!(studio_store.handle_url_query("?app=studio&reset=1&ws=off"));
    assert!(studio_store.load_frame().is_none());
    assert!(studio_store.load_state().is_none());
    assert!(studio_store.storage().get(&snapshot_storage_key("studio", 1)).is_none());
    assert_eq!(
        studio_store.storage().get(&snapshot_storage_key("counter", 1)).as_deref(),
        Some("keep-counter")
    );

    // Size limit guard prevents QuotaExceededError crashes.
    let mut tiny_limit = WebSnapshotStore::new(MemorySnapshotStorage::new(), "studio", 2)
        .with_limits(32, 32);
    assert!(matches!(
        tiny_limit.save_frame(&payload),
        Err(SnapshotError::SizeLimitExceeded { .. })
    ));
    assert!(matches!(
        tiny_limit.save_all(&payload, &state_bytes),
        Err(SnapshotError::SizeLimitExceeded { .. })
    ));

    // Underlying browser storage quota error fails gracefully and leaves no stale frame.
    let mut quota_store = WebSnapshotStore::new(MemorySnapshotStorage::with_quota_bytes(48), "studio", 2);
    assert_eq!(
        quota_store.save_all(&payload, &state_bytes),
        Err(SnapshotError::StorageQuotaExceeded)
    );
    assert!(quota_store.load_frame().is_none());
}

#[test]
fn inject_ssg_into_shell_html_populates_cold_styles_mount_root_and_bootstrap_for_any_app() {
    let web = sample_ssr_backend();
    let ssr = web.render_ssr();
    let shell = "<!doctype html><html><head><title>App</title></head><body><main id=\"nve-root\"><div class=\"boot-fallback\">Loading...</div></main><script type=\"module\">console.log('data-nve-ssr');boot()</script></body></html>";

    let injected = inject_ssg_into_shell_html(shell, "nve-root", "custom-app", 7, &ssr);
    assert!(injected.contains("<style data-nve-base>"));
    assert!(injected.contains("<style data-nve-ssr data-nve-hashes=\""));
    assert!(injected.contains("data-nve-ssr-hash=\""));
    assert!(injected.contains("Aether · Studio ⚡"));
    assert!(!injected.contains("boot-fallback"));
    assert!(injected.contains("getElementById('nve-root')"));
    assert!(injected.contains(":custom-app:v7"));
    let boot_pos = injected.find("<script data-nve-snapshot-boot>").unwrap();
    let replay_pos = injected.find("<script data-nve-replay>").unwrap();
    let mod_pos = injected.find("<script type=\"module\">").unwrap();
    assert!(boot_pos < replay_pos && replay_pos < mod_pos);
}

#[test]
fn render_ssg_app_document_emits_generic_standalone_cold_ssg_with_instant_restore_and_replay() {
    let web = sample_ssr_backend();
    let doc = web.render_ssg_app_document(&WebSsgDocumentOptions {
        title: "Custom NVE App".into(),
        app_slug: "custom-app".into(),
        schema_version: 4,
        mount_id: "nve-root".into(),
        description: Some("Generic NVE SSG document".into()),
        body_scripts: Some("<script type=\"module\" src=\"/app.js\"></script>".into()),
        ..Default::default()
    });
    assert!(doc.starts_with("<!doctype html>"));
    assert!(doc.contains("<title>Custom NVE App</title>"));
    assert!(doc.contains("<style data-nve-base>"));
    assert!(doc.contains("<style data-nve-ssr data-nve-hashes=\""));
    assert!(doc.contains("<div id=\"nve-root\"><div data-nve-id=\"1\" data-nve-ssr-hash=\""));
    assert!(doc.contains("Aether · Studio ⚡"));
    assert!(doc.contains("<script data-nve-snapshot-boot>"));
    assert!(doc.contains(":custom-app:v4"));
    assert!(doc.contains("<script data-nve-replay>"));
    assert!(doc.contains("__nveReplayDrain"));
    let root_end = doc.find("</div>\n<script data-nve-snapshot-boot>").unwrap();
    let replay_pos = doc.find("<script data-nve-replay>").unwrap();
    let app_js_pos = doc.find("<script type=\"module\" src=\"/app.js\"></script>").unwrap();
    assert!(root_end < replay_pos && replay_pos < app_js_pos);
}
