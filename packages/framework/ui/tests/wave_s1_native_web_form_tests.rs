//! Deep integration tests for Wave S1:
//! - Native HTML5 `<dialog closedby>`, `command`/`commandfor`, `popover`/`popovertarget`,
//!   `<details>`/`<summary>` (`Disclosure` & `Accordion` + `DisclosureSpring`), and `<Form>`
//!   emitting exact semantic tags/attrs in `WebBackend::render_ssr`, `render_html`, and hydrating
//!   with `mismatches = 0`.
//! - `<Form>` validation, dirty/touched tracking, PRG `303 See Other` zero-JS submission roundtrip,
//!   `FormDraftStore` auto-save & restore across crash-atomic `MmapOffsetArena` (`NativeStateStore`)
//!   reopen AND `MemorySnapshotStorage`, and automatic clear on verified submit.

mod common;

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use common::UiHarness;
use nve_abi::{FrameId, SurfaceId};
use nve_backend_web::{
    decode_form_urlencoded_map, MemorySnapshotStorage, SnapshotStorage, WebBackend,
};
use nve_dioxus::elements::backend::StoreBackend;
use nve_dioxus::prelude::*;
use nve_motion::MotionPreference;
use nve_dioxus::DioxusSurface;
use nve_ui::prelude::*;
use nve_web_html_web::register_web_html_systems;
use nve_ui::{
    decode_form_draft, encode_form_draft, FormDraftError, FormDraftStore,
    NativeMmapFormDraftStorage, StringKeyValueDraftBackend, StringKeyValueFormDraftStorage,
};

struct SnapshotStorageBridge(MemorySnapshotStorage);

impl StringKeyValueDraftBackend for SnapshotStorageBridge {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key)
    }

    fn set_string(&mut self, key: &str, value: &str) -> Result<(), FormDraftError> {
        self.0
            .set(key, value)
            .map_err(|_| FormDraftError::StorageQuotaExceeded)
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
}

#[component]
fn NativeWebPrimitivesShowcase() -> Element {
    let items = vec![
        MenuItem::new("copy", "Copy"),
        MenuItem::new("delete", "Delete").destructive(true),
    ];
    let faq = vec![
        AccordionItem::new("q1", "Does it work without JS?", "Yes — native HTML5 details."),
        AccordionItem::new("q2", "How are overlays styled?", "Via NVE base reset & top layer."),
    ];

    rsx! {
        ThemeProvider {
            theme: UiTheme::light(),
            Col {
                padding: 16.0, gap: 12.0,
                Dialog {
                    id: "confirm-modal",
                    test_id: "confirm-modal",
                    open: false,
                    trigger_label: "Open Confirm",
                    close_label: "Dismiss",
                    title: "Confirm Deploy",
                    description: "Ship Wave S1 to production?",
                    text { "Zero-JS native dialog body." }
                }
                Popover {
                    test_id: "info-popover",
                    open: false,
                    keep_mounted: true,
                    trigger_label: "More Info",
                    anchor_rect: Rect::new(40.0, 80.0, 120.0, 36.0),
                    viewport: Viewport::new(800.0, 600.0),
                    text { "Native Popover API content" }
                }
                DropdownMenu {
                    test_id: "row-menu",
                    open: false,
                    keep_mounted: true,
                    trigger_label: "Row Actions",
                    items: items,
                    anchor_rect: Rect::new(40.0, 140.0, 120.0, 36.0),
                    viewport: Viewport::new(800.0, 600.0),
                }
                Disclosure {
                    id: "adv-opts",
                    test_id: "adv-opts",
                    summary: "Advanced Options",
                    default_open: true,
                    text { "Zero-JS disclosure panel" }
                }
                Accordion {
                    test_id: "faq-acc",
                    exclusive: true,
                    default_open_id: Some("q1".into()),
                    items: faq,
                }
                Form {
                    id: "signup-form",
                    test_id: "signup-form",
                    method: FormMethod::Post,
                    action: "/api/signup",
                    novalidate: true,
                    FormField {
                        name: "email",
                        label: "Email",
                        input_type: "email",
                        required: true,
                        placeholder: "ada@nve.dev",
                    }
                    FormSubmitButton {
                        test_id: "signup-submit",
                        name: "intent",
                        value: "create",
                        "Create Account"
                    }
                }
            }
        }
    }
}

