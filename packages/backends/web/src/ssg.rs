//! Generic, app-agnostic Web SSG + 0ms Instant-Restore document builder & shell injector.
//!
//! Works with **any** `nve_engine::NveApp` and **any** `WebBackend` mount container (`#app`,
//! `#nve-root`, etc.):
//! - Build-time [`prerender_nve_app_frame0`] renders Frame 0 of any `NveApp` into a headless
//!   [`WebBackend`].
//! - [`WebBackend::render_ssg_app_document`] emits a standalone HTML5 document containing:
//!   1. Build-time Frame-0 `<style data-nve-base>`, `<style data-nve-ssr data-nve-hashes="...">`,
//!      and semantic HTML inside `#<mount_id>` so brand-new cold visitors paint at **0ms** while
//!      WASM downloads.
//!   2. Synchronous `<script data-nve-snapshot-boot>` (`< 900` B) right after `#<mount_id>` that
//!      clears `localStorage` on `?reset=1` (keeping clean build-time SSG HTML) or swaps in a
//!      returning user's last cached CSS+HTML from `localStorage` in `< 1ms` before first paint.
//!   3. `<script data-nve-replay>` arming `window.__nveReplayDrain()` for BOTH cold visitors (SSG)
//!      and returning users (cached snapshot).
//! - [`inject_ssg_into_shell_html`] / [`inject_ssg_with_scripts_into_shell_html`] inject the exact
//!   same build-time SSG + bootstrap scripts into any existing host `index.html` shell template.

use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind, ROOT_NODE_ID};
use nve_engine::{BackendError, NveApp, RenderBackend, Viewport};

use crate::html::{push_attr_escaped, push_text_escaped};
use crate::snapshot::snapshot_and_replay_script_tags;
use crate::ssr::SsrOutput;
use crate::WebBackend;

/// Options for [`WebBackend::render_ssg_app_document`] generic SSG + 0ms instant-restore emission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebSsgDocumentOptions {
    pub title: String,
    pub app_slug: String,
    pub schema_version: u32,
    /// DOM element ID for the mount container (defaults to `"app"` when empty).
    pub mount_id: String,
    pub description: Option<String>,
    pub canonical_url: Option<String>,
    pub og_image: Option<String>,
    pub lang: Option<String>,
    pub head_extra: Option<String>,
    pub body_scripts: Option<String>,
}

/// Runs Frame 0 of **any** [`NveApp`] at `viewport` into a headless [`WebBackend`] ready for
/// [`WebBackend::render_ssr`] or [`WebBackend::render_ssg_app_document`].
pub fn prerender_nve_app_frame0<A: NveApp>(viewport: Viewport) -> Result<WebBackend, BackendError> {
    let mut app = A::create(1);
    if viewport.width.is_finite()
        && viewport.height.is_finite()
        && viewport.width > 0.0
        && viewport.height > 0.0
    {
        let layout = encode_events(&[NativeEvent {
            surface_id: 1,
            node_id: ROOT_NODE_ID,
            timestamp_us: 0,
            kind: NativeEventKind::Layout,
            payload: EventPayload::Layout {
                x: 0.0,
                y: 0.0,
                width: viewport.width,
                height: viewport.height,
            },
        }]);
        let _ = app.dispatch_events(&layout);
    }
    let frame_bytes = app.render(1).to_vec();
    let mut backend = WebBackend::new();
    backend.set_viewport(1, viewport);
    backend.submit_frame(&frame_bytes)?;
    Ok(backend)
}

impl WebBackend {
    /// Emits a complete standalone HTML5 SSG document for **any** NVE web app with:
    /// - Build-time Frame-0 `<style data-nve-base>` and `<style data-nve-ssr data-nve-hashes="...">`
    /// - Build-time Frame-0 semantic HTML inside `<div id="{mount_id}">` (0ms paint for cold visitors)
    /// - Inline `<script data-nve-snapshot-boot>` (`< 900` B) for returning-user 0ms state override
    ///   and `?reset=1` cleanup
    /// - Inline `<script data-nve-replay>` arming `window.__nveReplayDrain()` before `body_scripts`
    pub fn render_ssg_app_document(&self, opts: &WebSsgDocumentOptions) -> String {
        let ssr = self.render_ssr();
        let lang = opts
            .lang
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("en");
        let mount_id = if opts.mount_id.trim().is_empty() {
            "app"
        } else {
            opts.mount_id.trim()
        };
        let app_slug = if opts.app_slug.trim().is_empty() {
            "app"
        } else {
            opts.app_slug.trim()
        };
        let schema_version = opts.schema_version.max(1);
        let bootstrap = snapshot_and_replay_script_tags(mount_id, app_slug, schema_version);

        let mut out = String::with_capacity(
            ssr.base_css.len() + ssr.style_css.len() + ssr.html.len() + bootstrap.len() + 512,
        );
        out.push_str("<!doctype html>\n<html lang=\"");
        push_attr_escaped(lang, &mut out);
        out.push_str("\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>");
        push_text_escaped(&opts.title, &mut out);
        out.push_str("</title>\n");

        if let Some(desc) = opts.description.as_deref().filter(|s| !s.is_empty()) {
            out.push_str("<meta name=\"description\" content=\"");
            push_attr_escaped(desc, &mut out);
            out.push_str("\">\n<meta property=\"og:description\" content=\"");
            push_attr_escaped(desc, &mut out);
            out.push_str("\">\n");
        }
        if !opts.title.is_empty() {
            out.push_str("<meta property=\"og:title\" content=\"");
            push_attr_escaped(&opts.title, &mut out);
            out.push_str("\">\n");
        }
        if let Some(canonical) = opts.canonical_url.as_deref().filter(|s| !s.is_empty()) {
            out.push_str("<link rel=\"canonical\" href=\"");
            push_attr_escaped(canonical, &mut out);
            out.push_str("\">\n<meta property=\"og:url\" content=\"");
            push_attr_escaped(canonical, &mut out);
            out.push_str("\">\n");
        }
        if let Some(og_img) = opts.og_image.as_deref().filter(|s| !s.is_empty()) {
            out.push_str("<meta property=\"og:image\" content=\"");
            push_attr_escaped(og_img, &mut out);
            out.push_str("\">\n");
        }

        out.push_str("<style data-nve-base>");
        out.push_str(ssr.base_css);
        out.push_str("</style>\n<style data-nve-ssr data-nve-hashes=\"");
        for (idx, h) in ssr.rule_hashes.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(h);
        }
        out.push_str("\">");
        out.push_str(&ssr.style_css);
        out.push_str("</style>\n");

