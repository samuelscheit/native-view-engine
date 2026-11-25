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
use nve_clipboard::ClipboardClient;
use nve_host_services::{HostServiceProvider, MemoryHostServices};
use nve_mdx::TokenKind;
use nve_router::Routable;
use nve_system_env::{ColorScheme, MemorySystemEnv, ThemeSnapshot};

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
    let sys_env = Arc::new(MemorySystemEnv::new());
    mem.register_arc(sys_env.clone());
    sys_env.set_theme(ThemeSnapshot {
        color_scheme: ColorScheme::Light,
        ..ThemeSnapshot::default()
    });
    assert!(!DocsTheme::from_system_env(mem.as_ref(), true).is_dark);

    sys_env.set_theme(ThemeSnapshot {
        color_scheme: ColorScheme::Dark,
        ..ThemeSnapshot::default()
    });
    assert!(DocsTheme::from_system_env(mem.as_ref(), false).is_dark);

    sys_env.set_theme(ThemeSnapshot {
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
    sys_env.set_theme(ThemeSnapshot {
        color_scheme: ColorScheme::Light,
        ..ThemeSnapshot::default()
    });
    assert!(app_state.sync_with_system_env());
    assert!(!app_state.dark_mode);
}

#[test]
fn desktop_1280x800_three_column_and_mobile_390x844_drawer_collapse() {
    use nve_dioxus::prelude::MediaQuery;

    assert_eq!(MOBILE_BREAKPOINT_PT, 960.0);
    assert_eq!(TOC_BREAKPOINT_PT, 1200.0);

    // Desktop 1280x800: navbar + desktop sidebar + article + TOC + CSS-hidden mobile menu toggle
    let desktop = DocsHarness::mount("/", DocsViewport::new(1280.0, 800.0));
    let root = desktop.find("docs-root");
    assert_eq!(desktop.raw_str(root, "viewport-mode"), Some("desktop"));
    assert_eq!(
        desktop.raw_str(desktop.find("docs-navbar"), "accessibilityRole"),
        Some("banner")
    );
    let desktop_sidebar = desktop.find("docs-sidebar-desktop");
    assert_eq!(
        desktop.raw_str(desktop_sidebar, "accessibilityRole"),
        Some("navigation")
    );
    assert_eq!(
        desktop.surface.node_show_on(desktop_sidebar),
        Some(MediaQuery::min_width(960))
    );
    assert_eq!(
        desktop.raw_str(desktop.find("docs-article"), "accessibilityRole"),
        Some("article")
    );
    let desktop_toc = desktop.find("docs-toc");
    assert_eq!(
        desktop.raw_str(desktop_toc, "accessibilityRole"),
        Some("complementary")
    );
    assert_eq!(
        desktop.surface.node_show_on(desktop_toc),
        Some(MediaQuery::min_width(1200))
    );
    let desktop_menu_toggle = desktop.find("docs-mobile-menu-toggle");
    assert_eq!(
        desktop.surface.node_show_on(desktop_menu_toggle),
        Some(MediaQuery::max_width(959))
    );
    assert_eq!(
        desktop.surface.node_show_on(desktop.find("docs-section-tabs")),
        Some(MediaQuery::min_width(960))
    );
    assert_eq!(
        desktop.surface.node_show_on(desktop.find("docs-github-link")),
        Some(MediaQuery::min_width(960))
    );
    assert!(
        desktop
            .surface
            .node_responsive_rules(desktop.find("docs-article-container"))
            .iter()
            .any(|r| r.query == MediaQuery::max_width(959))
    );
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

    // Tablet 1024x768: identical DOM tree with declarative `@media` hiding TOC (< 1200pt)
    let tablet = DocsHarness::mount("/", DocsViewport::new(1024.0, 768.0));
    assert_eq!(
        tablet.raw_str(tablet.find("docs-root"), "viewport-mode"),
        Some("tablet")
    );
    assert_eq!(
        tablet.surface.node_show_on(tablet.find("docs-sidebar-desktop")),
        Some(MediaQuery::min_width(960))
    );
    assert_eq!(
        tablet.surface.node_show_on(tablet.find("docs-toc")),
        Some(MediaQuery::min_width(1200))
    );

    // Mobile 390x844: identical initial DOM tree (0 hydration mismatches), sidebar & TOC hidden via
    // CSS `@media` at T=0ms, mobile drawer toggle opens overlay with `max_width(959)`.
    let mut mobile = DocsHarness::mount("/", DocsViewport::new(390.0, 844.0));
    assert_eq!(
        mobile.raw_str(mobile.find("docs-root"), "viewport-mode"),
        Some("mobile")
    );
    assert_eq!(
        mobile.surface.node_show_on(mobile.find("docs-sidebar-desktop")),
        Some(MediaQuery::min_width(960))
    );
    assert_eq!(
        mobile.surface.node_show_on(mobile.find("docs-toc")),
        Some(MediaQuery::min_width(1200))
    );
    assert_eq!(
        mobile.surface.node_show_on(mobile.find("docs-mobile-menu-toggle")),
        Some(MediaQuery::max_width(959))
    );
    assert!(mobile.maybe_find("docs-sidebar-drawer").is_none());

    // Verify SSR emits exact zero-flicker `@media` hide rules & mobile article padding.
    let ssr_services: Arc<dyn HostServiceProvider> = Arc::new(MemoryHostServices::new());
    let mut ssr_surface = create_docs_surface_at_route(
        99,
        "/",
        build_config(),
        build_fixture_site(),
        build_playgrounds(),
        ssr_services,
    );
    let (_, _, frame_bytes) = ssr_surface.render_frame(1);
    let mut ssr_web = WebBackend::new();
    ssr_web.submit_frame(&frame_bytes).expect("ssr submit_frame");
    ssr_web.sync_responsive_from(
        ssr_surface.iter_show_on(),
        ssr_surface.iter_responsive_rules(),
    );
    let ssr = ssr_web.render_ssr();
    assert!(
        ssr.style_css.contains("@media (max-width:959.98px){.nve-m-"),
        "expected desktop-only @media hide rule in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("@media (max-width:1199.98px){.nve-m-"),
        "expected TOC-only @media hide rule in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("@media (min-width:960px){.nve-m-"),
        "expected mobile-only @media hide rule in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("padding:20px 16px;"),
        "expected mobile responsive article padding rule in SSR CSS:\n{}",
        ssr.style_css
    );

    mobile.press("docs-mobile-menu-toggle");
    assert!(mobile.maybe_find("docs-sidebar-drawer").is_some());
    assert_eq!(
        mobile.surface.node_show_on(mobile.find("docs-sidebar-overlay")),
        Some(MediaQuery::max_width(959))
    );

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
    let services: Arc<dyn HostServiceProvider> = Arc::new(MemoryHostServices::new());
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

#[test]
fn docs_prefix_and_custom_pages_coexist_cleanly() {
    use nve_dioxus::prelude::*;
    use nve_docs::{CustomPageRegistry, create_docs_surface_with_pages};

    let cfg = build_config()
        .with_base_path("/repo")
        .with_docs_prefix("/docs");
    assert_eq!(cfg.docs_route_for_content("/").as_str(), "/docs");
    assert_eq!(
        cfg.docs_route_for_content("/guides/quickstart").as_str(),
        "/docs/guides/quickstart"
    );
    assert_eq!(
        cfg.docs_route_for_content("/docs/guides/quickstart").as_str(),
        "/docs/guides/quickstart"
    );
    assert_eq!(cfg.content_route_for("/docs").as_deref(), Some("/"));
    assert_eq!(
        cfg.content_route_for("/docs/guides/quickstart").as_deref(),
        Some("/guides/quickstart")
    );
    assert_eq!(cfg.content_route_for("/"), None);
    assert_eq!(cfg.content_route_for("/examples"), None);
    assert_eq!(
        cfg.resolve_docs_href("/guides/quickstart"),
        "/repo/docs/guides/quickstart"
    );

    let mut custom = CustomPageRegistry::new();
    custom.register("/", || {
        rsx! {
            view {
                test_id: "custom-landing",
                text { "Custom Landing Hero" }
            }
        }
    });
    let custom = Arc::new(custom);
    let services: Arc<dyn HostServiceProvider> = Arc::new(MemoryHostServices::new());

    // Render custom landing at "/"
    let mut landing_surface = create_docs_surface_with_pages(
        10,
        "/",
        DocsViewport::desktop_default(),
        cfg.clone(),
        build_fixture_site(),
        build_playgrounds(),
        custom.clone(),
        services.clone(),
    );
    let (_, _, landing_bytes) = landing_surface.render_frame(1);
    let mut web = WebBackend::new();
    web.submit_frame(&landing_bytes).expect("landing submit");
    let landing_html = web.render_ssr().html;
    assert!(landing_html.contains("Custom Landing Hero"));
    assert!(!landing_html.contains("docs-sidebar-desktop"));

    // Render prefixed docs page at "/docs/guides/quickstart"
    let mut docs_surface = create_docs_surface_with_pages(
        11,
        "/docs/guides/quickstart",
        DocsViewport::desktop_default(),
        cfg,
        build_fixture_site(),
        build_playgrounds(),
        custom,
        services,
    );
    let (_, _, docs_bytes) = docs_surface.render_frame(1);
    let mut web_docs = WebBackend::new();
    web_docs.submit_frame(&docs_bytes).expect("docs submit");
    let docs_html = web_docs.render_ssr().html;
    assert!(docs_html.contains("Quickstart"));
    assert!(docs_html.contains("docs-sidebar-desktop"));
    assert!(docs_html.contains("href=\"/repo/docs/guides/components\""));
}
