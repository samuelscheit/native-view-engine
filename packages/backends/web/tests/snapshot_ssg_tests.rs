//! 0ms Web Instant Snapshot & State Restore (`WebSnapshotPayload`, `WebSnapshotStore`,
//! quota/size guards, schema invalidation, `?reset=1`, inline bootstrap
//! `<script data-nve-snapshot-boot>` size budget) and cold SSG document / shell injection
//! (`</style>` breakout escaping, stable `<style data-nve-ssr>` anchor, idempotent injection).

mod support;

use nve_backend_web::snapshot::{
    MemorySnapshotStorage, SNAPSHOT_BOOTSTRAP_MAX_BYTES, SnapshotError, SnapshotStorage,
    WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG, WebSnapshotPayload, WebSnapshotStore,
    decode_state_envelope, encode_state_envelope, query_has_reset_param,
    snapshot_and_replay_script_tags, snapshot_bootstrap_script_tag, snapshot_storage_key,
    state_storage_key,
};
use nve_backend_web::ssg::{
    WebSsgDocumentOptions, inject_ssg_into_shell_html, inject_ssg_with_scripts_into_shell_html,
};
use nve_backend_web::{SsrOutput, StyleSheetCache, WebBackend};
use nve_components::tables::{Accessibility, AriaRole, Layout, Scroll, Typography};
use nve_store::{NodeId, NodeStore, Rgba};
use support::*;

const UNICODE_TITLE: &str = "Aether · Studio ⚡ — \"quoted\" & <safe>";

fn sample_ssr_backend() -> (NodeStore, WebBackend) {
    let mut web = web();
    let mut store = NodeStore::new();
    commit(&mut web, &mut store, |tx| {
        let h = text(tx, 2, UNICODE_TITLE);
        tx.edit::<Accessibility>(h).role(AriaRole::Heading).level(1);
        tx.edit::<Typography>(h).color(Rgba::WHITE).font_size(20.0);
        let scroll = node(tx, 3);
        tx.edit::<Scroll>(scroll);
        tx.edit::<Layout>(scroll);
        tx.set_children(NodeId::ROOT, &[h, scroll]);
    });
    (store, web)
}

fn sample_ssr() -> SsrOutput {
    let (store, web) = sample_ssr_backend();
    web.render_ssr(&store)
}

fn hostile_ssr(style_css: &str) -> SsrOutput {
    SsrOutput {
        base_css: "a{}</StYlE><script>alert(0)</script>",
        style_css: style_css.to_owned(),
        rule_hashes: vec!["abc".into(), "d\"e".into()],
        html: "<p>ok</p>".into(),
        root_hash: 7,
    }
}

#[test]
fn scroll_containers_carry_nve_s_marker_in_ssr_and_live_class_resolution() {
    let (store, web) = sample_ssr_backend();
    let html = web.render_ssr(&store).html;
    assert_eq!(html.matches("class=\"nve nve-s").count(), 1, "{html}");
    assert!(html.contains("data-nve-id=\"3\" class=\"nve nve-s"), "{html}");

    let mut live = StyleSheetCache::new();
    assert!(live.resolve_node(&store, NodeId(3)).classes.starts_with("nve nve-s"));
    let heading = live.resolve_node(&store, NodeId(2)).classes;
    assert!(!heading.split(' ').any(|c| c == "nve-s"), "{heading}");
    let root = live.resolve_node(&store, NodeId::ROOT).classes;
    assert!(!root.split(' ').any(|c| c == "nve-s"), "{root}");
}

