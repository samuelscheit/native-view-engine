//! Deep integration tests verifying Aether Studio Workspace's adoption of:
//! 1. Typed `packages/framework/router` routing (`StudioRoute` deep links + Back/Forward stack sync),
//! 2. Unified Environment `TraitKey`s (`SizeClass`, `ColorScheme`, `UiDensity`, `Locale`,
//!    `StudioAccentTrait`, `StudioCompactModeTrait`, `TraitProvider`, `<TraitScope>`, and
//!    breakpoint-only `update_viewport` with zero churn on 1px desktop drags),
//! 3. `nve-host-services` + `nve-clipboard` + `nve-system-env` (`ClipboardClient` Markdown &
//!    deep-link copy + `SystemEnvClient` theme & locale sync over both direct zero-serialization
//!    `MemoryHostServices` fallback and live `NVE_IFACE_HOST = 5` FFI).

use std::sync::{Arc, Mutex};

mod common;

use common::{data_attr_in_store, find_in_store, live_node_ids, register_studio_tables};
use nve_abi::{
    EventBatchBuf, FrameId, NodeId, NveMapEntry, NvePropsRef, NveValue, SurfaceId, TimestampUs,
    ROOT_NODE_ID,
};
use nve_backend_headless::HeadlessBackend;
use nve_clipboard::{ClipboardClient, MemoryClipboard};
use nve_tables::tables::DebugId;
use nve_dioxus::prelude::*;
use nve_dioxus::traits::{ReactiveTraitStore, TraitProvider, TraitScope};
use nve_dioxus::DioxusSurface;
use nve_engine::{NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_host_services::{HostCallbackBridge, MemoryHostServices};
use nve_interop::traits::{
    ColorScheme, HighContrast, Locale, ReducedMotion, SizeClass, TraitStore, UiDensity,
};
use nve_router::Routable;
use nve_studio_workspace::theme::LayoutMetrics;
use nve_studio_workspace::{
    create_surface_with_services, resolve_studio_layout, studio_accent_override, AccentTheme,
    ActiveView, DensityMode, StudioAccentTrait, StudioCompactModeTrait, StudioHostServices,
    StudioRoute, StudioTraitPreviewChip, StudioWorkspaceModule, WorkspaceState,
};
use nve_system_env::{
    ContrastLevel, LocaleSnapshot, MemorySystemEnv, SystemEnvEvent, TextDirection, ThemeSnapshot,
};

struct StudioHarness {
    surface: DioxusSurface,
    headless: HeadlessBackend,
    store: Arc<Mutex<WorkspaceState>>,
    memory: Arc<MemoryHostServices>,
    viewport: Viewport,
    next_frame: u64,
}

impl StudioHarness {
    fn new(width: f32, height: f32) -> Self {
        let mut initial = WorkspaceState::default();
        initial.set_viewport_size(width, height);
        let store = Arc::new(Mutex::new(initial));

        let memory = Arc::new(MemoryHostServices::new());
        memory.register(MemoryClipboard::new());
        memory.register(MemorySystemEnv::with_initial(
            ThemeSnapshot {
                color_scheme: Some(ColorScheme::Dark),
                ..ThemeSnapshot::default()
            },
            LocaleSnapshot::default(),
        ));
        let host_services = Arc::new(StudioHostServices::with_memory(Arc::clone(&memory)));

        let viewport = Viewport {
            width,
            height,
            scale_factor: 1.0,
        };
        let mut headless = HeadlessBackend::new();
        headless.set_viewport(SurfaceId::ROOT, viewport);

        let mut h = Self {
            surface: create_surface_with_services(SurfaceId::ROOT, Arc::clone(&store), host_services),
            headless,
            store,
            memory,
            viewport,
            next_frame: 1,
        };
        h.render_next();
        h
    }

    fn render_next(&mut self) {
        let fid = self.next_frame;
        self.next_frame += 1;
        self.surface
            .commit_to(FrameId(fid), &mut self.headless)
            .expect("headless commit");
    }

    fn find_by_test_id(&self, test_id: &str) -> Option<NodeId> {
        find_in_store(self.surface.store(), test_id)
    }

    fn prop_str(&self, test_id: &str, prop: &str) -> Option<String> {
        let id = self.find_by_test_id(test_id)?;
        data_attr_in_store(self.surface.store(), id, prop)
    }

    fn press(&mut self, test_id: &str) {
        let node_id = self.find_by_test_id(test_id);
        assert!(
            node_id.is_some(),
            "missing testID='{test_id}'\n{}",
            self.ascii()
        );
        let node_id = node_id.expect("checked above");
        let rect = self.headless.layout_of(node_id);
        assert!(rect.is_some(), "missing layout rect for '{test_id}'");
        let rect = rect.expect("checked above");
        assert!(
            rect.width > 0.0 && rect.height > 0.0,
            "zero-sized control '{test_id}': {rect:?}"
        );
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        let hit = self.headless.hit_test(self.surface.store(), cx, cy);
        assert!(hit.is_some(), "hit_test missed '{test_id}' at ({cx}, {cy})");
        let hit = hit.expect("checked above");
        assert_eq!(hit, node_id, "hit_test mismatch for '{test_id}'");

        let mut batch = EventBatchBuf::new();
        batch.push_press(
            SurfaceId::ROOT,
            hit,
            TimestampUs(self.next_frame * 16_000),
            cx,
            cy,
        );
        let handled = self.surface.dispatch_events(batch.as_validated());
        assert!(handled, "press on '{test_id}' was not handled");
        self.render_next();
    }

    fn ascii(&self) -> String {
        self.headless.render_ascii_tree(self.surface.store())
    }

    fn assert_no_horizontal_overflow(&self, label: &str) {
        let store = self.surface.store();
        for sid in live_node_ids(store) {
            let nid = sid;
            if let Some(rect) = self.headless.layout_of(nid) {
                let tid = store.try_row::<DebugId>(sid).map(|d| d.test_id());
                assert!(
                    rect.x >= -0.5 && rect.x + rect.width <= self.viewport.width + 1.0,
                    "[{label}] horizontal overflow on Node#{nid} ({tid:?}): rect={rect:?}, vp={}",
                    self.viewport.width
                );
            }
        }
    }
}

#[test]
fn typed_studio_route_deep_links_and_back_forward_navigation_sync() {
    // 1. Round-trip every typed `StudioRoute` path, query, custom scheme, and universal link.
    let cases = [
        ("/board", StudioRoute::Board, ActiveView::Board),
        ("/task/2", StudioRoute::Task { id: 2 }, ActiveView::Board),
        (
            "/channels",
            StudioRoute::Channels { channel: None },
            ActiveView::ChannelEngineering,
        ),
        (
            "/channels?channel=design",
            StudioRoute::Channels {
                channel: Some("design".into()),
            },
            ActiveView::ChannelDesign,
        ),
        ("/docs", StudioRoute::Docs, ActiveView::Docs),
        ("/doc/2", StudioRoute::Doc { id: 2 }, ActiveView::Docs),
        ("/workbench", StudioRoute::Workbench, ActiveView::Workbench),
        ("/stream", StudioRoute::Stream, ActiveView::Stream100k),
        ("/islands", StudioRoute::Islands, ActiveView::Islands),
        ("/metrics", StudioRoute::Metrics, ActiveView::Metrics),
    ];
    for (path, expected_route, expected_view) in cases {
        let parsed = StudioRoute::parse_uri(path)
            .unwrap_or_else(|| panic!("expected valid route for {path:?}"));
        assert_eq!(parsed, expected_route);
        assert_eq!(parsed.to_path(), path);
        assert_eq!(parsed.active_view(), expected_view);

        let deep_link = parsed.deep_link_uri();
        assert_eq!(StudioRoute::parse_deep_link(&deep_link), Some(parsed.clone()));

        let universal = format!("https://studio.nve.dev{path}");
        assert_eq!(StudioRoute::parse_deep_link(&universal), Some(parsed));
    }

    assert_eq!(StudioRoute::parse_deep_link("/"), Some(StudioRoute::Board));
    for invalid in [
        "",
        "/task/not-a-number",
        "/doc/",
        "/unknown/route",
        "ftp://untrusted.example/board",
    ] {
        assert_eq!(
            StudioRoute::parse_deep_link(invalid),
            None,
            "invalid URI {invalid:?} must be rejected"
        );
    }

    // 2. Live UI navigation pushes history entries and Back/Forward stays synchronized with ActiveView.
    let mut h = StudioHarness::new(1280.0, 800.0);
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/board"));
    h.assert_no_horizontal_overflow("route-board-1280");

    // Click task key to open typed `/task/2` route.
    h.press("task-open-2");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/task/2"));
    assert_eq!(h.store.lock().unwrap().focused_task_id, Some(2));

    // Navigate to Docs -> switch to Doc #2 (`/doc/2`) -> jump to Metrics (`/metrics`).
    h.press("nav-docs");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/docs"));
    h.press("doc-tab-2");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/doc/2"));
    assert_eq!(h.store.lock().unwrap().current.active_doc_id, 2);

    h.press("nav-metrics");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/metrics"));
    assert_eq!(h.store.lock().unwrap().current.active_view, ActiveView::Metrics);

    // Walk Back through `/doc/2` -> `/docs` -> `/task/2` -> `/board`.
    h.press("nav-back");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/doc/2"));
    assert_eq!(h.store.lock().unwrap().current.active_view, ActiveView::Docs);
    assert_eq!(h.store.lock().unwrap().current.active_doc_id, 2);

    h.press("nav-back");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/docs"));
    assert_eq!(h.store.lock().unwrap().current.active_view, ActiveView::Docs);

    h.press("nav-back");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/task/2"));
    assert_eq!(h.store.lock().unwrap().current.active_view, ActiveView::Board);
    assert_eq!(h.store.lock().unwrap().focused_task_id, Some(2));

    // Walk Forward back to `/docs` -> `/doc/2`.
    h.press("nav-forward");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/docs"));
    h.press("nav-forward");
    assert_eq!(h.prop_str("workspace-root", "active-route").as_deref(), Some("/doc/2"));
    assert_eq!(h.store.lock().unwrap().current.active_doc_id, 2);

    // 3. Verify `NWS1` v2 binary codec preserves the complete router stack, cursor, and focused task.
    let snapshot_state = h.store.lock().unwrap().clone();
    let encoded = snapshot_state.to_bytes();
    let restored = WorkspaceState::from_bytes(&encoded).expect("decode NWS1 with router stack");
    assert_eq!(restored, snapshot_state);
    assert!(restored.can_navigate_back());
    assert!(restored.can_navigate_forward());
    assert_eq!(restored.current_route(), &StudioRoute::Doc { id: 2 });

    // 4. Deep-link navigation via `StudioWorkspaceModule` direct props.
    let mut module = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let _ = module.render(FrameId(1));
    let entries = [NveMapEntry::new("route", NveValue::from_str("aether://studio/doc/2"))];
    // SAFETY: `entries` outlives the synchronous `set_props` call.
    assert!(module.set_props(unsafe { NvePropsRef::from_slice(&entries) }));
    let snap = module.state_snapshot();
    assert_eq!(snap.current.active_view, ActiveView::Docs);
    assert_eq!(snap.current.active_doc_id, 2);
    assert_eq!(snap.current_route(), &StudioRoute::Doc { id: 2 });

    let stream_entries = [NveMapEntry::new("route", NveValue::from_str("/stream"))];
    // SAFETY: `stream_entries` outlives the synchronous `set_props` call.
    assert!(module.set_props(unsafe { NvePropsRef::from_slice(&stream_entries) }));
    let snap2 = module.state_snapshot();
    assert_eq!(snap2.current.active_view, ActiveView::Stream100k);
    assert_eq!(snap2.current_route(), &StudioRoute::Stream);
    assert!(snap2.can_navigate_back());
}

#[test]
fn breakpoint_only_size_class_collapse_and_zero_churn_on_1px_desktop_drags() {
    // 1. Direct `TraitStore` breakpoint tracking: 1px drags inside Expanded bump zero SizeClass revisions.
    let mut state = WorkspaceState::default();
    let _ = state.update_viewport_with_mask(1200.0, 800.0);
    assert_eq!(state.size_class(), SizeClass::Expanded);
    assert!(!state.is_mobile_portrait());

    let m = LayoutMetrics::from_studio(&state.current.studio);
    let expanded_layout = resolve_studio_layout(&state.trait_environment(), m);
    assert!(!expanded_layout.is_mobile);
    assert!(!expanded_layout.compact_header);

    let rev_before = state.trait_store().revision::<SizeClass>();
    for w in [1201.0f32, 1202.0, 1199.0, 1280.0, 1024.0] {
        let mask = state.update_viewport_with_mask(w, 800.0);
        assert!(
            !mask.size_class_changed && !mask.orientation_changed,
            "1px desktop drag to {w}x800 must not cross SizeClass/Orientation breakpoint"
        );
        assert_eq!(state.trait_store().revision::<SizeClass>(), rev_before);
    }

    // Crossing Expanded -> Medium (1024 -> 900) bumps SizeClass once and enables compact_header.
    let med_mask = state.update_viewport_with_mask(900.0, 800.0);
    assert!(med_mask.size_class_changed);
    assert_eq!(state.size_class(), SizeClass::Medium);
    assert_eq!(state.trait_store().revision::<SizeClass>(), rev_before + 1);
    let med_layout = resolve_studio_layout(&state.trait_environment(), m);
    assert!(!med_layout.is_mobile);
    assert!(med_layout.compact_header);

    // Crossing Medium -> Compact (600 -> 599) collapses to mobile portrait via SizeClass::Compact.
    let compact_mask = state.update_viewport_with_mask(599.0, 800.0);
    assert!(compact_mask.size_class_changed);
    assert!(compact_mask.orientation_changed);
    assert_eq!(state.size_class(), SizeClass::Compact);
    assert!(state.is_mobile_portrait());
    assert_eq!(state.trait_store().revision::<SizeClass>(), rev_before + 2);
    let compact_layout = resolve_studio_layout(&state.trait_environment(), m);
    assert!(compact_layout.is_mobile);
    assert!(compact_layout.compact_header);

    // 2. Live `StudioWorkspaceModule::dispatch_events`: 1px desktop drags do NOT churn surface.
    let mut app = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let _ = app.render(FrameId(1));
    let dispatch_resize = |app: &mut StudioWorkspaceModule, w: f32, h: f32, ts: u64| {
        let mut ev = EventBatchBuf::new();
        ev.push_layout(SurfaceId::ROOT, ROOT_NODE_ID, TimestampUs(ts), 0.0, 0.0, w, h);
        app.dispatch_events(ev.as_validated())
    };

    // Initial default is 960x640 (Medium) -> 1200x800 (Expanded) crosses breakpoint.
    assert!(dispatch_resize(&mut app, 1200.0, 800.0, 1000));
    assert_eq!(app.state_snapshot().size_class(), SizeClass::Expanded);

    // 1px drag 1200 -> 1201 -> 1202 stays Expanded: returns `false` (zero layout churn!).
    assert!(!dispatch_resize(&mut app, 1201.0, 800.0, 2000));
    assert!(!dispatch_resize(&mut app, 1202.0, 800.0, 3000));
    assert_eq!(app.state_snapshot().size_class(), SizeClass::Expanded);

    // Cross into Compact mobile portrait (`393x852`): triggers collapse!
    assert!(dispatch_resize(&mut app, 393.0, 852.0, 4000));
    assert_eq!(app.state_snapshot().size_class(), SizeClass::Compact);
    assert!(app.state_snapshot().is_mobile_portrait());

    register_studio_tables();
    let frame = app.render(FrameId(2)).to_vec();
    let mut host = HeadlessBackend::new();
    host.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 1.0,
        },
    );
    let mut rep = StoreWireReplica::new(host, nve_tables::tables::core_realization_order());
    rep.submit_nve2_frame(&frame).unwrap();
    let root_id = find_in_store(rep.store(), "workspace-root").expect("workspace-root");
    assert_eq!(
        data_attr_in_store(rep.store(), root_id, "size-class").as_deref(),
        Some("Compact")
    );
    assert_eq!(
        data_attr_in_store(rep.store(), root_id, "viewport-mode").as_deref(),
        Some("mobile-portrait")
    );
}

