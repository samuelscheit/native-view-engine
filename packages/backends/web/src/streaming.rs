//! 2-Stage Zero-JS Declarative Shadow DOM `<slot>` Streaming & Zero-Reload Live Updates:
//!
//! - **Stage 1 ([`StreamingSsrSession`])**: Out-of-order async SSR streaming using an inner
//!   ephemeral `<div data-nve-dsd-host>` carrying `<template shadowrootmode="open">` +
//!   `<slot name="nve-slot-{id}">{fallback}</slot>` inside `<div id="app">`.
//!   Resolved Light DOM `<div slot="nve-slot-{id}" data-nve-slot="{id}" class="nve-slot">...</div>`
//!   chunks stream in any completion order, optionally interleaved with Option B mid-stream query
//!   payloads (`<script type="application/x-nve-query">`) and async module boot scripts, then the
//!   document closes cleanly (`<template data-nve-stream-end></template></div></div></body></html>`).
//! - **Stage 2 ([`ZeroJsLiveSession`])**: Opt-in zero-JS interactive live mode over HTTP/2+
//!   chunked DSD using a hidden `<iframe name="nve-sink">` (`204 No Content` form actions),
//!   strictly per-slot scoped `[data-nve-slot="X"]:has(~ [data-nve-slot="X"]){display:none!important}`,
//!   enforced input/output slot separation, hard compaction cap (`DEFAULT_MAX_LIVE_UPDATES = 150`),
//!   and automatic HTTP/1.1 degradation to Stage-1 SSR + PRG `303 See Other`.

use core::fmt::{self, Write as _};
use std::collections::BTreeSet;

use crate::css::NVE_BASE_RESET_CSS;
use crate::html::{
    push_async_boot_script, push_attr_escaped, push_query_payload_tag, push_style_css_escaped,
    push_stylesheet_links, push_text_escaped, write_style_nonce_attr,
};
use crate::ssr_stream::{SsrDeltaChunk, StreamingShellOutput};
use crate::WebBackend;

pub use crate::streaming_live::{
    decode_form_urlencoded, decode_form_urlencoded_map, FormActionResponse, LiveUpdateChunk,
    ZeroJsLiveConfig, ZeroJsLiveSession,
};

/// Prefix applied to Declarative Shadow DOM `<slot name="nve-slot-{id}">` markers.
pub const NVE_STREAM_SLOT_PREFIX: &str = "nve-slot-";

/// Target browsing context name for Stage-2 zero-JS `204 No Content` form submissions.
pub const NVE_LIVE_SINK_IFRAME_NAME: &str = "nve-sink";

/// Hidden zero-size iframe capturing Stage-2 zero-JS form action responses (`204 No Content`).
pub const NVE_LIVE_SINK_IFRAME_HTML: &str = "<iframe name=\"nve-sink\" hidden style=\"display:none;width:0;height:0;border:0\" aria-hidden=\"true\" tabindex=\"-1\"></iframe>";

/// Lightweight HTML comment flushed periodically to keep reverse proxies and CDNs alive.
pub const NVE_STREAM_KEEPALIVE_COMMENT: &str = "<!-- nve-keepalive -->\n";

/// Default hard cap on live slot updates before cleanly closing the stream and flipping form
/// targets off `nve-sink` for a single PRG `303 See Other` DOM compaction reload.
pub const DEFAULT_MAX_LIVE_UPDATES: usize = 150;

/// Negotiated streaming delivery strategy for an incoming HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingMode {
    /// Stage 1: Out-of-order async SSR streaming closing cleanly at `</html>`, paired with
    /// standard PRG `303 See Other` form submissions when JS/WASM is absent.
    AsyncSsrWithPrgFallback,
    /// Stage 2: Opt-in HTTP/2+ zero-JS live mode keeping the DSD host open with hidden
    /// `<iframe name="nve-sink">` `204 No Content` form actions and bounded compaction cap.
    ZeroJsLiveStream,
}

impl StreamingMode {
    /// Negotiates the safe streaming mode from protocol version and request opt-in.
    ///
    /// HTTP/1.1 requests ALWAYS degrade cleanly to [`Self::AsyncSsrWithPrgFallback`] to prevent
    /// the browser 6-connection-per-origin deadlock on long-lived streaming responses.
    #[inline]
    pub const fn negotiate(is_http2_or_higher: bool, requested_live: bool) -> Self {
        if is_http2_or_higher && requested_live {
            Self::ZeroJsLiveStream
        } else {
            Self::AsyncSsrWithPrgFallback
        }
    }
}

