//! Deep headless + SSR integration tests for `nve-docs`.

mod common;

use std::sync::Arc;

use common::{DocsHarness, RUST_SNIPPET, build_config, build_fixture_site, build_playgrounds};
use nve_abi::ColorRgba;
use nve_backend_headless::HeadlessBackend;
use nve_backend_web::WebBackend;
use nve_web_html_web::register_web_html_systems;
use nve_docs::{
    DocsAppState, DocsRoute, DocsViewport, callout_tone, docs_theme_config, syntax_color,
    system_appearance, MOBILE_BREAKPOINT_PT, TOC_BREAKPOINT_PT,
    create_docs_surface, create_docs_surface_at_route,
};
use nve_clipboard::ClipboardClient;
use nve_host_services::{HostServiceProvider, MemoryHostServices};
use nve_interop::ContrastLevel;
use nve_mdx::{CalloutKind, TokenKind};
use nve_ui::{AppBrand, Appearance, Polarity, Theme, ThemeMode, relative_luminance, wcag_contrast};
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
fn docs_theme_follows_os_with_docs_brand_and_syntax_and_callout_tones() {
    let dark = docs_theme_config(ThemeMode::System).resolve(Appearance::DARK);
    let light = docs_theme_config(ThemeMode::System).resolve(Appearance::LIGHT);
    assert_eq!(dark, Theme::preset(Polarity::Dark, AppBrand::Docs));
    assert_eq!(light, Theme::preset(Polarity::Light, AppBrand::Docs));
    assert_eq!(dark.surface.canvas, ColorRgba::rgb(0x09, 0x09, 0x0b));
    assert_eq!(light.surface.base, ColorRgba::WHITE);
    assert_ne!(dark.status.accent.solid, Theme::preset(Polarity::Dark, AppBrand::Studio).status.accent.solid);
    // An explicit user mode overrides the OS.
    assert!(!docs_theme_config(ThemeMode::Light).resolve(Appearance::DARK).is_dark());

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
    // Light elevation reads canvas (tinted) → base (white card).
    assert!(relative_luminance(light.surface.canvas) < relative_luminance(light.surface.base));
    for p in Polarity::ALL {
        let theme = Theme::preset(p, AppBrand::Docs);
        let min = if theme.is_high_contrast() { 7.0 } else { 4.5 };
        for kind in all_kinds {
            let c = syntax_color(&theme.syntax, kind);
            // Code well and highlighted lines (`accent.subtle`).
            for (bg_name, bg) in [("code well", theme.surface.sunken), ("highlighted line", theme.status.accent.subtle)] {
                let r = wcag_contrast(c, bg);
                assert!(r >= min, "{p:?} {kind:?} on {bg_name}: {r:.2} < {min}");
            }
        }
        for kind in [CalloutKind::Tip, CalloutKind::Info, CalloutKind::Warning, CalloutKind::Danger, CalloutKind::Note] {
            let tone = callout_tone(&theme, kind);
            let r = wcag_contrast(tone.colors.text, tone.colors.subtle);
            assert!(r >= min, "{p:?} {kind:?} callout title: {r:.2} < {min}");
        }
    }
    assert_eq!(callout_tone(&dark, CalloutKind::Danger).colors, dark.status.danger);

    let mem = Arc::new(MemoryHostServices::new());
    let sys_env = Arc::new(MemorySystemEnv::new());
    mem.register_arc(sys_env.clone());
    sys_env.set_theme(ThemeSnapshot {
        color_scheme: Some(ColorScheme::Light),
        ..ThemeSnapshot::default()
    });
    assert_eq!(system_appearance(mem.as_ref(), true), Appearance::LIGHT);

    sys_env.set_theme(ThemeSnapshot {
        color_scheme: Some(ColorScheme::Dark),
        contrast: ContrastLevel::High,
        ..ThemeSnapshot::default()
    });
    assert_eq!(
        system_appearance(mem.as_ref(), false),
        Appearance { scheme: ColorScheme::Dark, contrast: ContrastLevel::High }
    );

    sys_env.set_theme(ThemeSnapshot {
        color_scheme: None,
        ..ThemeSnapshot::default()
    });
    assert_eq!(system_appearance(mem.as_ref(), true), Appearance::DARK);

    let mut app_state = DocsAppState::new(
        build_config(),
        build_fixture_site(),
        build_playgrounds(),
        mem.clone(),
        DocsRoute::root(),
        DocsViewport::desktop_default(),
    );
    assert_eq!(app_state.theme_mode(), ThemeMode::System);
    assert!(app_state.theme().is_dark(), "None scheme falls back to config.default_dark");
    // The user mode pins a scheme, and writes through the cell shared with the host.
    let host_cell = app_state.config.theme_mode.clone();
    app_state.set_theme_mode(ThemeMode::Light);
    assert_eq!(app_state.theme_mode(), ThemeMode::Light);
    assert_eq!(host_cell.get(), ThemeMode::Light);
    assert!(!app_state.theme().is_dark());
}