#[test]
fn native_overlays_disclosure_accordion_and_form_emit_exact_html5_and_hydrate_zero_mismatches() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, NativeWebPrimitivesShowcase);

    let mut web = WebBackend::new();
    register_web_html_systems(web.table_registry_mut());
    surface.commit_to(FrameId(1), &mut web).unwrap();

    let ssr = web.render_ssr(surface.store());
    let mirror = web.render_html(surface.store());

    for html in [&ssr.html, &mirror] {
        // Native <dialog> + Invoker Commands (`show-modal` & `close`).
        assert!(html.contains("<dialog "), "expected native <dialog> in {html}");
        assert!(html.contains("id=\"confirm-modal\""), "{html}");
        assert!(html.contains("closedby=\"any\""), "{html}");
        assert!(html.contains("command=\"show-modal\""), "{html}");
        assert!(html.contains("commandfor=\"confirm-modal\""), "{html}");
        assert!(html.contains("command=\"close\""), "{html}");

        // Native Popover API + DropdownMenu preserving positioning.rs geometry.
        assert!(html.contains("popover=\"auto\""), "{html}");
        assert!(html.contains("popovertarget=\"info-popover-content\""), "{html}");
        assert!(html.contains("popovertargetaction=\"toggle\""), "{html}");
        assert!(html.contains("command=\"toggle-popover\""), "{html}");
        assert!(html.contains("popovertarget=\"row-menu-menu\""), "{html}");
        assert!(html.contains("data-placement="), "{html}");
        assert!(html.contains("data-x="), "{html}");

        // Native <details> / <summary> + exclusive Accordion `<details name="...">`.
        assert!(html.contains("<details "), "{html}");
        assert!(html.contains("<summary "), "{html}");
        assert!(html.contains("name=\"faq-acc-group\""), "{html}");

        // Native <form> + <input name="email" required> + <button type="submit">.
        assert!(html.contains("<form "), "{html}");
        assert!(html.contains("method=\"post\""), "{html}");
        assert!(html.contains("action=\"/api/signup\""), "{html}");
        assert!(html.contains(" novalidate"), "{html}");
        assert!(html.contains("name=\"email\""), "{html}");
        assert!(html.contains("type=\"email\""), "{html}");
        assert!(html.contains(" required"), "{html}");
        assert!(html.contains("type=\"submit\""), "{html}");
        assert!(html.contains("name=\"intent\""), "{html}");
        assert!(html.contains("value=\"create\""), "{html}");
    }

    // Zero-flicker hydration must adopt every node with 0 mismatches.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut hydrated = WebBackend::hydrate_from_ssr(&ssr);
        register_web_html_systems(hydrated.table_registry_mut());
        hydrated
            .commit(surface.store(), surface.last_changes())
            .unwrap();
        let report = hydrated.hydration_report();
        assert!(
            report.mismatches.is_empty(),
            "expected 0 hydration mismatches, got {:?}",
            report.mismatches
        );
        assert_eq!(report.replaced_nodes, 0);
        assert!(report.adopted_nodes >= 20);
    }
}

#[test]
fn disclosure_spring_animates_smoothly_and_respects_reduced_motion() {
    let mut spring = DisclosureSpring::new(false);
    assert_eq!(spring.progress(), 0.0);
    spring.set_open(true);
    let mid = spring.step(1.0 / 60.0);
    assert!(mid > 0.0 && mid < 1.0, "expected animated progress, got {mid}");
    for _ in 0..120 {
        spring.step(1.0 / 60.0);
    }
    assert!((spring.progress() - 1.0).abs() < 0.02);

    let mut reduced = DisclosureSpring::new(false).with_motion_preference(MotionPreference::Reduce);
    reduced.set_open(true);
    assert_eq!(reduced.progress(), 1.0);
    assert!(reduced.is_settled());
}

