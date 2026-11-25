//! Deep headless + SSR integration tests for `nve-docs`.

mod common;

use std::sync::Arc;

use common::{DocsHarness, RUST_SNIPPET, build_config, build_fixture_site, build_playgrounds};
use nve_abi::{ColorRgba, RawPropValue};
use nve_backend_headless::HeadlessBackend;
use nve_backend_web::WebBackend;
use nve_dioxus::dioxus_elements::backend::RenderBackend;
use nve_docs::{
    DocsAppState, DocsRoute, DocsTheme, DocsViewport, MOBILE_BREAKPOINT_PT, TOC_BREAKPOINT_PT,
    create_docs_surface, create_docs_surface_at_route,
};
use nve_mdx::TokenKind;
use nve_router::Routable;
use nve_sdk::{ClipboardClient, ColorScheme, MemoryHostServices, ThemeSnapshot};

#[test]
fn route_normalization_and_routable_roundtrip() {
    assert_eq!(DocsRoute::new("").as_str(), "/");
    assert_eq!(
        DocsRoute::new("/guides/quickstart/").as_str(),
        "/guides/quickstart"
    );
    assert_eq!(
        DocsRoute::parse_uri("/guides/quickstart?tab=cargo#install"),
        Some(DocsRoute("/guides/quickstart".into()))
    );
    assert_eq!(
        DocsRoute::new("/guides/quickstart").to_path(),
        "/guides/quickstart"
    );
    assert_eq!(DocsRoute::new("/").href("/docs/"), "/docs");
    assert_eq!(
        DocsRoute::new("/guides/quickstart").href("/docs"),
        "/docs/guides/quickstart"
    );
    assert_eq!(
        DocsRoute::parse_link_target("/guides/quickstart#install"),
        Some((
            Some(DocsRoute::new("/guides/quickstart")),
            Some("install".into())
        ))
    );
    assert_eq!(
        DocsRoute::parse_link_target("#binary-pipeline"),
        Some((None, Some("binary-pipeline".into())))
    );
    assert_eq!(DocsRoute::parse_link_target("https://example.com"), None);
}

#[test]
fn theme_tokens_syntax_palette_ui_sync_and_system_env() {
    let dark = DocsTheme::dark();
    let light = DocsTheme::light();
    assert_eq!(dark.canvas, "#09090b");
    assert_eq!(dark.surface, "#111114");
    assert_eq!(dark.accent, "#38bdf8");
    assert_eq!(light.canvas, "#ffffff");
    assert_eq!(light.surface, "#fafafa");
    assert_eq!(light.accent, "#0284c7");

    let all_kinds = [
        TokenKind::Plain,
        TokenKind::Keyword,
        TokenKind::TypeName,
        TokenKind::StringLit,
        TokenKind::Comment,
        TokenKind::Number,
        TokenKind::Function,
        TokenKind::Macro,
        TokenKind::Attribute,
        TokenKind::Punctuation,
        TokenKind::DiffAdd,
        TokenKind::DiffDel,
    ];
    for kind in all_kinds {
        assert!(dark.token_color(kind).starts_with('#'));
        assert!(light.token_color(kind).starts_with('#'));
    }

    let ui_dark = dark.to_ui_theme();
    assert!(ui_dark.is_dark());
    assert_eq!(ui_dark.palette.surface.as_ref(), "#09090b");
    assert_eq!(ui_dark.palette.accent.as_ref(), "#38bdf8");

    let mem = Arc::new(MemoryHostServices::new());
    mem.set_theme(ThemeSnapshot {
        color_scheme: ColorScheme::Light,
        ..ThemeSnapshot::default()
    });
    assert!(!DocsTheme::from_system_env(mem.as_ref(), true).is_dark);

    mem.set_theme(ThemeSnapshot {
        color_scheme: ColorScheme::Dark,
        ..ThemeSnapshot::default()
    });
    assert!(DocsTheme::from_system_env(mem.as_ref(), false).is_dark);

    mem.set_theme(ThemeSnapshot {
        color_scheme: ColorScheme::Unspecified,
        ..ThemeSnapshot::default()
    });
    assert!(DocsTheme::from_system_env(mem.as_ref(), true).is_dark);

    let mut app_state = DocsAppState::new(
        build_config(),
        build_fixture_site(),
        build_playgrounds(),
        mem.clone(),
        DocsRoute::root(),
        DocsViewport::desktop_default(),
    );
    assert!(app_state.dark_mode);
    mem.set_theme(ThemeSnapshot {
        color_scheme: ColorScheme::Light,
        ..ThemeSnapshot::default()
    });
    assert!(app_state.sync_with_system_env());
    assert!(!app_state.dark_mode);
}

