//! Static SSR of the official docs whose first paint already matches the visitor's appearance.
//!
//! Pages render once in the base appearance (dark, standard contrast). The same route is rendered
//! again for every other appearance and diffed into unlayered override rules carrying only the
//! changed properties ([`nve_backend_web::ssr_variants`]), scoped on
//! `<html data-nve-appearance>`. [`THEME_MODE_BOOT_SCRIPT`] sets that attribute before first paint
//! from the persisted user mode and `prefers-color-scheme` / `prefers-contrast`, so a light-OS
//! visitor never sees a dark flash, and hydration adopts every node unchanged (only colors differ
//! between the variants). The client removes the node overrides (`<style
//! data-nve-docs-appearance>`) after its first frame. The document chrome (canvas, ink) also
//! follows the media queries directly, so it is right without JavaScript too, and keeps following
//! `<html data-theme-mode>` after hydration.

use std::sync::Arc;

use nve_backend_web::ssr_variants::{SsrStyleOverride, ssr_style_overrides, write_scoped_overrides};
use nve_backend_web::{SsrOutput, WebBackend};
use nve_dioxus::DioxusSurface;
use nve_docs::{DocsViewport, docs_theme_config};
use nve_mdx::CompiledSite;
use nve_system_env::{ColorScheme, ContrastLevel, ThemeSnapshot};
use nve_ui::{Appearance, ColorHex, ThemeMode};

use crate::host::DocsHost;
use crate::site::embedded_compiled_site;

/// Before first paint: `<html data-theme-mode>` from the persisted user mode (key
/// [`crate::THEME_MODE_STORAGE_KEY`]) and `<html data-nve-appearance>` (see [`appearance_token`])
/// from that mode and the OS appearance.
pub const THEME_MODE_BOOT_SCRIPT: &str = "<script>try{var d=document.documentElement,q=function(x){return matchMedia(x).matches},\
m=localStorage.getItem(\"nve-docs-theme-mode\");if(m===\"light\"||m===\"dark\")d.setAttribute(\"data-theme-mode\",m);else m=null;\
d.setAttribute(\"data-nve-appearance\",(m||(q(\"(prefers-color-scheme: dark)\")?\"dark\":\"light\"))+(q(\"(prefers-contrast: more)\")?\"-hc\":\"\"))}catch(e){}</script>";

/// `<html data-nve-appearance>` value of an appearance (`light`, `dark`, `light-hc`, `dark-hc`).
pub const fn appearance_token(appearance: Appearance) -> &'static str {
    match (appearance.scheme, appearance.contrast) {
        (ColorScheme::Light, ContrastLevel::Standard) => "light",
        (ColorScheme::Dark, ContrastLevel::Standard) => "dark",
        (ColorScheme::Light, ContrastLevel::High) => "light-hc",
        (ColorScheme::Dark, ContrastLevel::High) => "dark-hc",
    }
}

/// Marker attribute of the node-override `<style>` the client removes after hydrating.
pub const APPEARANCE_STYLE_ATTR: &str = "data-nve-docs-appearance";

/// The appearance static pages render in; every other one is emitted as overrides.
const BASE: Appearance = Appearance { scheme: ColorScheme::Dark, contrast: ContrastLevel::Standard };

const VARIANTS: [Appearance; 3] = [
    Appearance { scheme: ColorScheme::Light, contrast: ContrastLevel::Standard },
    Appearance { scheme: ColorScheme::Dark, contrast: ContrastLevel::High },
    Appearance { scheme: ColorScheme::Light, contrast: ContrastLevel::High },
];

/// A committed docs page ready for `WebBackend::render_ssr*`, plus the `<head>` markup that makes
/// it appearance-correct before hydration.
pub struct DocsSsr {
    pub surface: DioxusSurface,
    pub backend: WebBackend,
    pub head: String,
}

impl DocsSsr {
    pub fn output(&self) -> SsrOutput {
        self.backend.render_ssr(self.surface.store())
    }
}

fn web_backend() -> WebBackend {
    let mut backend = WebBackend::new();
    crate::register_web_systems(backend.table_registry_mut());
    backend
}

fn render(host: &DocsHost, site: &Arc<CompiledSite>, route: &str, base_path: &str, viewport: DocsViewport) -> Result<(DioxusSurface, WebBackend), String> {
    let mut surface = host.mount(1, Arc::clone(site), route, base_path, viewport);
    let mut backend = web_backend();
    surface.commit_to(1, &mut backend).map_err(|e| format!("docs SSR commit for '{route}': {e}"))?;
    Ok((surface, backend))
}