/// Required response headers for streaming Declarative Shadow DOM responses.
///
/// Leaves wire framing (`Transfer-Encoding: chunked` on HTTP/1.1, `DATA` frames on HTTP/2+)
/// entirely to Hyper while disabling shared caching, transform/buffering proxies, and MIME sniffing.
#[inline]
pub const fn streaming_http_headers() -> [(&'static str, &'static str); 5] {
    [
        ("Content-Type", "text/html; charset=utf-8"),
        ("Cache-Control", "private, no-store, no-transform"),
        ("Vary", "Cookie, Authorization"),
        ("X-Content-Type-Options", "nosniff"),
        ("X-Accel-Buffering", "no"),
    ]
}

/// Errors produced by [`StreamingSsrSession`], [`ZeroJsLiveSession`], and
/// [`WebBackend::render_ssr_streaming_shell`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamingError {
    ShellNotStarted,
    ShellAlreadyStarted,
    StreamClosed,
    InvalidSlotId(String),
    ImmutableInputSlot(String),
    InvalidSlotPlacement(String),
}

impl fmt::Display for StreamingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShellNotStarted => write!(f, "streaming shell has not been started yet"),
            Self::ShellAlreadyStarted => write!(f, "streaming shell was already emitted"),
            Self::StreamClosed => write!(f, "streaming session is already closed"),
            Self::InvalidSlotId(id) => write!(f, "invalid streaming slot id: {id:?}"),
            Self::ImmutableInputSlot(id) => {
                write!(f, "refused to overwrite immutable input slot {id:?}")
            }
            Self::InvalidSlotPlacement(reason) => {
                write!(f, "invalid streaming slot placement: {reason}")
            }
        }
    }
}

impl std::error::Error for StreamingError {}

/// Sanitizes a slot identifier to safe ASCII `[A-Za-z0-9_-]` characters for CSS selectors & attrs.
pub fn sanitize_stream_slot_id(raw: &str) -> Option<String> {
    let clean: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    if clean.is_empty() { None } else { Some(clean) }
}

/// Formats the canonical Shadow DOM slot name (`nve-slot-{id}`).
pub fn stream_slot_name(slot_id: &str) -> Option<String> {
    sanitize_stream_slot_id(slot_id).map(|clean| format!("{NVE_STREAM_SLOT_PREFIX}{clean}"))
}

/// Emits a Declarative Shadow DOM `<slot name="nve-slot-{id}">{fallback_html}</slot>` placeholder.
pub fn slot_placeholder_html(slot_id: &str, fallback_html: &str) -> Result<String, StreamingError> {
    let slot_name = stream_slot_name(slot_id)
        .ok_or_else(|| StreamingError::InvalidSlotId(slot_id.to_owned()))?;
    Ok(format!("<slot name=\"{slot_name}\">{fallback_html}</slot>"))
}

/// Emits strictly per-slot scoped CSS hiding earlier updates of each slot ID WITHOUT affecting
/// sibling slots or the latest update of any slot:
/// `[data-nve-slot="X"]:has(~ [data-nve-slot="X"]){display:none!important}`.
pub fn scoped_slot_hiding_css<'a>(slot_ids: impl IntoIterator<Item = &'a str>) -> String {
    let mut seen = BTreeSet::new();
    let mut css = String::new();
    for raw in slot_ids {
        if let Some(clean) = sanitize_stream_slot_id(raw)
            && seen.insert(clean.clone())
        {
            let _ = write!(
                &mut css,
                "[data-nve-slot=\"{clean}\"]:has(~ [data-nve-slot=\"{clean}\"]){{display:none!important}}"
            );
        }
    }
    css
}

/// Document head & stylesheet configuration shared by Stage-1 and Stage-2 streaming sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamingDocumentOptions {
    pub title: String,
    pub lang: Option<String>,
    pub description: Option<String>,
    pub base_css: &'static str,
    pub style_css: String,
    pub rule_hashes: Vec<String>,
    pub stylesheet_links: Vec<String>,
    pub head_extra: Option<String>,
    pub body_scripts: Option<String>,
    /// Optional per-request CSP `nonce` stamped onto every emitted `<style>` tag.
    pub style_nonce: Option<String>,
}

