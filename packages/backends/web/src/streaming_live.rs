//! Stage-2 Opt-In Zero-JS Interactive Live Mode ([`ZeroJsLiveSession`]); servers decode its form
//! bodies with `nve_rpc::FormPairs` (the single `application/x-www-form-urlencoded` decoder).

use std::collections::BTreeSet;

use crate::streaming::{
    format_light_dom_slot_chunk, sanitize_stream_slot_id, scoped_slot_hiding_css,
    write_document_head, write_shared_styles, StreamingDocumentOptions, StreamingError,
    StreamingMode, DEFAULT_MAX_LIVE_UPDATES, NVE_LIVE_SINK_IFRAME_HTML,
    NVE_LIVE_SINK_IFRAME_NAME, NVE_STREAM_KEEPALIVE_COMMENT,
};

/// Configuration for Stage-2 [`ZeroJsLiveSession`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroJsLiveConfig {
    pub mode: StreamingMode,
    pub max_live_updates: usize,
    pub compaction_redirect_url: String,
    pub input_slots: BTreeSet<String>,
    pub output_slots: Vec<String>,
}

impl Default for ZeroJsLiveConfig {
    fn default() -> Self {
        Self {
            mode: StreamingMode::ZeroJsLiveStream,
            max_live_updates: DEFAULT_MAX_LIVE_UPDATES,
            compaction_redirect_url: "/".to_owned(),
            input_slots: BTreeSet::new(),
            output_slots: Vec::new(),
        }
    }
}

impl ZeroJsLiveConfig {
    pub fn negotiated(is_http2_or_higher: bool, requested_live: bool) -> Self {
        Self {
            mode: StreamingMode::negotiate(is_http2_or_higher, requested_live),
            ..Default::default()
        }
    }

    pub fn with_max_live_updates(mut self, max_updates: usize) -> Self {
        self.max_live_updates = max_updates.max(1);
        self
    }

    pub fn with_compaction_redirect(mut self, url: impl Into<String>) -> Self {
        self.compaction_redirect_url = url.into();
        self
    }

    pub fn with_input_slot(mut self, slot_id: impl Into<String>) -> Self {
        if let Some(clean) = sanitize_stream_slot_id(&slot_id.into()) {
            self.input_slots.insert(clean);
        }
        self
    }

    pub fn with_output_slot(mut self, slot_id: impl Into<String>) -> Self {
        if let Some(clean) = sanitize_stream_slot_id(&slot_id.into())
            && !self.output_slots.contains(&clean)
        {
            self.output_slots.push(clean);
        }
        self
    }
}

/// HTTP response action a server should send for a form POST under [`ZeroJsLiveSession`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormActionResponse {
    /// Live stream is active below compaction cap: respond `204 No Content` to `target="nve-sink"`.
    NoContent204,
    /// HTTP/1.1 fallback or compaction cap reached: respond `303 See Other` PRG reload.
    SeeOther303 { location: String },
}

/// Output produced by [`ZeroJsLiveSession::push_slot_update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveUpdateChunk {
    pub slot_html: String,
    pub closing_html: Option<String>,
    pub update_index: usize,
    pub compaction_triggered: bool,
}

impl LiveUpdateChunk {
    /// Combines the slot chunk and optional closing document trailer into one wire string.
    pub fn into_combined_html(mut self) -> String {
        if let Some(closing) = self.closing_html {
            self.slot_html.push_str(&closing);
        }
        self.slot_html
    }
}

/// Stage 2: Opt-in Zero-JS Interactive Live Mode session over an open HTTP/2+ chunked DSD stream.
pub struct ZeroJsLiveSession {
    opts: StreamingDocumentOptions,
    config: ZeroJsLiveConfig,
    known_output_slots: BTreeSet<String>,
    live_update_count: usize,
    shell_started: bool,
    compaction_required: bool,
    closed: bool,
}

impl ZeroJsLiveSession {
    pub fn new(opts: StreamingDocumentOptions, config: ZeroJsLiveConfig) -> Self {
        let known_output_slots: BTreeSet<String> = config
            .output_slots
            .iter()
            .filter_map(|s| sanitize_stream_slot_id(s))
            .collect();
        Self {
            opts,
            config,
            known_output_slots,
            live_update_count: 0,
            shell_started: false,
            compaction_required: false,
            closed: false,
        }
    }

    #[inline]
    pub fn mode(&self) -> StreamingMode {
        self.config.mode
    }

    #[inline]
    pub fn live_update_count(&self) -> usize {
        self.live_update_count
    }

    #[inline]
    pub fn max_live_updates(&self) -> usize {
        self.config.max_live_updates
    }