#[component]
fn InteractiveDisclosureAndFormApp() -> Element {
    let mut submitted_log = use_signal(|| String::from("none"));
    let ctrl = use_signal(|| {
        let mut c = FormController::with_persistence("profile-form", MemoryFormDraftStorage::new());
        c.register_text_field("email", "");
        c.register_text_field("bio", "");
        c.add_text_validator("email", |v| {
            if v.contains('@') {
                Ok(())
            } else {
                Err("Must contain @".into())
            }
        });
        c
    });

    let faq = vec![
        AccordionItem::new("a", "Section A", "Body A"),
        AccordionItem::new("b", "Section B", "Body B"),
    ];

    rsx! {
        ThemeProvider {
            theme: UiTheme::light(),
            Col {
                padding: 12.0, gap: 10.0,
                Accordion {
                    test_id: "faq",
                    exclusive: true,
                    default_open_id: Some("a".into()),
                    items: faq,
                }
                Form {
                    test_id: "profile-form",
                    method: FormMethod::Post,
                    action: "/profile",
                    controller: Some(ctrl),
                    on_submit: move |payload: FormSubmitPayload| {
                        let email = payload.values.get("email").cloned().unwrap_or_default();
                        let bio = payload.values.get("bio").cloned().unwrap_or_default();
                        submitted_log.set(format!("{email}|{bio}"));
                    },
                    FormField {
                        name: "email",
                        test_id: "email-field",
                        label: "Email",
                    }
                    FormField {
                        name: "bio",
                        test_id: "bio-field",
                        label: "Bio",
                    }
                    FormSubmitButton {
                        test_id: "save-btn",
                        "Save"
                    }
                }
                text { test_id: "submitted-log", "{submitted_log}" }
            }
        }
    }
}

#[test]
fn interactive_accordion_and_form_validation_dirty_tracking_and_submit() {
    let mut h = UiHarness::new(InteractiveDisclosureAndFormApp);

    // Exclusive accordion: A starts open, pressing B closes A and opens B.
    assert_eq!(h.raw_bool(h.find("faq-item-a"), "open"), Some(true));
    assert_eq!(h.raw_bool(h.find("faq-item-b"), "open"), Some(false));
    h.press("faq-item-b-summary");
    assert_eq!(h.raw_bool(h.find("faq-item-a"), "open"), Some(false));
    assert_eq!(h.raw_bool(h.find("faq-item-b"), "open"), Some(true));

    // Form starts clean (`data-nve-dirty="false"`).
    assert_eq!(h.raw_str(h.find("profile-form"), "data-nve-dirty"), Some("false"));

    // Typing invalid email marks form dirty, blur reveals validation error, submit blocks.
    h.text_change("email-field", "invalid-email");
    assert_eq!(h.raw_str(h.find("profile-form"), "data-nve-dirty"), Some("true"));
    h.blur("email-field");
    assert_eq!(h.raw_str(h.find("email-field"), "ariaInvalid"), Some("true"));
    h.press("save-btn");
    assert_eq!(h.text_of("submitted-log"), "none");

    // Fix email + fill bio -> verified submit succeeds & commits clean baseline!
    h.text_change("email-field", "ada@nve.dev");
    h.text_change("bio-field", "Systems engineer");
    h.press("save-btn");
    assert_eq!(h.text_of("submitted-log"), "ada@nve.dev|Systems engineer");
    assert_eq!(h.raw_str(h.find("profile-form"), "data-nve-dirty"), Some("false"));
}