impl Default for StreamingDocumentOptions {
    fn default() -> Self {
        Self {
            title: String::new(),
            lang: None,
            description: None,
            base_css: NVE_BASE_RESET_CSS,
            style_css: String::new(),
            rule_hashes: Vec::new(),
            stylesheet_links: Vec::new(),
            head_extra: None,
            body_scripts: None,
            style_nonce: None,
        }
    }
}

impl StreamingDocumentOptions {
    /// Populates `base_css`, `style_css`, and `rule_hashes` from a rendered [`WebBackend`].
    pub fn from_backend(title: impl Into<String>, backend: &WebBackend) -> Self {
        let ssr = backend.render_ssr();
        Self {
            title: title.into(),
            base_css: ssr.base_css,
            style_css: ssr.style_css,
            rule_hashes: ssr.rule_hashes,
            ..Default::default()
        }
    }

    /// Populates `base_css`, `style_css`, and `rule_hashes` from a [`StreamingShellOutput`].
    pub fn from_streaming_shell(title: impl Into<String>, shell: &StreamingShellOutput) -> Self {
        Self {
            title: title.into(),
            base_css: shell.ssr.base_css,
            style_css: shell.ssr.style_css.clone(),
            rule_hashes: shell.ssr.rule_hashes.clone(),
            ..Default::default()
        }
    }

    /// Sets an opt-in per-request CSP `nonce` stamped onto every emitted `<style>` element.
    pub fn with_style_nonce(mut self, nonce: impl Into<String>) -> Self {
        let n = nonce.into();
        self.style_nonce = (!n.is_empty()).then_some(n);
        self
    }

    /// Sets extra raw HTML appended inside `<head>` (e.g. `<link rel="modulepreload" ...>`).
    pub fn with_head_extra(mut self, extra: impl Into<String>) -> Self {
        let e = extra.into();
        self.head_extra = (!e.is_empty()).then_some(e);
        self
    }
}

pub(crate) fn write_shared_styles(opts: &StreamingDocumentOptions, out: &mut String) {
    let nonce = opts.style_nonce.as_deref();
    out.push_str("<style data-nve-base");
    write_style_nonce_attr(nonce, out);
    out.push('>');
    push_style_css_escaped(opts.base_css, out);
    out.push_str("</style>\n<style data-nve-ssr");
    write_style_nonce_attr(nonce, out);
    out.push_str(" data-nve-hashes=\"");
    for (idx, h) in opts.rule_hashes.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        push_attr_escaped(h, out);
    }
    out.push_str("\">");
    push_style_css_escaped(&opts.style_css, out);
    out.push_str("</style>\n");
    push_stylesheet_links(&opts.stylesheet_links, out);
}

pub(crate) fn write_document_head(
    opts: &StreamingDocumentOptions,
    live_slot_css: &str,
    head_scripts: Option<&str>,
    out: &mut String,
) {
    let lang = opts
        .lang
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("en");
    let nonce = opts.style_nonce.as_deref();
    out.push_str("<!doctype html>\n<html lang=\"");
    push_attr_escaped(lang, out);
    out.push_str("\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>");
    push_text_escaped(&opts.title, out);
    out.push_str("</title>\n");
    if let Some(desc) = opts.description.as_deref().filter(|s| !s.is_empty()) {
        out.push_str("<meta name=\"description\" content=\"");
        push_attr_escaped(desc, out);
        out.push_str("\">\n");
    }
    write_shared_styles(opts, out);
    if !live_slot_css.is_empty() {
        out.push_str("<style data-nve-live-slots");
        write_style_nonce_attr(nonce, out);
        out.push('>');
        push_style_css_escaped(live_slot_css, out);
        out.push_str("</style>\n");
    }
    if let Some(extra) = opts.head_extra.as_deref().filter(|s| !s.is_empty()) {
        out.push_str(extra);
        if !extra.ends_with('\n') {
            out.push('\n');
        }
    }
    if let Some(scripts) = head_scripts.filter(|s| !s.is_empty()) {
        out.push_str(scripts);
        if !scripts.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("</head>\n<body>\n");
}

pub(crate) fn format_light_dom_slot_chunk(
    clean_id: &str,
    resolved_html: &str,
    extra_style_css: Option<&str>,
    rule_hashes: &[String],
    style_nonce: Option<&str>,
    extra_live_rule: bool,
) -> String {
    let mut out = String::with_capacity(resolved_html.len() + 160);
    if extra_live_rule {
        out.push_str("<style data-nve-live-slot=\"");
        out.push_str(clean_id);
        out.push('"');
        write_style_nonce_attr(style_nonce, &mut out);
        let _ = write!(
            &mut out,
            ">[data-nve-slot=\"{clean_id}\"]:has(~ [data-nve-slot=\"{clean_id}\"]){{display:none!important}}</style>"
        );
    }
    if let Some(css) = extra_style_css.filter(|s| !s.trim().is_empty()) {
        out.push_str("<style data-nve-ssr-chunk=\"");
        out.push_str(clean_id);
        out.push('"');
        write_style_nonce_attr(style_nonce, &mut out);
        out.push_str(" data-nve-hashes=\"");
        for (idx, h) in rule_hashes.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            push_attr_escaped(h, &mut out);
        }
        out.push_str("\">");
        push_style_css_escaped(css, &mut out);
        out.push_str("</style>");
    }
    // The trailing `data-nve-slot-ack` sibling marks the slot `<div>` as parser-complete, so a
    // TCP split right after `</div>` can never make an unfinished slot look complete.
    let _ = writeln!(
        &mut out,
        "<div slot=\"{NVE_STREAM_SLOT_PREFIX}{clean_id}\" data-nve-slot=\"{clean_id}\" class=\"nve-slot\">{resolved_html}</div><template data-nve-slot-ack=\"{clean_id}\"></template>"
    );
    out
}

