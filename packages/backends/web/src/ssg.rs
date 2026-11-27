//! Generic, app-agnostic Web SSG + 0ms Instant-Restore document builder & shell injector.

use nve_abi::{NveDirectEvent, NveEventBatchRef, ROOT_NODE_ID};
use nve_components::tables::core_realization_order;
use nve_engine::{BackendError, NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_store::NodeStore;

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
    pub mount_id: String,
    pub description: Option<String>,
    pub canonical_url: Option<String>,
    pub og_image: Option<String>,
    pub lang: Option<String>,
    pub head_extra: Option<String>,
    pub body_scripts: Option<String>,
}

/// Runs Frame 0 of **any** [`NveApp`] (`NVE2`) at `viewport` into a `StoreWireReplica<WebBackend>`
/// returning `(NodeStore, WebBackend)` ready for [`WebBackend::render_ssr`] or
/// [`WebBackend::render_ssg_app_document`].
pub fn prerender_nve_app_frame0<A: NveApp>(
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
        let _ = app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&layout) });
    }
    let frame_bytes = app.render(1).to_vec();
    let mut backend = WebBackend::new();
    backend.set_viewport(1, viewport);
    let mut replica = StoreWireReplica::new(backend, core_realization_order());
    if !frame_bytes.is_empty() {
        replica.submit_nve2_frame(&frame_bytes)?;
    }
    // Extract store & backend cleanly by re-rendering SSR or returning owned replica pair:
    let mut store = NodeStore::replica();
    store.set_realization_order(core_realization_order());
    let mut dec = nve_store::wire::WireDecoder::default();
    let mut changes = nve_store::ChangeSet::default();
    if !frame_bytes.is_empty() {
        let mut tx = store.tx();
        dec.apply(&mut tx, &frame_bytes)?;
        tx.commit_into(&mut changes);
    }
    let mut out_backend = WebBackend::new();
    out_backend.set_viewport(1, viewport);
    out_backend.commit(&store, &changes)?;
    Ok((store, out_backend))
}

impl WebBackend {
    /// Emits a complete standalone HTML5 SSG document for **any** NVE web app.
    pub fn render_ssg_app_document(
        &self,
        store: &NodeStore,
        opts: &WebSsgDocumentOptions,
    ) -> String {
        let ssr = self.render_ssr(store);
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
        let mut doc = String::with_capacity(ssr.html.len() + ssr.style_css.len() + 2048);
        doc.push_str("<!doctype html><html lang=\"");
        push_attr_escaped(lang, &mut doc);
        doc.push_str("\"><head><meta charset=\"utf-8\">");
        doc.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
        doc.push_str("<title>");
        push_text_escaped(&opts.title, &mut doc);
        doc.push_str("</title>");
        if let Some(ref desc) = opts.description {
            doc.push_str("<meta name=\"description\" content=\"");
            push_attr_escaped(desc, &mut doc);
            doc.push_str("\">");
        }
        if let Some(ref canon) = opts.canonical_url {
            doc.push_str("<link rel=\"canonical\" href=\"");
            push_attr_escaped(canon, &mut doc);
            doc.push_str("\">");
        }
        if let Some(ref og) = opts.og_image {
            doc.push_str("<meta property=\"og:image\" content=\"");
            push_attr_escaped(og, &mut doc);
            doc.push_str("\">");
        }
        doc.push_str("<style data-nve-base>");
        doc.push_str(ssr.base_css);
        doc.push_str("</style>");
        if !ssr.style_css.is_empty() {
            doc.push_str("<style data-nve-ssr data-nve-hashes=\"");
            for (i, h) in ssr.rule_hashes.iter().enumerate() {
                if i > 0 {
                    doc.push(',');
                }
                doc.push_str(h);
            }
            doc.push_str("\">");
            doc.push_str(&ssr.style_css);
            doc.push_str("</style>");
        }
        if let Some(ref extra) = opts.head_extra {
            doc.push_str(extra);
        }
        doc.push_str("</head><body><div id=\"");
        push_attr_escaped(mount_id, &mut doc);
        doc.push_str("\">");
        doc.push_str(&ssr.html);
        doc.push_str("</div>");
        doc.push_str(&snapshot_and_replay_script_tags(
            app_slug,
            mount_id,
            opts.schema_version,
        ));
        if let Some(ref scripts) = opts.body_scripts {
            doc.push_str(scripts);
        }
        doc.push_str("</body></html>");
        doc
    }
}

/// Injects pre-rendered [`SsrOutput`] styles + HTML into an existing host `index.html` shell.
pub fn inject_ssg_into_shell_html(shell_html: &str, mount_id: &str, ssr: &SsrOutput) -> String {
    inject_ssg_with_scripts_into_shell_html(shell_html, mount_id, ssr, "")
}

/// Injects pre-rendered [`SsrOutput`] + inline bootstrap scripts into an existing host `index.html`.
pub fn inject_ssg_with_scripts_into_shell_html(
    shell_html: &str,
    mount_id: &str,
    ssr: &SsrOutput,
    after_mount_scripts: &str,
) -> String {
    let mut style_block = String::with_capacity(ssr.base_css.len() + ssr.style_css.len() + 128);
    style_block.push_str("<style data-nve-base>");
    style_block.push_str(ssr.base_css);
    style_block.push_str("</style>");
    if !ssr.style_css.is_empty() {
        style_block.push_str("<style data-nve-ssr data-nve-hashes=\"");
        for (i, h) in ssr.rule_hashes.iter().enumerate() {
            if i > 0 {
                style_block.push(',');
            }
            style_block.push_str(h);
        }
        style_block.push_str("\">");
        style_block.push_str(&ssr.style_css);
        style_block.push_str("</style>");
    }

    let with_styles = if let Some(idx) = shell_html.find("</head>") {
        let mut s = String::with_capacity(shell_html.len() + style_block.len());
        s.push_str(&shell_html[..idx]);
        s.push_str(&style_block);
        s.push_str(&shell_html[idx..]);
        s
    } else {
        format!("{style_block}{shell_html}")
    };

    // Match any mount tag (`<main id="app">`, `<div id="app">`, etc.) even with extra attributes.
    let id_attr = format!("id=\"{mount_id}\"");
    if let Some(id_pos) = with_styles.find(&id_attr)
        && let Some(tag_start) = with_styles[..id_pos].rfind('<')
        && let Some(rel_open_end) = with_styles[id_pos..].find('>')
    {
        let open_end = id_pos + rel_open_end + 1;
        let tag_name = with_styles[tag_start + 1..id_pos]
            .split_whitespace()
            .next()
            .unwrap_or("div");
        let close_tag = format!("</{tag_name}>");
        if let Some(rel_close) = with_styles[open_end..].find(&close_tag) {
            let close_pos = open_end + rel_close;
            let after_close = close_pos + close_tag.len();
            let mut out = String::with_capacity(
                with_styles.len() + ssr.html.len() + after_mount_scripts.len(),
            );
            out.push_str(&with_styles[..open_end]);
            out.push_str(&ssr.html);
            out.push_str(&close_tag);
            out.push_str(after_mount_scripts);
            out.push_str(&with_styles[after_close..]);
            return out;
        }
    }
    if !after_mount_scripts.is_empty()
        && let Some(body_end) = with_styles.find("</body>")
    {
        let mut out = String::with_capacity(with_styles.len() + after_mount_scripts.len());
        out.push_str(&with_styles[..body_end]);
        out.push_str(after_mount_scripts);
        out.push_str(&with_styles[body_end..]);
        return out;
    }
    with_styles
}