#[test]
fn desktop_1280x800_three_column_and_mobile_390x844_drawer_collapse() {
    assert_eq!(MOBILE_BREAKPOINT_PT, 960.0);
    assert_eq!(TOC_BREAKPOINT_PT, 1200.0);

    // Desktop 1280x800: navbar + desktop sidebar + article + TOC
    let desktop = DocsHarness::mount("/", DocsViewport::new(1280.0, 800.0));
    let root = desktop.find("docs-root");
    assert_eq!(desktop.raw_str(root, "viewport-mode"), Some("desktop"));
    assert_eq!(
        desktop.raw_str(desktop.find("docs-navbar"), "accessibilityRole"),
        Some("banner")
    );
    assert_eq!(
        desktop.raw_str(desktop.find("docs-sidebar-desktop"), "accessibilityRole"),
        Some("navigation")
    );
    assert_eq!(
        desktop.raw_str(desktop.find("docs-article"), "accessibilityRole"),
        Some("article")
    );
    assert_eq!(
        desktop.raw_str(desktop.find("docs-toc"), "accessibilityRole"),
        Some("complementary")
    );
    assert!(desktop.maybe_find("docs-mobile-menu-toggle").is_none());
    assert!(desktop.maybe_find("docs-sidebar-drawer").is_none());

    let toc_link = desktop.find("docs-toc-architecture-overview");
    assert_eq!(desktop.raw_str(toc_link, "accessibilityRole"), Some("link"));
    assert_eq!(
        desktop.raw_str(toc_link, "href"),
        Some("#architecture-overview")
    );

    let heading = desktop.find("docs-heading-architecture-overview");
    assert_eq!(
        desktop.raw_str(heading, "accessibilityRole"),
        Some("heading")
    );
    assert_eq!(desktop.raw_str(heading, "aria-level"), Some("2"));

    // Tablet 1024x768: sidebar visible, TOC hidden (< 1200pt)
    let tablet = DocsHarness::mount("/", DocsViewport::new(1024.0, 768.0));
    assert_eq!(
        tablet.raw_str(tablet.find("docs-root"), "viewport-mode"),
        Some("tablet")
    );
    assert!(tablet.maybe_find("docs-sidebar-desktop").is_some());
    assert!(tablet.maybe_find("docs-toc").is_none());

    // Mobile 390x844: sidebar & TOC collapsed, mobile drawer toggle opens overlay
    let mut mobile = DocsHarness::mount("/", DocsViewport::new(390.0, 844.0));
    assert_eq!(
        mobile.raw_str(mobile.find("docs-root"), "viewport-mode"),
        Some("mobile")
    );
    assert!(mobile.maybe_find("docs-sidebar-desktop").is_none());
    assert!(mobile.maybe_find("docs-toc").is_none());
    assert!(mobile.maybe_find("docs-mobile-menu-toggle").is_some());
    assert!(mobile.maybe_find("docs-sidebar-drawer").is_none());

    mobile.press("docs-mobile-menu-toggle");
    assert!(mobile.maybe_find("docs-sidebar-drawer").is_some());

    // Navigating via mobile drawer link navigates and closes drawer
    mobile.press("docs-nav-guides-quickstart");
    assert!(mobile.maybe_find("docs-sidebar-drawer").is_none());
    assert_eq!(mobile.text_of("docs-page-title"), "Quickstart");
}

