//! 2-Stage Zero-JS Declarative Shadow DOM `<slot>` Streaming & Zero-Reload Live Updates:
//!
//! - **Stage 1 ([`StreamingSsrSession`])**: Out-of-order async SSR streaming using
//!   `<template shadowrootmode="open">` + `<slot name="nve-slot-{id}">{fallback}</slot>`.
//!   Resolved Light DOM `<div slot="nve-slot-{id}" data-nve-slot="{id}">...</div>` chunks stream
//!   in any completion order, then the document closes cleanly (`</div></body></html>`) with zero
//!   infinite spinner so `DOMContentLoaded` fires and WASM hydrates seamlessly.
//! - **Stage 2 ([`ZeroJsLiveSession`])**: Opt-in zero-JS interactive live mode over HTTP/2+
//!   chunked DSD using a hidden `<iframe name="nve-sink">` (`204 No Content` form actions),
//!   strictly per-slot scoped `[data-nve-slot="X"]:has(~ [data-nve-slot="X"]){display:none!important}`,
//!   enforced input/output slot separation, hard compaction cap (`DEFAULT_MAX_LIVE_UPDATES = 150`),
//!   and automatic HTTP/1.1 degradation to Stage-1 SSR + PRG `303 See Other`.

use core::fmt::{self, Write as _};
use std::collections::BTreeSet;

use crate::css::NVE_BASE_RESET_CSS;
use crate::html::{push_attr_escaped, push_text_escaped};
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

/// HTTP/1.1 chunked transfer terminator.
pub const HTTP_CHUNK_TERMINATOR: &str = "0\r\n\r\n";

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
/// Omits the hop-by-hop `Transfer-Encoding: chunked` header on HTTP/2+ (RFC 9113) while always
/// disabling reverse-proxy buffering (`X-Accel-Buffering: no`) and transform proxies
/// (`Cache-Control: no-cache, no-transform`).
pub fn streaming_http_headers(is_http2_or_higher: bool) -> Vec<(&'static str, &'static str)> {
    let mut headers = vec![
        ("Content-Type", "text/html; charset=utf-8"),
        ("X-Accel-Buffering", "no"),
        ("Cache-Control", "no-cache, no-transform"),
    ];
    if !is_http2_or_higher {
        headers.push(("Transfer-Encoding", "chunked"));
    }
    headers
}

/// Frames `payload` as a single HTTP/1.1 chunked transfer-encoding chunk (`<hex>\r\n<data>\r\n`).
pub fn encode_http_chunk(payload: &str) -> String {
    if payload.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(payload.len() + 16);
    let _ = write!(&mut out, "{:x}\r\n{payload}\r\n", payload.len());
    out
}

/// Errors produced by [`StreamingSsrSession`] and [`ZeroJsLiveSession`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamingError {
    ShellNotStarted,
    ShellAlreadyStarted,
    StreamClosed,
    InvalidSlotId(String),
    ImmutableInputSlot(String),
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
}

pub(crate) fn write_shared_styles(opts: &StreamingDocumentOptions, out: &mut String) {
    out.push_str("<style data-nve-base>");
    out.push_str(opts.base_css);
    out.push_str("</style>\n<style data-nve-ssr data-nve-hashes=\"");
    for (idx, h) in opts.rule_hashes.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(h);
    }
    out.push_str("\">");
    out.push_str(&opts.style_css);
    out.push_str("</style>\n");
    for href in &opts.stylesheet_links {
        if !href.trim().is_empty() {
            out.push_str("<link rel=\"stylesheet\" href=\"");
            push_attr_escaped(href.trim(), out);
            out.push_str("\">\n");
        }
    }
}

pub(crate) fn write_document_head(
    opts: &StreamingDocumentOptions,
    live_slot_css: &str,
    out: &mut String,
) {
    let lang = opts
        .lang
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("en");
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
        out.push_str("<style data-nve-live-slots>");
        out.push_str(live_slot_css);
        out.push_str("</style>\n");
    }
    if let Some(extra) = opts.head_extra.as_deref().filter(|s| !s.is_empty()) {
        out.push_str(extra);
        if !extra.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("</head>\n<body>\n");
}

pub(crate) fn format_light_dom_slot_chunk(
    clean_id: &str,
    resolved_html: &str,
    extra_style_css: Option<&str>,
    extra_live_rule: bool,
) -> String {
    let mut out = String::with_capacity(resolved_html.len() + 128);
    if extra_live_rule {
        let _ = write!(
            &mut out,
            "<style data-nve-live-slot=\"{clean_id}\">[data-nve-slot=\"{clean_id}\"]:has(~ [data-nve-slot=\"{clean_id}\"]){{display:none!important}}</style>"
        );
    }
    if let Some(css) = extra_style_css.filter(|s| !s.trim().is_empty()) {
        let _ = write!(
            &mut out,
            "<style data-nve-ssr-chunk=\"{clean_id}\">{css}</style>"
        );
    }
    let _ = writeln!(
        &mut out,
        "<div slot=\"{NVE_STREAM_SLOT_PREFIX}{clean_id}\" data-nve-slot=\"{clean_id}\">{resolved_html}</div>"
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

    /// Emits the immediate TTFB HTML shell with `<template shadowrootmode="open">` and embedded
    /// `<style data-nve-base>` / `<style data-nve-ssr>` so fallbacks paint immediately with 0 JS.
    pub fn begin_shell(&mut self, shadow_shell_html: &str) -> Result<String, StreamingError> {
        if self.shell_started {
            return Err(StreamingError::ShellAlreadyStarted);
        }
        self.shell_started = true;
        let mut out = String::with_capacity(shadow_shell_html.len() + self.opts.style_css.len() + 512);
        write_document_head(&self.opts, "", &mut out);
        out.push_str("<div id=\"app\" data-nve-stream-host=\"ssr\">\n<template shadowrootmode=\"open\">\n");
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
        if !self.shell_started {
            return Err(StreamingError::ShellNotStarted);
        }
        if self.closed {
            return Err(StreamingError::StreamClosed);
        }
        let clean = sanitize_stream_slot_id(slot_id)
            .ok_or_else(|| StreamingError::InvalidSlotId(slot_id.to_owned()))?;
        self.pending_slots.remove(&clean);
        self.resolved_order.push(clean.clone());
        Ok(format_light_dom_slot_chunk(&clean, resolved_html, extra_style_css, false))
    }

    /// Renders a resolved `WebBackend` fragment and flushes it as a Light DOM slot chunk.
    pub fn resolve_slot_from_backend(
        &mut self,
        slot_id: &str,
        backend: &WebBackend,
    ) -> Result<String, StreamingError> {
        let ssr = backend.render_ssr();
        let css = (!ssr.style_css.is_empty()).then_some(ssr.style_css.as_str());
        self.resolve_slot(slot_id, &ssr.html, css)
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

    /// Closes the DSD host and HTML document cleanly (`</div></body></html>`) so `DOMContentLoaded`
    /// fires with zero infinite spinner and WASM can hydrate seamlessly.
    pub fn finish(&mut self) -> Result<String, StreamingError> {
        if !self.shell_started {
            return Err(StreamingError::ShellNotStarted);
        }
        if self.closed {
            return Err(StreamingError::StreamClosed);
        }
        self.closed = true;
        let mut out = String::with_capacity(64);
        out.push_str("</div>\n");
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