        if let Some(extra) = opts.head_extra.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(extra);
            if !extra.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("</head>\n<body>\n<div id=\"");
        push_attr_escaped(mount_id, &mut out);
        out.push_str("\">");
        out.push_str(&ssr.html);
        out.push_str("</div>\n");
        out.push_str(&bootstrap);
        out.push('\n');
        if let Some(scripts) = opts.body_scripts.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(scripts);
            if !scripts.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("</body>\n</html>\n");
        out
    }
}

/// Injects build-time Frame-0 SSG CSS + HTML plus single-app `<script data-nve-snapshot-boot>`
/// and `<script data-nve-replay>` into any host shell HTML template.
pub fn inject_ssg_into_shell_html(
    shell_html: &str,
    mount_id: &str,
    app_slug: &str,
    schema_version: u32,
    ssr: &SsrOutput,
) -> String {
    let scripts = snapshot_and_replay_script_tags(mount_id, app_slug, schema_version);
    inject_ssg_with_scripts_into_shell_html(shell_html, mount_id, ssr, &scripts)
}

/// Injects build-time Frame-0 SSG `<style data-nve-base>`, `<style data-nve-ssr data-nve-hashes>`,
/// `#<mount_id>` inner HTML, and caller-provided bootstrap `<script>` tags (unless already present)
/// into a host shell HTML template.
pub fn inject_ssg_with_scripts_into_shell_html(
    shell_html: &str,
    mount_id: &str,
    ssr: &SsrOutput,
    bootstrap_scripts: &str,
) -> String {
    let hashes_csv = ssr.rule_hashes.join(",");
    let mut style_tags = String::new();
    if !shell_html.contains("<style data-nve-base") {
        style_tags.push_str("<style data-nve-base>");
        style_tags.push_str(ssr.base_css);
        style_tags.push_str("</style>\n");
    }
    if !shell_html.contains("<style data-nve-ssr") {
        style_tags.push_str("<style data-nve-ssr data-nve-hashes=\"");
        style_tags.push_str(&hashes_csv);
        style_tags.push_str("\">");
        style_tags.push_str(&ssr.style_css);
        style_tags.push_str("</style>\n");
    }

    let with_styles = if !style_tags.is_empty() {
        if let Some(head_end) = shell_html.find("</head>") {
            format!(
                "{}{style_tags}{}",
                &shell_html[..head_end],
                &shell_html[head_end..]
            )
        } else {
            format!("{style_tags}{shell_html}")
        }
    } else {
        shell_html.to_owned()
    };

    // Replace inner contents of `<main id="<mount_id>">...</main>` or `<div id="<mount_id>">...</div>`.
    let mut with_body = with_styles;
    let mut mount_close_end: Option<usize> = None;
    for tag in ["main", "div"] {
        let open_needle = format!("<{tag} id=\"{mount_id}\"");
        let close_needle = format!("</{tag}>");
        if let Some(open_pos) = with_body.find(&open_needle)
            && let Some(tag_end_rel) = with_body[open_pos..].find('>')
        {
            let content_start = open_pos + tag_end_rel + 1;
            if let Some(close_rel) = with_body[content_start..].find(&close_needle) {
                let content_end = content_start + close_rel;
                let replaced = format!(
                    "{}{}{}",
                    &with_body[..content_start],
                    ssr.html,
                    &with_body[content_end..]
                );
                mount_close_end = Some(content_start + ssr.html.len() + close_needle.len());
                with_body = replaced;
                break;
            }
        }
    }

    if !bootstrap_scripts.is_empty() && !with_body.contains("data-nve-snapshot-boot") {
        if let Some(idx) = with_body.find("<script type=\"module\">") {
            return format!(
                "{}{bootstrap_scripts}\n{}",
                &with_body[..idx],
                &with_body[idx..]
            );
        }
        if let Some(after_mount) = mount_close_end {
            return format!(
                "{}\n{bootstrap_scripts}{}",
                &with_body[..after_mount],
                &with_body[after_mount..]
            );
        }
        if let Some(idx) = with_body.find("</body>") {
            return format!(
                "{}{bootstrap_scripts}\n{}",
                &with_body[..idx],
                &with_body[idx..]
            );
        }
        return format!("{with_body}\n{bootstrap_scripts}");
    }

    with_body
}
