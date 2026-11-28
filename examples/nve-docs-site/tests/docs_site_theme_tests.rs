//! Theme contract of the official docs site on every host path: OS-following appearance (headless,
//! browser, native module), the user's mode shared with the host across rebuilds, appearance-correct
//! static SSR, and rendered text contrast on real pages in all four polarities.

use nve_abi::{EventBatchBuf, FrameId, NodeId, SurfaceId, TimestampUs};
use nve_backend_headless::HeadlessBackend;
use nve_tables::tables::DebugId;
use nve_dioxus::DioxusSurface;
use nve_docs::DocsViewport;
use nve_docs_site::ssr::{APPEARANCE_STYLE_ATTR, THEME_MODE_BOOT_SCRIPT, appearance_overrides, appearance_token};
use nve_docs_site::{
    DocsHost, DocsSiteModule, THEME_MODE_STORAGE_KEY, create_nve_docs_surface_full, embedded_compiled_site,
    render_docs_ssr,
};
use nve_engine::NveApp;
use nve_system_env::{ColorScheme, ContrastLevel, ThemeSnapshot};
use nve_ui::audit::text_contrast_violations;
use nve_ui::{AppBrand, Appearance, ColorHex, Polarity, Theme, ThemeMode};
use nve_web_html::{DataAttrList, HtmlDataAttrs};

/// A `data-*` attribute of the rendered `docs-root` (what the user sees; hosts read the host).
fn rendered_root_attr(surface: &DioxusSurface, key: &str) -> Option<String> {
    let store = surface.store();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if store.try_row::<DebugId>(id).is_some_and(|d| d.test_id() == "docs-root") {
            let list = store.resource::<DataAttrList>(store.try_row::<HtmlDataAttrs>(id)?.entries())?;
            return list.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_owned());
        }
        stack.extend_from_slice(store.children(id));
    }
    None
}

fn find(surface: &DioxusSurface, test_id: &str) -> NodeId {
    let store = surface.store();
    store
        .live_nodes()
        .find(|&id| store.try_row::<DebugId>(id).is_some_and(|d| d.test_id() == test_id))
        .expect(test_id)
}

struct Site {
    surface: DioxusSurface,
    backend: HeadlessBackend,
    frame: u64,
}

impl Site {
    fn new(surface: DioxusSurface) -> Self {
        let mut site = Self { surface, backend: HeadlessBackend::new(), frame: 0 };
        site.render();
        site
    }

    fn render(&mut self) {
        self.frame += 1;
        self.surface.commit_to(FrameId(self.frame), &mut self.backend).expect("headless commit_to");
    }

    fn theme_attr(&self) -> Option<String> {
        rendered_root_attr(&self.surface, "theme")
    }

    fn press(&mut self, test_id: &str) {
        let mut batch = EventBatchBuf::new();
        batch.push_press(
            SurfaceId::ROOT,
            find(&self.surface, test_id),
            TimestampUs(self.frame * 16_000),
            8.0,
            8.0,
        );
        assert!(self.surface.dispatch_events(batch.as_validated()), "{test_id} press handled");
        self.render();
    }
}

fn os_theme(scheme: ColorScheme, contrast: ContrastLevel) -> ThemeSnapshot {
    ThemeSnapshot { color_scheme: Some(scheme), contrast, ..ThemeSnapshot::default() }
}

fn mount(host: &DocsHost, route: &str) -> Site {
    Site::new(host.mount(SurfaceId::ROOT, embedded_compiled_site(), route, "", DocsViewport::desktop_default()))
}

#[test]
fn headless_hosts_honor_default_dark_when_the_os_reports_no_scheme() {
    for (default_dark, attr) in [(true, "dark"), (false, "light")] {
        let site = Site::new(create_nve_docs_surface_full(
            SurfaceId::ROOT,
            embedded_compiled_site(),
            "/",
            "",
            DocsViewport::desktop_default(),
            default_dark,
        ));
        assert_eq!(site.theme_attr().as_deref(), Some(attr), "default_dark={default_dark}");
        assert_eq!(rendered_root_attr(&site.surface, "theme-mode").as_deref(), Some("system"));
    }
}

#[test]
fn live_os_changes_retheme_and_rebuilds_keep_the_user_mode() {
    let host = DocsHost::new(os_theme(ColorScheme::Light, ContrastLevel::Standard), ThemeMode::System, true);
    let mut site = mount(&host, "/");
    assert_eq!(site.theme_attr().as_deref(), Some("light"));

    host.publish_theme(os_theme(ColorScheme::Dark, ContrastLevel::Standard));
    site.render();
    assert_eq!(site.theme_attr().as_deref(), Some("dark"), "System mode follows the OS live");
    assert!(host.theme().is_dark());

    // The user pins Light on the mounted surface: the host reads it from the shared cell...
    site.press("docs-theme-mode-light");
    assert_eq!(host.theme_mode(), ThemeMode::Light);
    assert!(!host.theme().is_dark(), "host chrome resolves the pinned mode");
    // ...and a host rebuild (browser popstate, native route prop) keeps it.
    let rebuilt = mount(&host, "/docs/guides/routing");
    assert_eq!(rendered_root_attr(&rebuilt.surface, "theme-mode").as_deref(), Some("light"));
    assert_eq!(rebuilt.theme_attr().as_deref(), Some("light"), "pinned mode beats the dark OS");
}