#[test]
fn desktop_1280x800_three_column_and_mobile_390x844_drawer_collapse() {
    use nve_dioxus::prelude::MediaQuery;

    assert_eq!(MOBILE_BREAKPOINT_PT, 960.0);
    assert_eq!(TOC_BREAKPOINT_PT, 1200.0);

    // Desktop 1280x800: navbar + desktop sidebar + article + TOC + CSS-hidden mobile menu toggle
    let desktop = DocsHarness::mount("/", DocsViewport::new(1280.0, 800.0));
    let root = desktop.find("docs-root");
    assert_eq!(
        desktop.raw_str(root, "viewport-mode").as_deref(),
        Some("desktop")
    );
    assert_eq!(
        desktop
            .raw_str(desktop.find("docs-navbar"), "accessibilityRole")
            .as_deref(),
        Some("banner")
    );
    let desktop_sidebar = desktop.find("docs-sidebar-desktop");
    assert_eq!(
        desktop
            .raw_str(desktop_sidebar, "accessibilityRole")
            .as_deref(),
        Some("navigation")
    );
    assert_eq!(
        desktop.surface.node_show_on(desktop_sidebar),
        Some(MediaQuery::min_width(960))
    );
    assert_eq!(
        desktop
            .raw_str(desktop.find("docs-article"), "accessibilityRole")
            .as_deref(),
        Some("article")
    );
    let desktop_toc = desktop.find("docs-toc");
    assert_eq!(
        desktop.raw_str(desktop_toc, "accessibilityRole").as_deref(),
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
    assert_eq!(
        desktop.raw_str(toc_link, "accessibilityRole").as_deref(),
        Some("link")
    );
    assert_eq!(
        desktop.raw_str(toc_link, "href").as_deref(),
        Some("#architecture-overview")
    );

    let heading = desktop.find("docs-heading-architecture-overview");
    assert_eq!(
        desktop.raw_str(heading, "accessibilityRole").as_deref(),
        Some("heading")
    );
    assert_eq!(desktop.raw_str(heading, "aria-level").as_deref(), Some("2"));

    // Tablet 1024x768: identical DOM tree with declarative `@media` hiding TOC (< 1200pt)
    let tablet = DocsHarness::mount("/", DocsViewport::new(1024.0, 768.0));
    assert_eq!(
        tablet.raw_str(tablet.find("docs-root"), "viewport-mode").as_deref(),
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
        mobile.raw_str(mobile.find("docs-root"), "viewport-mode").as_deref(),
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
    let mut ssr_web = WebBackend::new();
    register_web_html_systems(ssr_web.table_registry_mut());
    ssr_surface.commit_to(1, &mut ssr_web).expect("ssr commit_to");
    let ssr = ssr_web.render_ssr(ssr_surface.store());
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
fn cmd_k_search_modal_filters_nvs2_index_and_navigates() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    assert!(h.maybe_find("docs-search-dialog").is_none());

    // Open via Cmd+K shortcut on root
    h.keydown("docs-root", "k", true, false, false, false);
    assert!(h.maybe_find("docs-search-dialog").is_some());

    // Close via Escape, then reopen via navbar trigger button
    h.keydown("docs-search-input", "Escape", false, false, false, false);
    assert!(h.maybe_find("docs-search-dialog").is_none());

    h.press("docs-search-trigger");
    // The dialog owns its elevated overlay surface (no shell override).
    let dialog = h.find("docs-search-dialog");
    assert_eq!(h.bg(dialog), Theme::preset(Polarity::Light, AppBrand::Docs).surface.overlay);

    // Type unique query present only in /guides/components
    h.type_text("docs-search-input", "quantum-flux-capacitor");
    let hit0_text = h.text_of("docs-search-hit-0");
    assert!(
        hit0_text.contains("Interactive Components"),
        "expected NVS2 search hit for Interactive Components, got: {hit0_text}"
    );

    // Press Enter on selected hit -> navigates directly to /guides/components & closes modal
    h.keydown("docs-search-input", "Enter", false, false, false, false);
    assert!(h.maybe_find("docs-search-dialog").is_none());
    assert_eq!(h.text_of("docs-page-title"), "Interactive Components");

    h.set_os_theme(ColorScheme::Dark, ContrastLevel::Standard);
    h.press("docs-search-trigger");
    let dialog = h.find("docs-search-dialog");
    let dark = Theme::preset(Polarity::Dark, AppBrand::Docs);
    assert_eq!(h.bg(dialog), dark.surface.overlay);
    assert_ne!(dark.surface.overlay, dark.surface.base, "overlay elevation is distinct in Dark");
}

#[test]
fn live_os_appearance_changes_rethemes_mounted_docs_until_user_pins_a_mode() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    let root = h.find("docs-root");
    assert_eq!(h.raw_str(root, "data-theme").as_deref(), Some("light"));

    // OS switches after mount: the live nve-system-env subscription re-resolves System mode.
    for (scheme, contrast, polarity, attr) in [
        (ColorScheme::Dark, ContrastLevel::Standard, Polarity::Dark, "dark"),
        (ColorScheme::Dark, ContrastLevel::High, Polarity::HighContrastDark, "dark"),
        (ColorScheme::Light, ContrastLevel::High, Polarity::HighContrastLight, "light"),
        (ColorScheme::Light, ContrastLevel::Standard, Polarity::Light, "light"),
    ] {
        h.set_os_theme(scheme, contrast);
        let root = h.find("docs-root");
        assert_eq!(h.raw_str(root, "data-theme").as_deref(), Some(attr), "{polarity:?}");
        assert_eq!(h.bg(root), Theme::preset(polarity, AppBrand::Docs).surface.canvas, "{polarity:?}");
    }

    // The user pins Dark; later OS changes no longer override it.
    h.press("docs-theme-mode-dark");
    h.set_os_theme(ColorScheme::Light, ContrastLevel::Standard);
    let root = h.find("docs-root");
    assert_eq!(h.raw_str(root, "data-theme").as_deref(), Some("dark"));
    assert_eq!(h.raw_str(root, "data-theme-mode").as_deref(), Some("dark"));
    assert_eq!(h.bg(root), Theme::preset(Polarity::Dark, AppBrand::Docs).surface.canvas);

    // Back to System: the OS appearance applies again immediately.
    h.press("docs-theme-mode-system");
    let root = h.find("docs-root");
    assert_eq!(h.raw_str(root, "data-theme").as_deref(), Some("light"));
    assert_eq!(h.raw_str(root, "data-theme-mode").as_deref(), Some("system"));
}

/// The System / Light / Dark control is a radio group: exactly one segment is checked, and it
/// follows the user's choice.
#[test]
fn theme_mode_control_checks_exactly_the_active_mode() {
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    for (press, expected) in [(None, "system"), (Some("light"), "light"), (Some("dark"), "dark"), (Some("system"), "system")] {
        if let Some(mode) = press {
            h.press(&format!("docs-theme-mode-{mode}"));
        }
        for mode in ["system", "light", "dark"] {
            let seg = h.find(&format!("docs-theme-mode-{mode}"));
            assert_eq!(h.aria_checked(seg), Some(mode == expected), "{mode} after {press:?}");
        }
    }
}

/// Rendered-contrast guard: every visible text run of the mounted fixture site (shell, MDX
/// content, code, callouts, search results) meets WCAG AA (AAA in high contrast) against its
/// actual painted background, in all four polarities.
#[test]
fn every_rendered_text_run_is_readable_in_all_four_polarities() {
    let mut failures = Vec::new();
    for route in ["/", "/guides/components"] {
        let mut h = DocsHarness::mount(route, DocsViewport::desktop_default());
        for (scheme, contrast, min) in [
            (ColorScheme::Light, ContrastLevel::Standard, 4.5),
            (ColorScheme::Dark, ContrastLevel::Standard, 4.5),
            (ColorScheme::Light, ContrastLevel::High, 7.0),
            (ColorScheme::Dark, ContrastLevel::High, 7.0),
        ] {
            h.set_os_theme(scheme, contrast);
            let non_text = if contrast == ContrastLevel::High { 4.5 } else { 3.0 };
            // The shell and page, then the open search modal with results, in this polarity.
            for surface in ["page", "search"] {
                if surface == "search" {
                    h.press("docs-search-trigger");
                    h.type_text("docs-search-input", "component");
                }
                for v in h.text_contrast_violations(min).into_iter().chain(h.indicator_contrast_violations(non_text)) {
                    failures.push(format!("{route} {surface} {scheme:?}/{contrast:?}: {v}"));
                }
            }
            h.keydown("docs-search-input", "Escape", false, false, false, false);
            assert!(h.maybe_find("docs-search-input").is_none(), "Escape closes search");
        }
    }
    assert!(failures.is_empty(), "{} low-contrast text runs:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn theme_follows_os_then_toggle_pins_canvas_and_surface_colors() {
    // The harness host reports a light OS appearance: System mode follows it on mount.
    let mut h = DocsHarness::mount("/", DocsViewport::desktop_default());
    let dark = Theme::preset(Polarity::Dark, AppBrand::Docs);
    let light = Theme::preset(Polarity::Light, AppBrand::Docs);
    let root_id = h.find("docs-root");
    let nav_id = h.find("docs-navbar");
    assert_eq!(h.raw_str(root_id, "data-theme").as_deref(), Some("light"));
    assert_eq!(h.bg(root_id), light.surface.canvas);
    assert_eq!(h.bg(nav_id), light.surface.base);

    // The user pins dark mode.
    h.press("docs-theme-mode-dark");
    // A pinned scheme survives a later OS change while the OS contrast preference still applies.
    h.set_os_theme(ColorScheme::Light, ContrastLevel::High);
    let root_id = h.find("docs-root");
    assert_eq!(h.raw_str(root_id, "data-theme").as_deref(), Some("dark"));
    assert_eq!(h.bg(root_id), Theme::preset(Polarity::HighContrastDark, AppBrand::Docs).surface.canvas);
    h.set_os_theme(ColorScheme::Light, ContrastLevel::Standard);
    let root_id = h.find("docs-root");
    let nav_id = h.find("docs-navbar");
    assert_eq!(h.raw_str(root_id, "data-theme").as_deref(), Some("dark"));
    assert_eq!(h.bg(root_id), dark.surface.canvas);
    assert_eq!(h.bg(nav_id), dark.surface.base);

    // And pins light mode.
    h.press("docs-theme-mode-light");
    let root_id = h.find("docs-root");
    assert_eq!(h.raw_str(root_id, "data-theme").as_deref(), Some("light"));
    assert_eq!(h.bg(root_id), light.surface.canvas);
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
        .find_matching(|s| s.starts_with("docs-tab-") && s.ends_with("-1"))
        .expect("second tab trigger");
    h.press_id(second_tab);

    let panel_after = h.find_prefix("docs-tab-panel-");
    assert!(
        h.collect_text(panel_after)
            .contains("ctest --test-dir cxx/build")
    );

    // 2. Verify LiveDemo island attributes & interactive Dioxus counter state
    let demo_frame = h.find("docs-demo-counter");
    assert_eq!(
        h.raw_str(demo_frame, "data-nve-island").as_deref(),
        Some("counter")
    );
    assert_eq!(
        h.raw_str(demo_frame, "data-nve-hydrate").as_deref(),
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
    let mut web = WebBackend::new();
    register_web_html_systems(web.table_registry_mut());
    surface.commit_to(1, &mut web).expect("web commit_to");
    let html = web.render_html(surface.store());
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
    let mut headless = HeadlessBackend::new();
    root_surface
        .commit_to(1, &mut headless)
        .expect("root commit_to");
    assert!(
        headless
            .dump_tree(root_surface.store())
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
    let mut web = WebBackend::new();
    register_web_html_systems(web.table_registry_mut());
    landing_surface
        .commit_to(1, &mut web)
        .expect("landing commit");
    let landing_html = web.render_ssr(landing_surface.store()).html;
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
    let mut web_docs = WebBackend::new();
    register_web_html_systems(web_docs.table_registry_mut());
    docs_surface.commit_to(1, &mut web_docs).expect("docs commit");
    let docs_html = web_docs.render_ssr(docs_surface.store()).html;
    assert!(docs_html.contains("Quickstart"));
    assert!(docs_html.contains("docs-sidebar-desktop"));
    assert!(docs_html.contains("href=\"/repo/docs/guides/components\""));
}