/// Stage 1: Default Zero-JS Out-of-Order Async SSR Streaming session over Declarative Shadow DOM.
pub struct StreamingSsrSession {
    opts: StreamingDocumentOptions,
    pending_slots: BTreeSet<String>,
    resolved_order: Vec<String>,
    shell_started: bool,
    closed: bool,
}

impl StreamingSsrSession {
    pub fn new(
        opts: StreamingDocumentOptions,
        expected_slots: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let mut pending_slots = BTreeSet::new();
        for s in expected_slots {
            if let Some(clean) = sanitize_stream_slot_id(&s.into()) {
                pending_slots.insert(clean);
            }
        }
        Self {
            opts,
            pending_slots,
            resolved_order: Vec::new(),
            shell_started: false,
            closed: false,
        }
    }

    fn ensure_open(&self) -> Result<(), StreamingError> {
        if !self.shell_started {
            return Err(StreamingError::ShellNotStarted);
        }
        if self.closed {
            return Err(StreamingError::StreamClosed);
        }
        Ok(())
    }

    /// Emits the immediate TTFB HTML shell with an ephemeral `<div data-nve-dsd-host>` inside
    /// Light-DOM `<div id="app">`, keeping `#app` permanently Light DOM.
    pub fn begin_shell(&mut self, shadow_shell_html: &str) -> Result<String, StreamingError> {
        self.begin_shell_with_head_scripts(shadow_shell_html, "")
    }

    /// Emits the immediate TTFB HTML shell with additional `<head>` tags (`<link rel="modulepreload">`,
    /// etc.) right before `</head>`, and opens `<div id="app"><div data-nve-dsd-host><template shadowrootmode="open">`.
    pub fn begin_shell_with_head_scripts(
        &mut self,
        shadow_shell_html: &str,
        head_scripts: &str,
    ) -> Result<String, StreamingError> {
        if self.shell_started {
            return Err(StreamingError::ShellAlreadyStarted);
        }
        self.shell_started = true;
        let mut out = String::with_capacity(
            shadow_shell_html.len() + self.opts.style_css.len() + head_scripts.len() + 512,
        );
        write_document_head(&self.opts, "", Some(head_scripts), &mut out);
        out.push_str(
            "<div id=\"app\" data-nve-stream-host=\"ssr\">\n<div data-nve-dsd-host>\n<template shadowrootmode=\"open\">\n",
        );
        write_shared_styles(&self.opts, &mut out);
        out.push_str(shadow_shell_html);
        out.push_str("\n</template>\n");
        Ok(out)
    }

    /// Flushes a resolved slot chunk into Light DOM in any completion order (e.g. slot 2 before slot 1).
    pub fn resolve_slot(
        &mut self,
        slot_id: &str,
        resolved_html: &str,
        extra_style_css: Option<&str>,
    ) -> Result<String, StreamingError> {
        self.resolve_slot_with_hashes(slot_id, resolved_html, extra_style_css, &[])
    }