    #[inline]
    pub fn needs_compaction(&self) -> bool {
        self.compaction_required
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Returns `Some("nve-sink")` while the live stream is active below the compaction cap,
    /// or `None` when degraded to HTTP/1.1 or after compaction cap is reached (flipping forms
    /// cleanly to PRG `303 See Other`).
    #[inline]
    pub fn form_target_attr(&self) -> Option<&'static str> {
        if self.config.mode == StreamingMode::ZeroJsLiveStream
            && !self.closed
            && !self.compaction_required
        {
            Some(NVE_LIVE_SINK_IFRAME_NAME)
        } else {
            None
        }
    }

    /// Returns the HTTP response status the form POST endpoint should emit (`204 No Content`
    /// while live, or `303 See Other` when degraded/compacted).
    pub fn form_action_response(&self) -> FormActionResponse {
        if self.form_target_attr().is_some() {
            FormActionResponse::NoContent204
        } else {
            FormActionResponse::SeeOther303 {
                location: self.config.compaction_redirect_url.clone(),
            }
        }
    }

    /// Emits the initial HTML shell.
    ///
    /// - In [`StreamingMode::ZeroJsLiveStream`]: emits scoped `:has(~)` CSS rules, the hidden
    ///   `<iframe name="nve-sink">`, and an open Declarative Shadow DOM host.
    /// - In [`StreamingMode::AsyncSsrWithPrgFallback`]: emits a standard closed Stage-1 shell so
    ///   HTTP/1.1 clients never hold an open streaming connection.
    pub fn begin_shell(&mut self, shadow_shell_html: &str) -> Result<String, StreamingError> {
        if self.shell_started {
            return Err(StreamingError::ShellAlreadyStarted);
        }
        self.shell_started = true;

        let live_css = if self.config.mode == StreamingMode::ZeroJsLiveStream {
            let ids: Vec<&str> = self.known_output_slots.iter().map(String::as_str).collect();
            scoped_slot_hiding_css(ids)
        } else {
            String::new()
        };

        let mut out = String::with_capacity(shadow_shell_html.len() + live_css.len() + 512);
        write_document_head(&self.opts, &live_css, None, &mut out);
        if self.config.mode == StreamingMode::ZeroJsLiveStream {
            out.push_str(NVE_LIVE_SINK_IFRAME_HTML);
            out.push('\n');
            out.push_str(
                "<div id=\"app\" data-nve-stream-host=\"live\">\n<div data-nve-dsd-host>\n<template shadowrootmode=\"open\">\n",
            );
            write_shared_styles(&self.opts, &mut out);
            out.push_str(shadow_shell_html);
            out.push_str("\n</template>\n");
        } else {
            out.push_str(
                "<div id=\"app\" data-nve-stream-host=\"prg-fallback\">\n<div data-nve-dsd-host>\n<template shadowrootmode=\"open\">\n",
            );
            write_shared_styles(&self.opts, &mut out);
            out.push_str(shadow_shell_html);
            out.push_str("\n</template>\n<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n");
            self.closed = true;
        }
        Ok(out)
    }

    /// Flushes a live Light DOM slot update on the open DSD stream, enforcing input/output slot
    /// separation and the hard compaction cap (`max_live_updates`).
    pub fn push_slot_update(
        &mut self,
        slot_id: &str,
        updated_html: &str,
        extra_style_css: Option<&str>,
    ) -> Result<LiveUpdateChunk, StreamingError> {
        if !self.shell_started {
            return Err(StreamingError::ShellNotStarted);
        }
        if self.closed || self.config.mode != StreamingMode::ZeroJsLiveStream {
            return Err(StreamingError::StreamClosed);
        }
        let clean = sanitize_stream_slot_id(slot_id)
            .ok_or_else(|| StreamingError::InvalidSlotId(slot_id.to_owned()))?;
        if self.config.input_slots.contains(&clean) {
            return Err(StreamingError::ImmutableInputSlot(clean));
        }

        let is_new_output = self.known_output_slots.insert(clean.clone());
        let slot_html = format_light_dom_slot_chunk(
            &clean,
            updated_html,
            extra_style_css,
            &[],
            self.opts.style_nonce.as_deref(),
            is_new_output,
        );
        self.live_update_count += 1;
        let update_index = self.live_update_count;

        let compaction_triggered = self.live_update_count >= self.config.max_live_updates;
        let closing_html = if compaction_triggered {
            self.compaction_required = true;
            self.closed = true;
            Some("<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n".to_owned())
        } else {
            None
        };

        Ok(LiveUpdateChunk {
            slot_html,
            closing_html,
            update_index,
            compaction_triggered,
        })
    }

    #[inline]
    pub fn keepalive_comment(&self) -> &'static str {
        NVE_STREAM_KEEPALIVE_COMMENT
    }

    /// Cleanly closes an open Stage-2 live stream.
    pub fn close(&mut self) -> Option<String> {
        if !self.shell_started || self.closed {
            return None;
        }
        self.closed = true;
        Some("<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n".to_owned())
    }
}