#[test]
fn scroll_axes_classes_restore_automatic_min_size_of_scroll_content() {
    use nve_backend_web::NVE_BASE_RESET_CSS;
    use nve_components::tables::ScrollAxes;

    let cases = [
        (ScrollAxes::Vertical, &["nve-s", "nve-sy"][..], &["nve-sx"][..]),
        (ScrollAxes::Horizontal, &["nve-s", "nve-sx"], &["nve-sy"]),
        (ScrollAxes::Both, &["nve-s", "nve-sy", "nve-sx"], &[]),
        (ScrollAxes::None, &["nve-s"], &["nve-sy", "nve-sx"]),
    ];
    let mut web = web();
    let mut store = NodeStore::new();
    commit(&mut web, &mut store, |tx| {
        let ids: Vec<NodeId> = cases
            .iter()
            .enumerate()
            .map(|(i, (axes, ..))| {
                let id = node(tx, 10 + i as u32);
                tx.edit::<Scroll>(id).axes(*axes);
                id
            })
            .collect();
        tx.set_children(NodeId::ROOT, &ids);
    });
    let mut live = StyleSheetCache::new();
    for (i, (axes, present, absent)) in cases.into_iter().enumerate() {
        let classes = live.resolve_node(&store, NodeId(10 + i as u32)).classes;
        let tokens: Vec<&str> = classes.split(' ').collect();
        for c in present {
            assert!(tokens.contains(c), "{axes:?}: missing {c} in {classes}");
        }
        for c in absent {
            assert!(!tokens.contains(c), "{axes:?}: unexpected {c} in {classes}");
        }
    }
    // The axis rules must follow the `min-*:0` reset in the same zero-specificity layer to win.
    let reset = NVE_BASE_RESET_CSS.find("min-height:0}").expect("reset rule");
    let sy = NVE_BASE_RESET_CSS.find(":where(.nve-sy>.nve){min-height:auto}").expect("nve-sy rule");
    let sx = NVE_BASE_RESET_CSS.find(":where(.nve-sx>.nve){min-width:auto}").expect("nve-sx rule");
    assert!(reset < sy && reset < sx);
}

