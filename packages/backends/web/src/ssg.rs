//! Generic, app-agnostic Web SSG + 0ms Instant-Restore document builder & shell injector.
//!
//! Works with **any** `nve_engine::NveApp` and **any** `WebBackend` mount container (`#app`,
//! `#nve-root`, etc.):
//! - Build-time [`prerender_nve_app_frame0`] renders Frame 0 of any `NveApp` into a caller-built
//!   [`WebBackend`] (with its `WebTableSystem` plugins) and returns the decoded [`NodeStore`].
//! - [`WebBackend::render_ssg_app_document`] emits a standalone HTML5 document containing:
//!   1. Build-time Frame-0 `<style data-nve-base>`, `<style data-nve-ssr data-nve-hashes="...">`
//!      (always present: the stable hydration anchor), and semantic HTML inside `#<mount_id>` so
//!      brand-new cold visitors paint at **0ms** while WASM downloads.
//!   2. Synchronous `<script data-nve-snapshot-boot>` (`< 900` B) right after `#<mount_id>` that
//!      clears `localStorage` on `?reset=1` (keeping clean build-time SSG HTML) or swaps in a
//!      returning user's last cached CSS+HTML from `localStorage` in `< 1ms` before first paint.
//!   3. `<script data-nve-replay>` arming `window.__nveReplayDrain()` for BOTH cold visitors (SSG)
//!      and returning users (cached snapshot).
//! - [`inject_ssg_into_shell_html`] / [`inject_ssg_with_scripts_into_shell_html`] inject the exact
//!   same build-time SSG + bootstrap scripts into any existing host `index.html` shell template.
//!
//! All `<style>` payloads go through `</style` breakout escaping.

use nve_abi::{NveDirectEvent, NveEventBatchRef, ROOT_NODE_ID};
use nve_components::tables::core_realization_order;
use nve_engine::{BackendError, NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_store::NodeStore;

use crate::WebBackend;
use crate::html::{push_attr_escaped, push_text_escaped};
use crate::snapshot::snapshot_and_replay_script_tags;
use crate::ssr::{SsrOutput, push_base_style_tag, push_ssr_style_tag};

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

/// Runs Frame 0 of **any** [`NveApp`] (`NVE2`) at `viewport` into `backend` and returns the
/// decoded `(NodeStore, WebBackend)` ready for [`WebBackend::render_ssr`] or
/// [`WebBackend::render_ssg_app_document`].
///
/// `backend` must already carry the app's `WebTableSystem` plugins: tags (`<form>`, semantic
/// HTML, canvas) and placement checks resolve through them.
pub fn prerender_nve_app_frame0<A: NveApp>(
    mut backend: WebBackend,
    viewport: Viewport,
) -> Result<(NodeStore, WebBackend), BackendError> {
    let mut app = A::create(1);
    if viewport.width.is_finite()
        && viewport.height.is_finite()
        && viewport.width > 0.0
        && viewport.height > 0.0
    {
        let layout = [NveDirectEvent::layout(
            1,
            ROOT_NODE_ID,
            0,
            0.0,
            0.0,
            viewport.width,
            viewport.height,
        )];
        // SAFETY: `layout` is a stack-local slice containing no nested pointers.
        let _ = app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&layout) });
    }
    backend.set_viewport(1, viewport);
    let mut replica = StoreWireReplica::new(backend, core_realization_order());
    let frame_bytes = app.render(1);
    if !frame_bytes.is_empty() {
        replica.submit_nve2_frame(frame_bytes)?;
    }
    Ok(replica.into_parts())
}

/// `trimmed` or `fallback` when blank.
fn non_blank<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

/// Appends `<tag attr="value">\n` for each non-empty `(tag, attr, value)` meta entry.
fn push_head_meta(entries: &[(&str, &str, Option<&str>)], out: &mut String) {
    for &(open, attr, value) in entries {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            out.push_str(open);
            out.push(' ');
            out.push_str(attr);
            out.push_str("=\"");
            push_attr_escaped(v, out);
            out.push_str("\">\n");
        }
    }
}

/// Appends `extra` followed by a newline unless empty.
fn push_line(extra: Option<&str>, out: &mut String) {
    if let Some(extra) = extra.filter(|s| !s.is_empty()) {
        out.push_str(extra);
        if !extra.ends_with('\n') {
            out.push('\n');
        }
    }
}