#[test]
fn form_draft_store_persists_across_mmap_offset_arena_crash_reopen_and_clears_on_submit() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let arena_path = std::env::temp_dir().join(format!("nve_form_draft_test_{unique}.arena"));

    // Session 1: user types dirty fields -> crash before submit!
    {
        let storage = NativeMmapFormDraftStorage::open(&arena_path).unwrap();
        let mut ctrl = FormController::with_persistence("checkout", storage);
        ctrl.register_text_field("email", "");
        ctrl.register_text_field("city", "");
        ctrl.add_text_validator("email", |v| {
            if v.contains('@') { Ok(()) } else { Err("Invalid".into()) }
        });

        ctrl.set_text("email", "grace@nve.dev");
        ctrl.set_text("city", "Zurich");
        assert!(ctrl.is_dirty());
        assert!(!ctrl.can_pop());

        // Failed server submit MUST preserve the persisted draft!
        let err = ctrl.submit_with(|_| Err("network timeout".into()));
        assert_eq!(err, Err(Some("network timeout".into())));
    }

    // Session 2: process restarts and reopens the exact same MmapOffsetArena file.
    {
        let storage = NativeMmapFormDraftStorage::open(&arena_path).unwrap();
        let mut ctrl = FormController::with_persistence("checkout", storage);
        ctrl.register_text_field("email", "");
        ctrl.register_text_field("city", "");
        assert!(ctrl.restore_draft());
        assert!(ctrl.has_restored_draft());
        assert!(ctrl.is_dirty());
        assert_eq!(ctrl.text_value("email"), "grace@nve.dev");
        assert_eq!(ctrl.text_value("city"), "Zurich");

        // Verified successful submit commits baseline AND clears draft automatically!
        let submitted = ctrl.submit_with(|vals| {
            assert_eq!(vals.get("email").map(String::as_str), Some("grace@nve.dev"));
            Ok(())
        });
        assert!(submitted.is_ok());
        assert!(!ctrl.is_dirty());
        assert!(!ctrl.has_restored_draft());
    }

    // Session 3: reopen proves draft was atomically removed on verified submit.
    {
        let storage = NativeMmapFormDraftStorage::open(&arena_path).unwrap();
        let mut ctrl = FormController::with_persistence("checkout", storage);
        ctrl.register_text_field("email", "");
        assert!(!ctrl.restore_draft());
        assert!(!ctrl.has_restored_draft());
        assert_eq!(ctrl.text_value("email"), "");
    }

    let _ = std::fs::remove_file(&arena_path);
}

#[test]
fn form_draft_store_works_with_memory_snapshot_storage_and_rejects_corruption_plus_prg_roundtrip() {
    // 1. Verify binary NVE_FORM_DRAFT_V1 binary roundtrip & tamper detection.
    let mut fields = BTreeMap::new();
    fields.insert("title".into(), "Launch Notes".into());
    fields.insert("body".into(), "Binary draft".into());
    let mut blob = encode_form_draft(&fields).unwrap();
    assert!(blob.starts_with(b"NVE_FORM_DRAFT_V1"));
    assert_eq!(decode_form_draft(&blob), Some(fields.clone()));
    // Flip one bit -> XXH64 checksum MUST reject.
    blob[20] ^= 0x01;
    assert_eq!(decode_form_draft(&blob), None);

    // 2. Persist through MemorySnapshotStorage via StringKeyValueFormDraftStorage.
    let adapter = StringKeyValueFormDraftStorage::new(SnapshotStorageBridge(MemorySnapshotStorage::new()));
    let mut store = FormDraftStore::new(adapter);
    let written = store.save_fields("article-new", &fields).unwrap();
    assert!(written > 0);
    // Stored string value uses self-documenting NVE_FORM_DRAFT_V1 prefix (never JSON!).
    let raw_stored = store
        .storage()
        .inner()
        .0
        .get("nve:form-draft:article-new")
        .unwrap();
    assert!(raw_stored.starts_with("NVE_FORM_DRAFT_V1|"));
    assert!(!raw_stored.contains('{'));
    assert_eq!(store.load_fields("article-new"), Some(fields));

    store.clear("article-new");
    assert_eq!(store.load_fields("article-new"), None);

    // 3. PRG 303 zero-JS form submission roundtrip decoding URL-encoded form body.
    let encoded_post = "email=ada%40nve.dev&bio=Hello+from+zero-JS+PRG&intent=create";
    let decoded = decode_form_urlencoded_map(encoded_post);
    assert_eq!(decoded.get("email").map(String::as_str), Some("ada@nve.dev"));
    assert_eq!(decoded.get("bio").map(String::as_str), Some("Hello from zero-JS PRG"));
    assert_eq!(decoded.get("intent").map(String::as_str), Some("create"));
}
