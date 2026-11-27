//! Configuration types (`SsrLocalPoolConfig`, `StreamPlan`, `SsrPageOptions`).

use core::time::Duration;

use http::StatusCode;

/// Default hard ceiling for the entire streaming SSR response (5 seconds).
pub const DEFAULT_TOTAL_DEADLINE: Duration = Duration::from_secs(5);

/// Default proxy/CDN keepalive HTML comment cadence (15 seconds).
pub const DEFAULT_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Default bounded chunk channel capacity between the `LocalSet` producer and Hyper's `ChannelBody`.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 16;

/// Default bounded pending-request queue depth per dedicated SSR worker thread.
pub const DEFAULT_QUEUE_CAPACITY_PER_WORKER: usize = 64;

/// Default maximum automatic SSR worker count when `worker_count` is not overridden.
pub const MAX_DEFAULT_WORKERS: usize = 16;

/// Response headers applied on the adaptive 0-suspense buffered Light-DOM fast path.
pub const FAST_PATH_HTTP_HEADERS: [(&str, &str); 4] = [
    ("Content-Type", "text/html; charset=utf-8"),
    ("Cache-Control", "private, no-store, no-transform"),
    ("Vary", "Cookie, Authorization"),
    ("X-Content-Type-Options", "nosniff"),
];

/// Configuration for [`crate::SsrLocalPool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SsrLocalPoolConfig {
    /// Number of dedicated OS worker threads running single-threaded Tokio `LocalSet` runtimes.
    /// Defaults to `available_parallelism().clamp(1, 16)`.
    pub worker_count: usize,
    /// Maximum queued + actively streaming renders per worker before
    /// [`crate::SsrError::PoolOverloaded`] (a render holds its slot until it completes, hits
    /// `total_deadline`, or the client disconnects).
    pub queue_capacity_per_worker: usize,
}

impl Default for SsrLocalPoolConfig {
    fn default() -> Self {
        let worker_count = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, MAX_DEFAULT_WORKERS);
        Self {
            worker_count,
            queue_capacity_per_worker: DEFAULT_QUEUE_CAPACITY_PER_WORKER,
        }
    }
}

impl SsrLocalPoolConfig {
    #[inline]
    pub fn with_worker_count(mut self, worker_count: usize) -> Self {
        self.worker_count = worker_count.max(1);
        self
    }

    #[inline]
    pub fn with_queue_capacity_per_worker(mut self, capacity: usize) -> Self {
        self.queue_capacity_per_worker = capacity.max(1);
        self
    }
}

/// Streaming timing and backpressure budget for [`crate::SsrLocalPool::render_streaming`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamPlan {
    /// Maximum wall-clock time spent waiting for pending suspense slots after the shell flushes.
    /// When exceeded, the server emits `<template data-nve-stream-end></template>` cleanly so the
    /// client recovers remaining pending slots via `QueryClient::finish_ssr_stream()`.
    pub total_deadline: Duration,
    /// Interval between `<!-- nve-keepalive -->\n` proxy heartbeats (`Duration::ZERO` disables
    /// keepalive comments).
    pub keepalive_interval: Duration,
    /// Bounded `mpsc::channel<Bytes>` capacity feeding Hyper's [`nve_http_hyper::ChannelBody`].
    pub channel_capacity: usize,
}

impl Default for StreamPlan {
    fn default() -> Self {
        Self {
            total_deadline: DEFAULT_TOTAL_DEADLINE,
            keepalive_interval: DEFAULT_KEEPALIVE_INTERVAL,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }
}

impl StreamPlan {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_total_deadline(mut self, total_deadline: Duration) -> Self {
        self.total_deadline = total_deadline;
        self
    }

    #[inline]
    pub fn with_keepalive_interval(mut self, keepalive_interval: Duration) -> Self {
        self.keepalive_interval = keepalive_interval;
        self
    }

    #[inline]
    pub fn with_channel_capacity(mut self, channel_capacity: usize) -> Self {
        self.channel_capacity = channel_capacity.max(1);
        self
    }
}

/// Per-request HTML document, CSP nonce, asset preloading, and status options for
/// [`crate::SsrLocalPool::render_streaming`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsrPageOptions {
    pub title: String,
    pub lang: Option<String>,
    pub description: Option<String>,
    pub head_extra: Option<String>,
    pub stylesheet_links: Vec<String>,
    pub body_scripts: Option<String>,
    /// Optional per-request CSP `nonce` stamped onto every emitted `<style>` tag.
    pub style_nonce: Option<String>,
    /// Optional ES module URL preloaded via `<link rel="modulepreload" href="...">` in `<head>`.
    pub modulepreload_src: Option<String>,
    /// Optional ES module URL emitted as `<script type="module" async src="..."></script>`
    /// strictly after `seq=0` query hydration ack (or `<template data-nve-stream-end>` on fast path).
    pub async_boot_src: Option<String>,
    /// Opt-in pre-hydration inline event replay snippet (`false` by default for strict zero-inline-JS CSP).
    pub enable_event_replay: bool,
    /// HTTP response status code (defaults to `200 OK`).
    pub status: StatusCode,
}

impl Default for SsrPageOptions {
    fn default() -> Self {
        Self {
            title: String::new(),
            lang: None,
            description: None,
            head_extra: None,
            stylesheet_links: Vec::new(),
            body_scripts: None,
            style_nonce: None,
            modulepreload_src: None,
            async_boot_src: None,
            enable_event_replay: false,
            status: StatusCode::OK,
        }
    }
}

#[inline]
fn non_empty(value: impl Into<String>) -> Option<String> {
    let s = value.into();
    (!s.is_empty()).then_some(s)
}

impl SsrPageOptions {
    #[inline]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    #[inline]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    #[inline]
    pub fn with_lang(mut self, lang: impl Into<String>) -> Self {
        self.lang = non_empty(lang);
        self
    }

    #[inline]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = non_empty(description);
        self
    }

    #[inline]
    pub fn with_head_extra(mut self, head_extra: impl Into<String>) -> Self {
        self.head_extra = non_empty(head_extra);
        self
    }

    #[inline]
    pub fn with_stylesheet_link(mut self, href: impl Into<String>) -> Self {
        let href = href.into();
        if !href.trim().is_empty() {
            self.stylesheet_links.push(href);
        }
        self
    }

    #[inline]
    pub fn with_stylesheet_links(
        mut self,
        links: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        for link in links {
            self = self.with_stylesheet_link(link);
        }
        self
    }

    #[inline]
    pub fn with_body_scripts(mut self, body_scripts: impl Into<String>) -> Self {
        self.body_scripts = non_empty(body_scripts);
        self
    }

    #[inline]
    pub fn with_style_nonce(mut self, nonce: impl Into<String>) -> Self {
        self.style_nonce = non_empty(nonce);
        self
    }

    #[inline]
    pub fn with_modulepreload_src(mut self, src: impl Into<String>) -> Self {
        self.modulepreload_src = non_empty(src);
        self
    }

    #[inline]
    pub fn with_async_boot_src(mut self, src: impl Into<String>) -> Self {
        self.async_boot_src = non_empty(src);
        self
    }

    #[inline]
    pub fn with_enable_event_replay(mut self, enable: bool) -> Self {
        self.enable_event_replay = enable;
        self
    }

    #[inline]
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }
}