impl WebBackend {
    /// Emits a complete standalone HTML5 SSG document for **any** NVE web app with:
    /// - Build-time Frame-0 `<style data-nve-base>` and `<style data-nve-ssr data-nve-hashes="...">`
    /// - Build-time Frame-0 semantic HTML inside `<div id="{mount_id}">` (0ms paint for cold visitors)
    /// - Inline `<script data-nve-snapshot-boot>` (`< 900` B) for returning-user 0ms state override
    ///   and `?reset=1` cleanup
    /// - Inline `<script data-nve-replay>` arming `window.__nveReplayDrain()` before `body_scripts`
    pub fn render_ssg_app_document(
        &self,
        store: &NodeStore,
        opts: &WebSsgDocumentOptions,
    ) -> String {
        let ssr = self.render_ssr(store);
        let lang = non_blank(opts.lang.as_deref().unwrap_or(""), "en");
        let mount_id = non_blank(&opts.mount_id, "app");
        let app_slug = non_blank(&opts.app_slug, "app");
        let bootstrap =
            snapshot_and_replay_script_tags(mount_id, app_slug, opts.schema_version.max(1));

        let mut out = String::with_capacity(
            ssr.base_css.len() + ssr.style_css.len() + ssr.html.len() + bootstrap.len() + 512,
        );
        out.push_str("<!doctype html>\n<html lang=\"");
        push_attr_escaped(lang, &mut out);
        out.push_str("\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>");
        push_text_escaped(&opts.title, &mut out);
        out.push_str("</title>\n");
        let description = opts.description.as_deref();
        let canonical = opts.canonical_url.as_deref();
        push_head_meta(
            &[
                ("<meta name=\"description\"", "content", description),
                ("<meta property=\"og:description\"", "content", description),
                ("<meta property=\"og:title\"", "content", Some(&opts.title)),
                ("<link rel=\"canonical\"", "href", canonical),
                ("<meta property=\"og:url\"", "content", canonical),
                (
                    "<meta property=\"og:image\"",
                    "content",
                    opts.og_image.as_deref(),
                ),
            ],
            &mut out,
        );
        push_base_style_tag(&ssr, None, &mut out);
        out.push('\n');
        push_ssr_style_tag(&ssr, None, &mut out);
        out.push('\n');
        push_line(opts.head_extra.as_deref(), &mut out);
        out.push_str("</head>\n<body>\n<div id=\"");
        push_attr_escaped(mount_id, &mut out);
        out.push_str("\">");
        out.push_str(&ssr.html);
        out.push_str("</div>\n");
        out.push_str(&bootstrap);
        out.push('\n');
        push_line(opts.body_scripts.as_deref(), &mut out);
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

/// Byte range `(content_start, content_end, after_close)` of the element carrying
/// `id="<mount_id>"`, matching nested same-name tags by depth (`<div id="app"><div>…</div></div>`).
fn find_mount_element(html: &str, mount_id: &str) -> Option<(usize, usize, usize)> {
    let id_attr = format!(" id=\"{mount_id}\"");
    let id_pos = html.find(&id_attr)?;
    let tag_start = html[..id_pos].rfind('<')?;
    let tag_name = html[tag_start + 1..id_pos].split_whitespace().next()?;
    let content_start = id_pos + html[id_pos..].find('>')? + 1;
    let open = format!("<{tag_name}");
    let close = format!("</{tag_name}>");
    let mut depth = 1usize;
    let mut cursor = content_start;
    loop {
        let next_close = cursor + html[cursor..].find(&close)?;
        // Count same-name openings (`<div>` / `<div ...>`, not `<divider>`) before this close.
        let mut scan = cursor;
        while let Some(rel) = html[scan..next_close].find(&open) {
            let after = scan + rel + open.len();
            if html[after..].starts_with(['>', ' ', '\t', '\n', '\r', '/']) {
                depth += 1;
            }
            scan = after;
        }
        depth -= 1;
        if depth == 0 {
            return Some((content_start, next_close, next_close + close.len()));
        }
        cursor = next_close + close.len();
    }
}

/// Injects build-time Frame-0 SSG `<style data-nve-base>`, `<style data-nve-ssr data-nve-hashes>`,
/// `#<mount_id>` inner HTML, and caller-provided bootstrap `<script>` tags into a host shell HTML
/// template. Idempotent: style tags and bootstrap scripts already present in the shell are not
/// duplicated. Bootstrap scripts go before the first `<script type="module"`, else right after
/// the mount element, else before `</body>`.
pub fn inject_ssg_with_scripts_into_shell_html(
    shell_html: &str,
    mount_id: &str,
    ssr: &SsrOutput,
    bootstrap_scripts: &str,
) -> String {
    let mut style_tags = String::with_capacity(ssr.base_css.len() + ssr.style_css.len() + 128);
    if !shell_html.contains("<style data-nve-base") {
        push_base_style_tag(ssr, None, &mut style_tags);
        style_tags.push('\n');
    }
    if !shell_html.contains("<style data-nve-ssr") {
        push_ssr_style_tag(ssr, None, &mut style_tags);
        style_tags.push('\n');
    }
    let head_end = shell_html.find("</head>").unwrap_or(0);
    let mut out = String::with_capacity(
        shell_html.len() + style_tags.len() + ssr.html.len() + bootstrap_scripts.len() + 2,
    );
    out.push_str(&shell_html[..head_end]);
    out.push_str(&style_tags);
    out.push_str(&shell_html[head_end..]);

    let mount_close_end = find_mount_element(&out, mount_id).map(|(start, end, after)| {
        out.replace_range(start..end, &ssr.html);
        after - (end - start) + ssr.html.len()
    });

    if bootstrap_scripts.is_empty() || out.contains("data-nve-snapshot-boot") {
        return out;
    }
    let (at, scripts) = if let Some(idx) = out.find("<script type=\"module\"") {
        (idx, format!("{bootstrap_scripts}\n"))
    } else if let Some(after_mount) = mount_close_end {
        (after_mount, format!("\n{bootstrap_scripts}"))
    } else if let Some(idx) = out.find("</body>") {
        (idx, format!("{bootstrap_scripts}\n"))
    } else {
        (out.len(), format!("\n{bootstrap_scripts}"))
    };
    out.insert_str(at, &scripts);
    out
}