#[test]
fn sidebar_and_prev_next_navigation_updates_rendered_page() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    assert_eq!(h.text_of("docs-page-title"), "Native View Engine Docs");

    // Sidebar link navigation -> /guides/quickstart
    h.press("docs-nav-guides-quickstart");
    assert_eq!(h.text_of("docs-page-title"), "Quickstart");
    assert!(h.text_of("docs-article").contains("Install Toolchain"));

    // Prev/Next pagination: Next -> Interactive Components
    assert!(
        h.text_of("docs-next-link")
            .contains("Interactive Components")
    );
    h.press("docs-next-link");
    assert_eq!(h.text_of("docs-page-title"), "Interactive Components");
    assert!(h.text_of("docs-article").contains("quantum-flux-capacitor"));

    // Prev -> Quickstart -> Overview
    h.press("docs-prev-link");
    assert_eq!(h.text_of("docs-page-title"), "Quickstart");
    h.press("docs-prev-link");
    assert_eq!(h.text_of("docs-page-title"), "Native View Engine Docs");
}

#[test]
fn cmd_k_search_modal_filters_nvs1_index_and_navigates() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    assert!(h.maybe_find("docs-search-dialog").is_none());

    // Open via Cmd+K shortcut on root
    h.keydown("docs-root", "k", true, false, false, false);
    assert!(h.maybe_find("docs-search-dialog").is_some());

    // Close via Escape, then reopen via navbar trigger button
    h.keydown("docs-search-input", "Escape", false, false, false, false);
    assert!(h.maybe_find("docs-search-dialog").is_none());

    h.press("docs-search-trigger");
    assert!(h.maybe_find("docs-search-dialog").is_some());

    // Type unique query present only in /guides/components
    h.type_text("docs-search-input", "quantum-flux-capacitor");
    let hit0_text = h.text_of("docs-search-hit-0");
    assert!(
        hit0_text.contains("Interactive Components"),
        "expected NVS1 search hit for Interactive Components, got: {hit0_text}"
    );

    // Press Enter on selected hit -> navigates directly to /guides/components & closes modal
    h.keydown("docs-search-input", "Enter", false, false, false, false);
    assert!(h.maybe_find("docs-search-dialog").is_none());
    assert_eq!(h.text_of("docs-page-title"), "Interactive Components");
}

#[test]
fn theme_toggle_updates_canvas_and_surface_colors() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    let root_id = h.find("docs-root");
    let nav_id = h.find("docs-navbar");

    assert_eq!(h.raw_str(root_id, "data-theme"), Some("dark"));
    assert_eq!(
        h.backend.node(root_id).unwrap().style.background,
        ColorRgba::rgb(0x09, 0x09, 0x0b)
    );
    assert_eq!(
        h.backend.node(nav_id).unwrap().style.background,
        ColorRgba::rgb(0x11, 0x11, 0x14)
    );

    // Toggle to light mode
    h.press("docs-theme-toggle");
    let root_id = h.find("docs-root");
    let nav_id = h.find("docs-navbar");
    assert_eq!(h.raw_str(root_id, "data-theme"), Some("light"));
    assert_eq!(
        h.backend.node(root_id).unwrap().style.background,
        ColorRgba::rgb(0xff, 0xff, 0xff)
    );
    assert_eq!(
        h.backend.node(nav_id).unwrap().style.background,
        ColorRgba::rgb(0xfa, 0xfa, 0xfa)
    );

    // Toggle back to dark mode
    h.press("docs-theme-toggle");
    let root_id = h.find("docs-root");
    assert_eq!(h.raw_str(root_id, "data-theme"), Some("dark"));
    assert_eq!(
        h.backend.node(root_id).unwrap().style.background,
        ColorRgba::rgb(0x09, 0x09, 0x0b)
    );
}