    /// Flushes a resolved slot chunk carrying explicit Option 5 Hybrid CSS `rule_hashes` for
    /// zero-duplicate hydration seeding.
    pub fn resolve_slot_with_hashes(
        &mut self,
        slot_id: &str,
        resolved_html: &str,
        extra_style_css: Option<&str>,
        rule_hashes: &[String],
    ) -> Result<String, StreamingError> {
        self.ensure_open()?;
        let clean = sanitize_stream_slot_id(slot_id)
            .ok_or_else(|| StreamingError::InvalidSlotId(slot_id.to_owned()))?;
        self.pending_slots.remove(&clean);
        self.resolved_order.push(clean.clone());
        Ok(format_light_dom_slot_chunk(
            &clean,
            resolved_html,
            extra_style_css,
            rule_hashes,
            self.opts.style_nonce.as_deref(),
            false,
        ))
    }

    /// Flushes a resolved [`SsrDeltaChunk`] produced by [`WebBackend::render_ssr_subtree_delta`]
    /// as one atomic chunk: the optional `(seq, base64 NVEQ)` payload (bound to this slot via
    /// `data-nve-slot-for`) strictly before the chunk CSS, `<div slot>`, and `data-nve-slot-ack`.
    pub fn resolve_slot_delta(
        &mut self,
        slot_id: &str,
        delta: &SsrDeltaChunk,
        query_payload: Option<(u64, &str)>,
    ) -> Result<String, StreamingError> {
        self.ensure_open()?;
        let clean = sanitize_stream_slot_id(slot_id)
            .ok_or_else(|| StreamingError::InvalidSlotId(slot_id.to_owned()))?;
        let mut out = String::new();
        if let Some((seq, b64)) = query_payload {
            push_query_payload_tag(seq, Some(&clean), b64, &mut out);
        }
        let css = (!delta.style_css.is_empty()).then_some(delta.style_css.as_str());
        out.push_str(&self.resolve_slot_with_hashes(&clean, &delta.html, css, &delta.rule_hashes)?);
        Ok(out)
    }

    /// Renders a resolved `WebBackend` fragment and flushes it as a Light DOM slot chunk.
    pub fn resolve_slot_from_backend(
        &mut self,
        slot_id: &str,
        backend: &WebBackend,
    ) -> Result<String, StreamingError> {
        let ssr = backend.render_ssr();
        let css = (!ssr.style_css.is_empty()).then_some(ssr.style_css.as_str());
        self.resolve_slot_with_hashes(slot_id, &ssr.html, css, &ssr.rule_hashes)
    }

    /// Emits an Option B mid-stream inert query payload chunk (`<script type="application/x-nve-query"
    /// data-nve-seq="{seq}">{b64}</script><template data-nve-q-ack="{seq}"></template>\n`).
    pub fn emit_query_payload(
        &mut self,
        seq: u64,
        base64_nveq: &str,
    ) -> Result<String, StreamingError> {
        self.ensure_open()?;
        let mut out = String::with_capacity(base64_nveq.len() + 112);
        push_query_payload_tag(seq, None, base64_nveq, &mut out);
        Ok(out)
    }

    /// Emits an Option B async ES module boot script (`<script type="module" async src="..."></script>\n`)
    /// so WASM can boot and hydrate mid-stream while slower slots continue resolving.
    pub fn emit_async_boot_script(&mut self, src: &str) -> Result<String, StreamingError> {
        self.ensure_open()?;
        let mut out = String::with_capacity(src.len() + 48);
        push_async_boot_script(Some(src), &mut out);
        Ok(out)
    }

    #[inline]
    pub fn keepalive_comment(&self) -> &'static str {
        NVE_STREAM_KEEPALIVE_COMMENT
    }

    #[inline]
    pub fn pending_slots(&self) -> &BTreeSet<String> {
        &self.pending_slots
    }

    #[inline]
    pub fn resolved_order(&self) -> &[String] {
        &self.resolved_order
    }

    #[inline]
    pub fn all_slots_resolved(&self) -> bool {
        self.pending_slots.is_empty()
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Closes the ephemeral DSD host, `#app` container, and HTML document cleanly
    /// (`<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n`) so
    /// `DOMContentLoaded` fires with zero infinite spinner and WASM detects stream completion.
    pub fn finish(&mut self) -> Result<String, StreamingError> {
        self.ensure_open()?;
        self.closed = true;
        let mut out = String::with_capacity(96);
        out.push_str("<template data-nve-stream-end></template></div></div>\n");
        if let Some(scripts) = self.opts.body_scripts.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(scripts);
            if !scripts.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("</body>\n</html>\n");
        Ok(out)
    }
}