#[test]
fn snapshot_bootstrap_scripts_enforce_strict_900_byte_budget() {
    let boot = WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG;
    assert!(
        boot.len() < SNAPSHOT_BOOTSTRAP_MAX_BYTES,
        "{} B",
        boot.len()
    );
    assert!(boot.starts_with("<script data-nve-snapshot-boot>"));
    assert!(boot.ends_with("</script>"));
    for needle in ["reset", "nveRestored", "nveHashes"] {
        assert!(boot.contains(needle), "{needle}");
    }

    let single_app = snapshot_bootstrap_script_tag("nve-root", "studio", 2);
    assert!(
        single_app.len() < SNAPSHOT_BOOTSTRAP_MAX_BYTES,
        "{} B",
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
    let payload = WebSnapshotPayload::from_ssr(&sample_ssr(), 2, vec![(0, 240), (2, 1080)]);
    let encoded = payload.encode_envelope();
    assert!(encoded.starts_with("NVWS1|2|"));
    let decoded = WebSnapshotPayload::decode_envelope(&encoded, 2)
        .expect("valid Unicode/emoji NVWS1 envelope must decode cleanly");
    assert_eq!(decoded, payload);

    assert!(WebSnapshotPayload::decode_envelope(&encoded, 3).is_none());
    assert!(WebSnapshotPayload::decode_envelope(&encoded, 1).is_none());
    assert!(WebSnapshotPayload::decode_envelope(&encoded[..encoded.len() - 5], 2).is_none());
    assert!(WebSnapshotPayload::decode_envelope(&format!("{encoded}EXTRA"), 2).is_none());
    for bad in ["", "{}", "NVWS1|", "NVWS1|2|bad\n", "NVWS2|2|0|0|0|0|0\nx"] {
        assert!(
            WebSnapshotPayload::decode_envelope(bad, 2).is_none(),
            "{bad:?}"
        );
    }
}

#[test]
fn binary_state_envelope_roundtrips_arbitrary_bytes_and_rejects_corrupt_or_wrong_version() {
    let sample: Vec<u8> = (0u8..=255).collect();
    let encoded = encode_state_envelope(2, &sample);
    assert!(encoded.starts_with("NVB1|2|"));
    assert_eq!(decode_state_envelope(&encoded, 2), Some(sample));
    for bytes in [&b""[..], b"A", b"AB", b"ABC", b"NWS1\x02\x00\xff\x00"] {
        let env = encode_state_envelope(1, bytes);
        assert_eq!(decode_state_envelope(&env, 1).as_deref(), Some(bytes));
    }
    assert!(decode_state_envelope(&encoded, 3).is_none());
    assert!(decode_state_envelope("NVB1|2|!!!not-base64!!!", 2).is_none());
    assert!(decode_state_envelope("NVB1|2|abc", 2).is_none());
    assert!(decode_state_envelope("bad-header", 2).is_none());
}

#[test]
fn snapshot_store_save_load_schema_invalidation_reset_and_quota_guards() {
    let payload = WebSnapshotPayload::from_ssr(&sample_ssr(), 2, vec![(0, 64)]);
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

    let v3_store = WebSnapshotStore::new(store.storage().clone(), "studio", 3);
    assert!(v3_store.load_frame().is_none());
    assert!(v3_store.load_state().is_none());

    let mut multi = store.storage().clone();
    multi
        .set(&snapshot_storage_key("studio", 1), "stale-v1")
        .unwrap();
    multi
        .set(&snapshot_storage_key("counter", 1), "keep-counter")
        .unwrap();
    let mut studio_store = WebSnapshotStore::new(multi, "studio", 2);
    assert!(!studio_store.handle_url_query("?app=studio&ws=off"));
    assert!(studio_store.load_state().is_some());
    assert!(query_has_reset_param("?app=studio&reset=1&ws=off"));
    assert!(studio_store.handle_url_query("?app=studio&reset=1&ws=off"));
    assert!(studio_store.load_frame().is_none());
    assert!(studio_store.load_state().is_none());
    assert!(
        studio_store
            .storage()
            .get(&snapshot_storage_key("studio", 1))
            .is_none()
    );
    assert_eq!(
        studio_store
            .storage()
            .get(&snapshot_storage_key("counter", 1))
            .as_deref(),
        Some("keep-counter")
    );

    let mut tiny =
        WebSnapshotStore::new(MemorySnapshotStorage::new(), "studio", 2).with_limits(32, 32);
    assert!(matches!(
        tiny.save_frame(&payload),
        Err(SnapshotError::SizeLimitExceeded { .. })
    ));
    assert!(matches!(
        tiny.save_all(&payload, &state_bytes),
        Err(SnapshotError::SizeLimitExceeded { .. })
    ));

    let mut quota = WebSnapshotStore::new(MemorySnapshotStorage::with_quota_bytes(48), "studio", 2);
    assert_eq!(
        quota.save_all(&payload, &state_bytes),
        Err(SnapshotError::StorageQuotaExceeded)
    );
    assert!(quota.load_frame().is_none());
}

#[test]
fn inject_ssg_into_shell_html_populates_cold_styles_mount_root_and_bootstrap_for_any_app() {
    let ssr = sample_ssr();
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

    // Re-injecting an already injected shell is idempotent (no duplicate styles or scripts).
    let twice = inject_ssg_into_shell_html(&injected, "nve-root", "custom-app", 7, &ssr);
    assert_eq!(twice, injected);
}

#[test]
fn inject_ssg_replaces_nested_same_tag_mount_content_and_places_scripts_after_mount() {
    let ssr = hostile_ssr("");
    let shell = "<html><head></head><body><div id=\"app\"><div><div>Loading</div></div><divider/></div><footer>keep</footer></body></html>";
    let out = inject_ssg_with_scripts_into_shell_html(shell, "app", &ssr, "<script>B</script>");
    assert!(
        out.contains("<div id=\"app\"><p>ok</p></div>\n<script>B</script><footer>keep</footer>"),
        "{out}"
    );
    assert!(!out.contains("Loading"));

    // Unknown mount: HTML untouched, scripts before `</body>`.
    let out = inject_ssg_with_scripts_into_shell_html(shell, "nope", &ssr, "<script>B</script>");
    assert!(
        out.contains("Loading") && out.contains("<script>B</script>\n</body>"),
        "{out}"
    );
}

#[test]
fn ssg_bootstrap_scripts_follow_the_mount_even_when_head_loads_a_module_script() {
    let ssr = hostile_ssr("");
    let shell = "<!doctype html><html><head><script type=\"module\" src=\"/main.js\"></script></head><body><div id=\"app\"></div></body></html>";
    let out = inject_ssg_with_scripts_into_shell_html(shell, "app", &ssr, "<script data-nve-snapshot-boot>B</script>");
    let head_end = out.find("</head>").unwrap();
    let boot = out.find("<script data-nve-snapshot-boot>").unwrap();
    let mount_close = out.find("<p>ok</p></div>").unwrap() + "<p>ok</p></div>".len();
    assert!(boot > head_end && boot >= mount_close, "boot script must follow the mount: {out}");
    assert_eq!(out.matches("<script type=\"module\"").count(), 1, "{out}");
}

#[test]
fn ssg_styles_never_precede_the_doctype_when_the_shell_has_no_head() {
    let ssr = hostile_ssr("");
    // (shell, exact prefix kept ahead of the styles, element the styles must precede)
    for (shell, prefix, before) in [
        ("<!DOCTYPE html><BODY><div id=\"app\"></div></BODY>", "<!DOCTYPE html>", "<BODY>"),
        ("<!doctype html>\n<div id=\"app\"></div>", "<!doctype html>", "\n<div id=\"app\">"),
        // `<body-shell>` is a custom element, not `<body>`: the styles follow the doctype.
        (
            "<!doctype html><body-shell><div id=\"app\"></div></body-shell>",
            "<!doctype html>",
            "<body-shell>",
        ),
        (
            "<!doctype html><body-shell></body-shell><body\tclass=\"x\"><div id=\"app\"></div></body>",
            "<!doctype html><body-shell></body-shell>",
            "<body\tclass=\"x\">",
        ),
        ("<div id=\"app\"></div>", "", "<div id=\"app\">"),
    ] {
        let out = inject_ssg_with_scripts_into_shell_html(shell, "app", &ssr, "");
        let style = out.find("<style data-nve-base>").unwrap();
        assert_eq!(&out[..style], prefix, "styles must sit right after the doctype: {out}");
        assert!(out[style..].contains(before), "{out}");
    }
}

#[test]
fn ssg_script_injection_is_idempotent_for_replay_only_and_custom_scripts() {
    let ssr = hostile_ssr("");
    let shell = "<html><head></head><body><div id=\"app\"></div></body></html>";
    for scripts in [
        nve_backend_web::PRE_HYDRATION_REPLAY_SCRIPT_TAG.to_owned(),
        "<script>window.custom = 1</script>\n".to_owned(),
    ] {
        let once = inject_ssg_with_scripts_into_shell_html(shell, "app", &ssr, &scripts);
        let twice = inject_ssg_with_scripts_into_shell_html(&once, "app", &ssr, &scripts);
        assert_eq!(twice, once, "re-injection must not duplicate {scripts}");
        assert_eq!(once.matches(scripts.trim()).count(), 1, "{once}");
    }
}

#[test]
fn ssg_style_payloads_escape_breakouts_and_always_emit_one_ssr_anchor() {
    let (store, web) = sample_ssr_backend();
    for style_css in ["", ".x{}</StYlE><script>alert(1)</script>"] {
        let ssr = hostile_ssr(style_css);
        let shell = "<html><head><title>t</title></head><body><div id=\"app\"></div></body></html>";
        let doc = inject_ssg_with_scripts_into_shell_html(shell, "app", &ssr, "");
        assert_eq!(doc.matches("<style data-nve-ssr").count(), 1, "{doc}");
        assert_eq!(doc.matches("<style data-nve-base").count(), 1, "{doc}");
        assert!(doc.contains("data-nve-hashes=\"abc,d&quot;e\""), "{doc}");
        // Only the two real closers survive: every payload `</style` is escaped as `<\/`, so
        // the `<script>` text stays inert CSS.
        assert!(!doc.contains("</StYlE>"), "{doc}");
        let closers = doc.as_bytes().windows(8).filter(|w| w.eq_ignore_ascii_case(b"</style>"));
        assert_eq!(closers.count(), 2, "{doc}");
        assert!(doc.contains("a{}<\\/StYlE><script>alert(0)"), "{doc}");
        let injected = doc;
        if !style_css.is_empty() {
            assert!(injected.contains("<\\/StYlE>"), "{injected}");
        }
    }

    // Shell already carrying both style tags: never duplicated.
    let pre = "<html><head><style data-nve-base>x</style><style data-nve-ssr data-nve-hashes=\"\"></style></head><body><div id=\"app\"></div></body></html>";
    let out = inject_ssg_with_scripts_into_shell_html(pre, "app", &hostile_ssr("y{}"), "");
    assert_eq!(out.matches("<style data-nve-ssr").count(), 1);
    assert_eq!(out.matches("<style data-nve-base").count(), 1);

    // Standalone document: anchor present even when the store produced no structural CSS.
    let empty = NodeStore::new();
    let doc = web.render_ssg_app_document(&empty, &WebSsgDocumentOptions::default());
    assert_eq!(
        doc.matches("<style data-nve-ssr data-nve-hashes=\"")
            .count(),
        1,
        "{doc}"
    );
    let doc = web.render_ssg_app_document(&store, &WebSsgDocumentOptions::default());
    assert_eq!(doc.matches("<style data-nve-ssr").count(), 1);
    assert!(
        doc.contains("<div id=\"app\">") && doc.contains(":app:v1"),
        "{doc}"
    );
}

#[test]
fn render_ssg_app_document_emits_generic_standalone_cold_ssg_with_instant_restore_and_replay() {
    let (store, web) = sample_ssr_backend();
    let doc = web.render_ssg_app_document(
        &store,
        &WebSsgDocumentOptions {
            title: "Custom NVE App".into(),
            app_slug: "custom-app".into(),
            schema_version: 4,
            mount_id: "nve-root".into(),
            description: Some("Generic NVE SSG document".into()),
            canonical_url: Some("https://example.test/a?b=1&c=\"2\"".into()),
            body_scripts: Some("<script type=\"module\" src=\"/app.js\"></script>".into()),
            ..Default::default()
        },
    );
    assert!(doc.starts_with("<!doctype html>"));
    assert!(doc.contains("<title>Custom NVE App</title>"));
    assert!(doc.contains("<meta property=\"og:title\" content=\"Custom NVE App\">"));
    assert!(
        doc.contains("<meta property=\"og:description\" content=\"Generic NVE SSG document\">")
    );
    assert!(doc.contains(
        "<link rel=\"canonical\" href=\"https://example.test/a?b=1&amp;c=&quot;2&quot;\">"
    ));
    assert!(doc.contains("<meta property=\"og:url\""));
    assert!(!doc.contains("og:image"));
    assert!(doc.contains("<style data-nve-base>"));
    assert!(doc.contains("<style data-nve-ssr data-nve-hashes=\""));
    assert!(doc.contains("<div id=\"nve-root\"><div data-nve-id=\"1\" data-nve-ssr-hash=\""));
    assert!(doc.contains("Aether · Studio ⚡"));
    assert!(doc.contains("<script data-nve-snapshot-boot>"));
    assert!(doc.contains("getElementById('nve-root')"));
    assert!(doc.contains(":custom-app:v4"));
    assert!(doc.contains("__nveReplayDrain"));
    let root_end = doc.find("</div>\n<script data-nve-snapshot-boot>").unwrap();
    let replay_pos = doc.find("<script data-nve-replay>").unwrap();
    let app_js_pos = doc
        .find("<script type=\"module\" src=\"/app.js\"></script>")
        .unwrap();
    assert!(root_end < replay_pos && replay_pos < app_js_pos);
}