#[test]
fn code_block_copy_writes_exact_snippet_to_clipboard() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    let copy_btn = h.find_prefix("docs-code-copy-");
    assert_eq!(h.collect_text(copy_btn), "Copy");

    h.press_id(copy_btn);

    let clip = ClipboardClient::new(h.services.as_ref());
    let copied = clip
        .read_text()
        .expect("clipboard read")
        .expect("clipboard should contain copied code");
    assert_eq!(copied, RUST_SNIPPET);

    let copy_btn_after = h.find_prefix("docs-code-copy-");
    assert_eq!(h.collect_text(copy_btn_after), "Copied");
}

#[test]
fn interactive_tabs_and_live_demo_playground_work_in_page() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());

    // 1. Switch :::tabs from Cargo -> CMake
    let panel_id = h.find_prefix("docs-tab-panel-");
    assert!(h.collect_text(panel_id).contains("cargo test --workspace"));

    let second_tab = h
        .backend
        .nodes()
        .values()
        .find_map(|n| match n.raw_props.get("testID") {
            Some(RawPropValue::String(s)) if s.starts_with("docs-tab-") && s.ends_with("-1") => {
                Some(n.id)
            }
            _ => None,
        })
        .expect("second tab trigger");
    h.press_id(second_tab);

    let panel_after = h.find_prefix("docs-tab-panel-");
    assert!(
        h.collect_text(panel_after)
            .contains("ctest --test-dir cxx/build")
    );

    // 2. Verify LiveDemo island attributes & interactive Dioxus counter state
    let demo_frame = h.find("docs-demo-counter");
    assert_eq!(h.raw_str(demo_frame, "data-nve-island"), Some("counter"));
    assert_eq!(
        h.raw_str(demo_frame, "data-nve-hydrate"),
        Some("interaction")
    );
    assert_eq!(h.text_of("demo-counter-value"), "Count: 0");

    h.press("demo-counter-inc");
    assert_eq!(h.text_of("demo-counter-value"), "Count: 1");
    h.press("demo-counter-inc");
    assert_eq!(h.text_of("demo-counter-value"), "Count: 2");

    // 3. Switch LiveDemo to Code tab and back to Preview
    h.press("docs-demo-tab-code-counter");
    assert!(h.maybe_find("docs-demo-code-counter").is_some());
    assert!(h.text_of("docs-demo-code-counter").contains("use_signal"));

    h.press("docs-demo-tab-preview-counter");
    assert!(h.maybe_find("docs-demo-preview-counter").is_some());
}

#[test]
fn create_docs_surface_at_route_renders_target_page_for_ssr() {
    let services: Arc<dyn nve_sdk::HostServiceProvider> = Arc::new(MemoryHostServices::new());
    let mut surface = create_docs_surface_at_route(
        1,
        "/guides/components",
        build_config(),
        build_fixture_site(),
        build_playgrounds(),
        services.clone(),
    );
    let (_, _, frame_bytes) = surface.render_frame(1);

    let mut web = WebBackend::new();
    web.submit_frame(&frame_bytes).expect("web submit_frame");
    let html = web.render_html();
    assert!(
        html.contains("Interactive Components"),
        "SSR HTML missing page title: {html}"
    );
    assert!(html.contains("role=\"article\""), "SSR HTML missing article role");
    assert!(html.contains("role=\"banner\""), "SSR HTML missing banner role");
    assert!(
        html.contains("role=\"navigation\""),
        "SSR HTML missing navigation role"
    );

    // Also verify default root entrypoint compiles & mounts cleanly
    let mut root_surface = create_docs_surface(
        2,
        build_config(),
        build_fixture_site(),
        build_playgrounds(),
        services,
    );
    let (_, _, root_bytes) = root_surface.render_frame(1);
    let mut headless = HeadlessBackend::new();
    headless
        .submit_frame(&root_bytes)
        .expect("root submit_frame");
    assert!(
        headless
            .render_ascii_tree()
            .contains("Native View Engine Docs")
    );
}