fn os_host(appearance: Appearance) -> DocsHost {
    let snapshot = ThemeSnapshot { color_scheme: Some(appearance.scheme), contrast: appearance.contrast, ..ThemeSnapshot::default() };
    DocsHost::new(snapshot, ThemeMode::System, true)
}

/// Renders `route` of `site` for static output: base page plus appearance `<head>` markup.
pub fn render_docs_ssr(site: &Arc<CompiledSite>, route: &str, base_path: &str, viewport: DocsViewport) -> Result<DocsSsr, String> {
    let (surface, backend) = render(&os_host(BASE), site, route, base_path, viewport)?;
    let mut nodes = String::new();
    let mut chrome = String::new();
    write_chrome(&mut chrome, "html", BASE);
    for appearance in VARIANTS {
        let (variant, _) = render(&os_host(appearance), site, route, base_path, viewport)?;
        let rules = ssr_style_overrides(surface.store(), variant.store()).map_err(|e| format!("docs SSR '{route}': {e}"))?;
        let scope = format!("html[data-nve-appearance=\"{}\"]", appearance_token(appearance));
        write_scoped_overrides(&rules, &scope, &mut nodes);
        write_variant(&mut chrome, appearance, |scope, out| write_chrome(out, scope, appearance));
    }
    let head = format!(
        "{metas}{THEME_MODE_BOOT_SCRIPT}<style data-nve-docs-chrome-theme>{chrome}</style><style {APPEARANCE_STYLE_ATTR}>{nodes}</style>",
        metas = theme_color_metas(),
    );
    Ok(DocsSsr { surface, backend, head })
}

/// Renders the base appearance of an embedded route (tests and tooling).
pub fn render_nve_docs_ssr(route: &str, base_path: &str, viewport: DocsViewport) -> Result<SsrOutput, String> {
    let (surface, backend) = render(&os_host(BASE), &embedded_compiled_site(), route, base_path, viewport)?;
    Ok(backend.render_ssr(surface.store()))
}

/// Wraps `body` (called with the `html` selector scope) in the conditions under which
/// `appearance` applies, without JavaScript: the OS contrast preference, and either a pinned
/// user mode of that scheme or the OS scheme when the user has not pinned the other one.
fn write_variant(out: &mut String, appearance: Appearance, body: impl Fn(&str, &mut String)) {
    let (scheme, other) = match appearance.scheme {
        ColorScheme::Light => ("light", "dark"),
        ColorScheme::Dark => ("dark", "light"),
    };
    let contrast = match appearance.contrast {
        ContrastLevel::High => "(prefers-contrast: more)",
        ContrastLevel::Standard => "not all and (prefers-contrast: more)",
    };
    out.push_str("@media ");
    out.push_str(contrast);
    out.push('{');
    body(&format!("html[data-theme-mode=\"{scheme}\"]"), out);
    out.push_str("@media (prefers-color-scheme: ");
    out.push_str(scheme);
    out.push_str("){");
    body(&format!("html:not([data-theme-mode=\"{other}\"])"), out);
    out.push_str("}}");
}

/// Document canvas, ink, and native control scheme for `appearance` on `scope` and its body.
fn write_chrome(out: &mut String, scope: &str, appearance: Appearance) {
    let theme = docs_theme_config(ThemeMode::System).resolve(appearance);
    let scheme = if theme.is_dark() { "dark" } else { "light" };
    out.push_str(&format!(
        "{scope},{scope} body{{background:{};color:{};color-scheme:{scheme};}}",
        theme.surface.canvas.hex(),
        theme.text.primary.hex(),
    ));
}

fn theme_color_metas() -> String {
    [ColorScheme::Light, ColorScheme::Dark]
        .into_iter()
        .map(|scheme| {
            let canvas = docs_theme_config(ThemeMode::System).resolve(Appearance { scheme, contrast: ContrastLevel::Standard }).surface.canvas;
            let media = if scheme == ColorScheme::Light { "light" } else { "dark" };
            format!("<meta name=\"theme-color\" media=\"(prefers-color-scheme: {media})\" content=\"{}\">", canvas.hex())
        })
        .collect()
}

/// Override rules restyling the base render of `route` into `appearance` (tests and tooling).
pub fn appearance_overrides(route: &str, appearance: Appearance) -> Result<Vec<SsrStyleOverride>, String> {
    let site = embedded_compiled_site();
    let viewport = DocsViewport::desktop_default();
    let (base, _) = render(&os_host(BASE), &site, route, "", viewport)?;
    let (variant, _) = render(&os_host(appearance), &site, route, "", viewport)?;
    ssr_style_overrides(base.store(), variant.store()).map_err(|e| e.to_string())
}