/// The native module re-themes on host `ThemeChanged` payloads (Skia shell, `NVE_IFACE_HOST`)
/// through `nve-system-env`, in place: no rebuild, same route, across all four appearances.
#[test]
fn native_module_follows_host_theme_events_without_rebuilding() {
    let host = DocsHost::new(os_theme(ColorScheme::Light, ContrastLevel::Standard), ThemeMode::System, true);
    let mut module = DocsSiteModule::with_host(SurfaceId::ROOT, host);
    let _ = module.render(FrameId(1));
    let root = find(module.surface(), "docs-root");
    for (frame, (scheme, contrast, polarity, attr)) in [
        (ColorScheme::Dark, ContrastLevel::Standard, Polarity::Dark, "dark"),
        (ColorScheme::Dark, ContrastLevel::High, Polarity::HighContrastDark, "dark"),
        (ColorScheme::Light, ContrastLevel::High, Polarity::HighContrastLight, "light"),
        (ColorScheme::Light, ContrastLevel::Standard, Polarity::Light, "light"),
    ]
    .into_iter()
    .enumerate()
    {
        let payload = os_theme(scheme, contrast).to_repr_c_bytes();
        assert!(module.dispatch_host_events(&payload), "{polarity:?} payload accepted");
        let _ = module.render(FrameId(frame as u64 + 2));
        assert_eq!(rendered_root_attr(module.surface(), "theme").as_deref(), Some(attr), "{polarity:?}");
        assert_eq!(find(module.surface(), "docs-root"), root, "{polarity:?}: re-themed in place, not rebuilt");
        assert_eq!(module.host().theme(), Theme::preset(polarity, AppBrand::Docs), "{polarity:?}");
    }
    assert!(!module.dispatch_host_events(b"not a theme"), "foreign payloads are ignored");
}

/// Static pages paint the visitor's appearance before hydration: every non-base appearance
/// yields node overrides on real pages, the trees stay identical (zero hydration mismatches), and
/// the `<head>` carries the mode boot script, the chrome for all four appearances, and the
/// removable node overrides.
#[test]
fn static_ssr_ships_overrides_for_every_appearance() {
    let site = embedded_compiled_site();
    for route in ["/", "/examples", "/docs/guides/ui-components", "/docs/reference/benchmarks"] {
        let ssr = render_docs_ssr(&site, route, "", DocsViewport::desktop_default()).expect("SSR");
        assert!(ssr.head.contains(THEME_MODE_BOOT_SCRIPT), "{route}");
        assert!(ssr.head.contains(&format!("<style {APPEARANCE_STYLE_ATTR}>")), "{route}");
        for polarity in Polarity::ALL {
            let canvas = Theme::preset(polarity, AppBrand::Docs).surface.canvas.hex().to_string();
            assert!(ssr.head.contains(&canvas), "{route}: chrome for {polarity:?} ({canvas})");
        }
        for appearance in [
            Appearance { scheme: ColorScheme::Light, contrast: ContrastLevel::Standard },
            Appearance { scheme: ColorScheme::Dark, contrast: ContrastLevel::High },
            Appearance { scheme: ColorScheme::Light, contrast: ContrastLevel::High },
        ] {
            let rules = appearance_overrides(route, appearance).expect("identical trees");
            assert!(rules.len() > 10, "{route} {appearance:?}: {} overrides", rules.len());
            let scope = format!("html[data-nve-appearance=\"{}\"]", appearance_token(appearance));
            assert!(ssr.head.contains(&scope), "{route}: overrides scoped on {scope}");
        }
        // Overrides carry only changed properties: a fraction of the page's own SSR CSS.
        let output = ssr.output();
        assert!(ssr.head.len() < 2 * output.style_css.len(), "{route}: head {} B vs SSR CSS {} B", ssr.head.len(), output.style_css.len());
    }
    // The boot script emits exactly the tokens the overrides are scoped on.
    assert!(THEME_MODE_BOOT_SCRIPT.contains(THEME_MODE_STORAGE_KEY));
    for token in ["\"dark\":\"light\"", "\"-hc\""] {
        assert!(THEME_MODE_BOOT_SCRIPT.contains(token), "{token}");
    }
}

/// Every visible text run on representative real pages (landing, gallery, guides with live
/// playgrounds, benchmarks) meets WCAG AA (AAA in high contrast) in all four polarities.
#[test]
fn every_rendered_text_run_on_site_pages_is_readable_in_all_four_polarities() {
    let mut failures = Vec::new();
    for route in [
        "/",
        "/examples",
        "/docs/guides/ui-components",
        "/docs/guides/canvas-draw1",
        "/docs/guides/local-first-crdt",
        "/docs/reference/benchmarks",
    ] {
        let host = DocsHost::headless(true);
        let mut site = mount(&host, route);
        for (scheme, contrast, min) in [
            (ColorScheme::Light, ContrastLevel::Standard, 4.5),
            (ColorScheme::Dark, ContrastLevel::Standard, 4.5),
            (ColorScheme::Light, ContrastLevel::High, 7.0),
            (ColorScheme::Dark, ContrastLevel::High, 7.0),
        ] {
            host.publish_theme(os_theme(scheme, contrast));
            site.render();
            for v in text_contrast_violations(site.surface.store(), min) {
                failures.push(format!("{route} {scheme:?}/{contrast:?}: {:?} {:.2}", v.text, v.ratio));
            }
        }
    }
    assert!(failures.is_empty(), "{} low-contrast text runs:\n{}", failures.len(), failures.join("\n"));
}
