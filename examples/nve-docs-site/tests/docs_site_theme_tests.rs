//! Theme contract of the official docs site: OS-following appearance on every host path,
//! rendered text contrast on real pages in all four polarities, and source hygiene.

use std::path::Path;
use std::sync::Arc;

use nve_backend_headless::HeadlessBackend;
use nve_dioxus::DioxusSurface;
use nve_docs::DocsViewport;
use nve_docs_site::{
    create_nve_docs_surface_full, create_nve_docs_surface_with_host, docs_host_services,
    docs_root_data_attr, embedded_compiled_site, nve_docs_site_config, surface_theme_mode,
};
use nve_system_env::{ColorScheme, ContrastLevel, MemorySystemEnv, ThemeSnapshot};
use nve_ui::audit::text_contrast_violations;
use nve_ui::ThemeMode;

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
        self.surface.commit_to(self.frame, &mut self.backend).expect("headless commit_to");
    }

    fn theme_attr(&self) -> Option<String> {
        docs_root_data_attr(&self.surface, "theme")
    }
}

fn live_site(route: &str, os: &Arc<MemorySystemEnv>, mode: ThemeMode) -> (Site, Arc<nve_host_services::MemoryHostServices>) {
    let host = docs_host_services(os.clone());
    let surface = create_nve_docs_surface_with_host(
        1,
        embedded_compiled_site(),
        route,
        DocsViewport::desktop_default(),
        nve_docs_site_config("").with_theme_mode(mode),
        Arc::clone(&host),
    );
    (Site::new(surface), host)
}

fn os_theme(scheme: ColorScheme, contrast: ContrastLevel) -> ThemeSnapshot {
    ThemeSnapshot { color_scheme: Some(scheme), contrast, ..ThemeSnapshot::default() }
}

#[test]
fn headless_hosts_honor_default_dark_when_the_os_reports_no_scheme() {
    for (default_dark, attr) in [(true, "dark"), (false, "light")] {
        let site = Site::new(create_nve_docs_surface_full(
            1,
            embedded_compiled_site(),
            "/",
            "",
            DocsViewport::desktop_default(),
            default_dark,
        ));
        assert_eq!(site.theme_attr().as_deref(), Some(attr), "default_dark={default_dark}");
        assert_eq!(surface_theme_mode(&site.surface), ThemeMode::System);
    }
}

#[test]
fn live_os_changes_retheme_and_rebuilds_keep_the_user_mode() {
    let os = Arc::new(MemorySystemEnv::with_initial(
        os_theme(ColorScheme::Light, ContrastLevel::Standard),
        Default::default(),
    ));
    let (mut site, host) = live_site("/", &os, ThemeMode::System);
    assert_eq!(site.theme_attr().as_deref(), Some("light"));

    os.set_theme_and_publish(host.typed_events(), os_theme(ColorScheme::Dark, ContrastLevel::Standard));
    site.render();
    assert_eq!(site.theme_attr().as_deref(), Some("dark"), "System mode follows the OS live");

    // A host rebuild (e.g. browser popstate) passes the previous user mode back in.
    let (pinned, _) = live_site("/docs/guides/routing", &os, ThemeMode::Light);
    assert_eq!(surface_theme_mode(&pinned.surface), ThemeMode::Light);
    assert_eq!(pinned.theme_attr().as_deref(), Some("light"), "pinned mode beats the dark OS");
}

/// Every visible text run on representative real pages (landing, gallery, guides with live
/// playgrounds, benchmarks) meets WCAG AA (AAA in high contrast) in all four polarities.
#[test]
fn every_rendered_text_run_on_site_pages_is_readable_in_all_four_polarities() {
    let os = Arc::new(MemorySystemEnv::new());
    let mut failures = Vec::new();
    for route in [
        "/",
        "/examples",
        "/docs/guides/ui-components",
        "/docs/guides/canvas-draw1",
        "/docs/guides/local-first-crdt",
        "/docs/reference/benchmarks",
    ] {
        let (mut site, host) = live_site(route, &os, ThemeMode::System);
        for (scheme, contrast, min) in [
            (ColorScheme::Light, ContrastLevel::Standard, 4.5),
            (ColorScheme::Dark, ContrastLevel::Standard, 4.5),
            (ColorScheme::Light, ContrastLevel::High, 7.0),
            (ColorScheme::Dark, ContrastLevel::High, 7.0),
        ] {
            os.set_theme_and_publish(host.typed_events(), os_theme(scheme, contrast));
            site.render();
            for v in text_contrast_violations(site.surface.store(), min) {
                failures.push(format!("{route} {scheme:?}/{contrast:?}: {:?} {:.2}", v.text, v.ratio));
            }
        }
    }
    assert!(failures.is_empty(), "{} low-contrast text runs:\n{}", failures.len(), failures.join("\n"));
}

/// Docs sources take every color from `nve_ui::Theme`: no CSS hex / `rgb(a)` literals, no raw
/// `ColorRgba` constructors, no per-render `.hex().to_string()`, no pre-unification theme types.
#[test]
fn docs_sources_use_theme_tokens_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for dir in [root.join("src"), root.join("../../packages/framework/docs/src")] {
        scan(&dir, &mut violations);
    }
    for doc in ["../../packages/framework/docs/README.md", "../../docs/content/guides/ui-components.mdx"] {
        let text = std::fs::read_to_string(root.join(doc)).unwrap_or_default();
        for stale in ["DocsTheme", "UiTheme", "UiPalette"] {
            if text.contains(stale) {
                violations.push(format!("{doc}: stale `{stale}`"));
            }
        }
    }
    assert!(violations.is_empty(), "theme hygiene violations:\n{}", violations.join("\n"));
}

fn scan(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let hit = has_css_hex(line)
                || line.contains("rgb(")
                || line.contains("rgba(")
                || line.contains(".hex().to_string()")
                || ["DocsTheme", "UiTheme", "UiPalette"].iter().any(|s| line.contains(s));
            if hit {
                out.push(format!("{}:{}: {}", path.display(), n + 1, code));
            }
        }
    }
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` followed by a non-identifier character.
fn has_css_hex(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        if b != b'#' {
            return false;
        }
        let digits = bytes[i + 1..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
        let end_ok = bytes.get(i + 1 + digits).is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_');
        matches!(digits, 3 | 6 | 8) && end_ok
    })
}