#[test]
fn custom_trait_key_and_trait_scope_override_isolation() {
    // 1. Dedicated `<TraitProvider>` + nested `<TraitScope>` custom `TraitKey` override isolation test.
    #[component]
    fn TraitScopeProbeApp() -> Element {
        let store = dioxus_core::use_hook(|| {
            let mut ts = TraitStore::new();
            ts.update_viewport(1280.0, 800.0);
            ts.set::<ColorScheme>(ColorScheme::Dark);
            ts.set::<UiDensity>(UiDensity::Comfortable);
            ts.set::<Locale>(Locale::new("en-US"));
            ts.set::<StudioAccentTrait>(AccentTheme::Cyan);
            ts.set::<StudioCompactModeTrait>(false);
            ReactiveTraitStore::new(ts)
        });

        let scoped_overrides = studio_accent_override(AccentTheme::Amber)
            .with::<StudioCompactModeTrait>(true)
            .with::<ColorScheme>(ColorScheme::Light);

        rsx! {
            TraitProvider {
                store,
                view {
                    style: "width: 100%; height: 100%; flex-direction: column; gap: 6px; padding: 8px;",
                    StudioTraitPreviewChip {
                        test_id: Some("probe-root".into()),
                    }
                    TraitScope {
                        overrides: scoped_overrides,
                        StudioTraitPreviewChip {
                            test_id: Some("probe-scoped".into()),
                        }
                    }
                    StudioTraitPreviewChip {
                        test_id: Some("probe-sibling".into()),
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, TraitScopeProbeApp);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 800.0,
            height: 400.0,
            scale_factor: 1.0,
        },
    );
    surface.commit_to(FrameId(1), &mut headless).unwrap();

    let read_prop = |tid: &str, key: &str| -> String {
        let id = find_in_store(surface.store(), tid)
            .unwrap_or_else(|| panic!("missing testID='{tid}'"));
        data_attr_in_store(surface.store(), id, key)
            .unwrap_or_else(|| panic!("missing prop '{key}' on '{tid}'"))
    };

    // Root and un-scoped sibling stay Cyan + Expanded + false.
    assert_eq!(read_prop("probe-root", "trait-accent"), "Cyan");
    assert_eq!(read_prop("probe-root", "trait-mode"), "Expanded");
    assert_eq!(read_prop("probe-root", "trait-compact-scope"), "false");

    // Nested `<TraitScope>` overrides StudioAccentTrait, StudioCompactModeTrait, and ColorScheme.
    assert_eq!(read_prop("probe-scoped", "trait-accent"), "Amber");
    assert_eq!(read_prop("probe-scoped", "trait-mode"), "CompactScope");
    assert_eq!(read_prop("probe-scoped", "trait-compact-scope"), "true");
    assert!(headless.render_ascii_tree(surface.store()).contains("Expanded | Light | en-US | Amber"));

    // Sibling after `<TraitScope>` is completely isolated from the override.
    assert_eq!(read_prop("probe-sibling", "trait-accent"), "Cyan");
    assert_eq!(read_prop("probe-sibling", "trait-mode"), "Expanded");
    assert_eq!(read_prop("probe-sibling", "trait-compact-scope"), "false");

    // 2. Verify live Studio Workspace renders both root `studio-trait-live` and scoped
    // `studio-trait-scoped` inside the docked Studio panel, updating accent reactively.
    let mut h = StudioHarness::new(1280.0, 800.0);
    assert_eq!(h.prop_str("studio-trait-live", "trait-accent").as_deref(), Some("Cyan"));
    assert_eq!(h.prop_str("studio-trait-live", "trait-mode").as_deref(), Some("Expanded"));
    assert_eq!(
        h.prop_str("studio-trait-scoped", "trait-mode").as_deref(),
        Some("CompactScope")
    );
    assert_eq!(
        h.prop_str("studio-trait-scoped", "trait-compact-scope").as_deref(),
        Some("true")
    );

    // Changing Studio accent to Violet propagates through `StudioAccentTrait` into both chips!
    h.press("accent-violet");
    assert_eq!(h.prop_str("studio-trait-live", "trait-accent").as_deref(), Some("Violet"));
    assert_eq!(h.prop_str("studio-trait-scoped", "trait-accent").as_deref(), Some("Violet"));

    // Switching Density to Compact updates built-in `UiDensity` in `TraitStore`.
    h.press("density-compact");
    assert_eq!(h.store.lock().unwrap().current.studio.density, DensityMode::Compact);
    assert_eq!(
        h.store.lock().unwrap().trait_store().get::<UiDensity>(),
        UiDensity::Compact
    );
}

#[test]
fn clipboard_client_markdown_export_and_deep_link_url_roundtrip() {
    let mut h = StudioHarness::new(1280.0, 800.0);
    let mem = Arc::clone(&h.memory);
    let clip = ClipboardClient::new(&*mem);
    assert!(!clip.has_text().unwrap());

    // Navigate to Docs -> switch to Doc #2 -> click "Copy MD" button wired to `ClipboardClient`.
    h.press("nav-docs");
    h.press("doc-tab-2");
    h.press("doc-copy-md");
    h.assert_no_horizontal_overflow("docs-copy-md-1280");

    let expected_md = h.store.lock().unwrap().active_doc_markdown();
    assert!(expected_md.starts_with("# Release Checklist: v1.0"));
    assert!(clip.has_text().unwrap());
    assert_eq!(clip.read_text().unwrap().as_deref(), Some(expected_md.as_str()));

    // Click route bar "Copy URL" -> verify `ClipboardClient` receives canonical deep-link URL
    // that round-trips cleanly back into `StudioRoute::Doc { id: 2 }`.
    h.press("btn-copy-route-url");
    let copied_url = clip
        .read_text()
        .unwrap()
        .expect("clipboard must contain copied deep-link URL");
    assert_eq!(copied_url, "aether://studio/doc/2");
    assert_eq!(
        StudioRoute::parse_deep_link(&copied_url),
        Some(StudioRoute::Doc { id: 2 })
    );
}

#[test]
fn nve_iface_host_5_ffi_callbacks_clipboard_and_system_env_sync() {
    let mut module = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let _ = module.render(FrameId(1));
    assert!(!module.host_services().is_ffi_bound());

    // Prepare a simulated native host behind `HostCallbackBridge` (`NVE_IFACE_HOST = 5`).
    let host_mem = Arc::new(MemoryHostServices::new());
    host_mem.register(MemoryClipboard::new());
    host_mem.register(MemorySystemEnv::with_initial(
        ThemeSnapshot {
            color_scheme: Some(ColorScheme::Light),
            contrast: ContrastLevel::High,
            reduced_motion: true,
            reduced_transparency: false,
            accent_rgba: 0x38BD_F8FF,
            font_scale_x1000: 1250,
            scale_factor_x100: 200,
        },
        LocaleSnapshot {
            tag: "de-DE".into(),
            language: "de".into(),
            region: "DE".into(),
            text_direction: TextDirection::Ltr,
            uses_24h_clock: true,
            timezone: "Europe/Berlin".into(),
        },
    ));
    let bridge = HostCallbackBridge::new(Arc::clone(&host_mem));

    // Bind host callbacks across the `NVE_IFACE_HOST = 5` ABI contract.
    module.bind_host(Some(bridge.callbacks()));
    assert!(module.host_services().is_ffi_bound());

    // Binding automatically synced host theme & locale via `SystemEnvClient` into `TraitStore`!
    let snap = module.state_snapshot();
    assert_eq!(snap.color_scheme(), ColorScheme::Light);
    assert!(snap.trait_store().get::<HighContrast>());
    assert!(snap.trait_store().get::<ReducedMotion>());
    assert_eq!(snap.locale().as_str(), "de-DE");
    assert!(!snap.locale().is_rtl());

    // Render frame and verify live UI reflects synced Light + de-DE traits.
    register_studio_tables();
    let frame_bytes = module.render(FrameId(2)).to_vec();
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 1280.0,
            height: 800.0,
            scale_factor: 1.0,
        },
    );
    let mut rep = StoreWireReplica::new(headless, nve_tables::tables::core_realization_order());
    rep.submit_nve2_frame(&frame_bytes).unwrap();
    let tree = rep.backend().render_ascii_tree(rep.store());
    assert!(
        tree.contains("Light | de-DE"),
        "Studio panel should render synced host ColorScheme + Locale:\n{tree}"
    );

    // Copying Markdown while FFI-bound routes through `NVE_IFACE_HOST = 5` into `host_mem`!
    let md = snap.active_doc_markdown();
    module
        .host_services()
        .copy_text(&md)
        .expect("FFI clipboard write");
    assert_eq!(
        ClipboardClient::new(&*host_mem).read_text().unwrap().as_deref(),
        Some(md.as_str())
    );

    // Deliver live typed theme (Dark) and locale (ar-EG RTL) updates directly with zero serialization.
    let dark_theme = ThemeSnapshot {
        color_scheme: Some(ColorScheme::Dark),
        contrast: ContrastLevel::Standard,
        reduced_motion: false,
        reduced_transparency: false,
        accent_rgba: 0xA78B_FAFF,
        font_scale_x1000: 1000,
        scale_factor_x100: 200,
    };
    let ar_locale = LocaleSnapshot {
        tag: "ar-EG".into(),
        language: "ar".into(),
        region: "EG".into(),
        text_direction: TextDirection::Rtl,
        uses_24h_clock: true,
        timezone: "Africa/Cairo".into(),
    };
    assert!(module.dispatch_system_env_events(&[
        SystemEnvEvent::ThemeChanged(dark_theme),
        SystemEnvEvent::LocaleChanged(ar_locale),
    ]));

    let updated = module.state_snapshot();
    assert_eq!(updated.color_scheme(), ColorScheme::Dark);
    assert_eq!(updated.locale().as_str(), "ar-EG");
    assert!(updated.locale().is_rtl());

    // Unbinding restores clean fallback to `MemoryHostServices`.
    module.bind_host(None);
    assert!(!module.host_services().is_ffi_bound());
    module
        .host_services()
        .copy_text("fallback-memory-ok")
        .expect("fallback clipboard write");
    assert_eq!(
        module.host_services().read_clipboard_text().unwrap().as_deref(),
        Some("fallback-memory-ok")
    );
}
