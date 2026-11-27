//! Shared typed procedures, controllable RPC transport, and HTML/NVEQ helpers for the
//! `nve-ssr` integration suites.

#![allow(dead_code)]

use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::time::Duration;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::task::Waker;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::task::Wake;
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bytes::Bytes;
use http_body_util::BodyExt as _;
use minicbor::{Decode, Encode};
use nve_backend_web::{WebBackend, WebTableSystem};
use nve_store::{NodeId, NodeStore, NveTable, Str, TableSlot};
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_http_hyper::HttpBody;
use nve_query::{QueryOptions, nveq_entry_count};
use nve_query_dioxus::prelude::*;
use nve_rpc::{
    BoxFuture, CborCodec, DomainStatus, Procedure, RawCall, RawReply, ReplyKind, RpcCodec,
    RpcStatus, Transport, TransportError, procedures,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, oneshot};
use http::{Request, Response};
use hyper::body::Incoming;
use nve_http_hyper::serve_service_with_shutdown;
use nve_ssr::{SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};

// ---------------------------------------------------------------------------
// Shared typed procedures & test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PanelData {
    #[n(0)]
    pub key: String,
    #[n(1)]
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub enum PanelErr {
    #[n(0)]
    Forbidden(#[n(0)] String),
}

impl core::fmt::Display for PanelErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Forbidden(msg) => write!(f, "forbidden: {msg}"),
        }
    }
}

impl DomainStatus for PanelErr {
    fn rpc_status(&self) -> RpcStatus {
        RpcStatus::PermissionDenied
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ProfileData {
    #[n(0)]
    pub user: String,
    #[n(1)]
    pub secret: String,
}

procedures! {
    pub query GetPanel = "ssr.test.v1/GetPanel" (String) -> PanelData, PanelErr;
    pub query GetProfile = "ssr.test.v1/GetProfile" (u32) -> ProfileData, ();
}

pub fn cbor_encode<T: Encode<()> + for<'b> Decode<'b, ()>>(val: &T) -> Bytes {
    let mut buf = Vec::new();
    CborCodec::default()
        .encode_into(val, &mut buf)
        .expect("encode CBOR");
    Bytes::from(buf)
}

/// Extracts all `(seq, decoded_nveq_bytes)` chunks from HTML in document order, also verifying
/// that every `<script type="application/x-nve-query" data-nve-seq="N">` is immediately followed
/// by `<template data-nve-q-ack="N"></template>`.
pub fn extract_nveq_chunks(html: &str) -> Vec<(u64, Vec<u8>)> {
    let mut out = Vec::new();
    let mut rest = html;
    let marker = "<script type=\"application/x-nve-query\" data-nve-seq=\"";
    while let Some(pos) = rest.find(marker) {
        let after = &rest[pos + marker.len()..];
        let q_end = after.find('"').expect("closing attr quote on data-nve-seq");
        let seq: u64 = after[..q_end].parse().expect("numeric data-nve-seq");
        // Slot chunk payloads carry `data-nve-slot-for="{id}"` binding them to their markup.
        let tag_end = after.find('>').expect("end of nve-query start tag");
        let attrs = &after[q_end + 1..tag_end];
        assert!(
            attrs.is_empty() || (attrs.starts_with(" data-nve-slot-for=\"") && attrs.ends_with('"')),
            "unexpected nve-query attrs {attrs:?}"
        );
        let payload_start = &after[tag_end + 1..];
        let close = "</script>";
        let close_pos = payload_start
            .find(close)
            .expect("closing </script> on nve-query");
        let b64 = &payload_start[..close_pos];
        let after_close = &payload_start[close_pos + close.len()..];
        let expected_ack = format!("<template data-nve-q-ack=\"{seq}\"></template>");
        assert!(
            after_close.starts_with(&expected_ack),
            "missing immediate ack for seq={seq}: {after_close}"
        );
        let decoded = BASE64_STANDARD
            .decode(b64)
            .expect("valid base64 NVEQ payload");
        out.push((seq, decoded));
        rest = &after_close[expected_ack.len()..];
    }
    out
}

/// Hydrates all extracted `NVEQ` chunks (`seq=0` via `hydrate_bytes`, `seq>0` via
/// `hydrate_stream_chunk_bytes`) into a client `ErasedQueryClient`.
pub fn hydrate_client_from_html(client: &ErasedQueryClient, html: &str) -> Vec<(u64, usize, u32)> {
    let chunks = extract_nveq_chunks(html);
    let mut stats = Vec::with_capacity(chunks.len());
    for (seq, bytes) in chunks {
        let entry_count = nveq_entry_count(&bytes).expect("valid NVEQ v2 header");
        let applied = if seq == 0 {
            client.hydrate_bytes(&bytes).expect("hydrate seq=0")
        } else {
            client
                .hydrate_stream_chunk_bytes(&bytes)
                .expect("hydrate seq>0")
        };
        stats.push((seq, applied, entry_count));
    }
    stats
}

pub struct FlagWaker(AtomicBool);
impl Wake for FlagWaker {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }
}

pub fn pump_client_settled(surface: &mut DioxusSurface, web: &mut WebBackend, frame: &mut u64) {
    let waker = Waker::from(Arc::new(FlagWaker(AtomicBool::new(false))));
    for _ in 0..32 {
        let mut progressed = false;
        while surface.poll_work(&waker) {
            *frame += 1;
            surface.commit_to(*frame, web).expect("commit frame");
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
}

/// Controllable async transport supporting per-key gates, domain errors, call counts, and drop
/// guards for cancellation tests.
#[derive(Clone, Default)]
pub struct ControlledTransport {
    gates: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
    released: Arc<Mutex<HashMap<String, bool>>>,
    responses: Arc<Mutex<HashMap<String, Result<PanelData, PanelErr>>>>,
    calls: Arc<AtomicUsize>,
    hang_forever: Arc<Mutex<HashMap<String, bool>>>,
    on_started: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
    on_dropped: Arc<Mutex<HashMap<String, oneshot::Sender<Instant>>>>,
}

impl ControlledTransport {
    pub fn set_ok(&self, key: &str, body: &str) {
        self.responses.lock().expect("lock not poisoned").insert(
            key.to_owned(),
            Ok(PanelData {
                key: key.to_owned(),
                body: body.to_owned(),
            }),
        );
    }

    pub fn set_domain_err(&self, key: &str, msg: &str) {
        self.responses
            .lock()
            .expect("lock not poisoned")
            .insert(key.to_owned(), Err(PanelErr::Forbidden(msg.to_owned())));
    }

    pub fn gate(&self, key: &str) {
        self.gates
            .lock()
            .expect("lock not poisoned")
            .insert(key.to_owned(), Arc::new(Notify::new()));
        self.released
            .lock()
            .expect("lock not poisoned")
            .insert(key.to_owned(), false);
    }

    pub fn release(&self, key: &str) {
        self.released
            .lock()
            .expect("lock not poisoned")
            .insert(key.to_owned(), true);
        if let Some(n) = self
            .gates
            .lock()
            .expect("lock not poisoned")
            .get(key)
            .cloned()
        {
            n.notify_waiters();
        }
    }

    pub fn hang_with_drop_guard(
        &self,
        key: &str,
        started_tx: oneshot::Sender<()>,
        dropped_tx: oneshot::Sender<Instant>,
    ) {
        self.hang_forever
            .lock()
            .expect("lock not poisoned")
            .insert(key.to_owned(), true);
        self.on_started
            .lock()
            .expect("lock not poisoned")
            .insert(key.to_owned(), started_tx);
        self.on_dropped
            .lock()
            .expect("lock not poisoned")
            .insert(key.to_owned(), dropped_tx);
    }

    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

pub struct DropNotifier(Option<oneshot::Sender<Instant>>);
impl Drop for DropNotifier {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(Instant::now());
        }
    }
}

impl Transport for ControlledTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let state = self.clone();
        Box::pin(async move {
            assert_eq!(req.name, GetPanel::NAME);
            let key: String = CborCodec::default()
                .decode_bounded(&req.body)
                .expect("decode RPC args");
            state.calls.fetch_add(1, Ordering::SeqCst);

            let hang = state
                .hang_forever
                .lock()
                .expect("lock not poisoned")
                .get(&key)
                .copied()
                .unwrap_or(false);
            if hang {
                let drop_tx = state
                    .on_dropped
                    .lock()
                    .expect("lock not poisoned")
                    .remove(&key);
                let _guard = DropNotifier(drop_tx);
                if let Some(started) = state
                    .on_started
                    .lock()
                    .expect("lock not poisoned")
                    .remove(&key)
                {
                    let _ = started.send(());
                }
                core::future::pending::<()>().await;
            }

            let gate = state
                .gates
                .lock()
                .expect("lock not poisoned")
                .get(&key)
                .cloned();
            if let Some(notify) = gate {
                loop {
                    let notified = notify.notified();
                    let is_released = state
                        .released
                        .lock()
                        .expect("lock not poisoned")
                        .get(&key)
                        .copied()
                        .unwrap_or(true);
                    if is_released {
                        break;
                    }
                    notified.await;
                }
            }

            let outcome = state
                .responses
                .lock()
                .expect("lock not poisoned")
                .get(&key)
                .cloned()
                .unwrap_or_else(|| {
                    Ok(PanelData {
                        key: key.clone(),
                        body: format!("default:{key}"),
                    })
                });

            match outcome {
                Ok(data) => Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: cbor_encode(&data),
                }),
                Err(err) => Ok(RawReply {
                    http_status: 403,
                    kind: ReplyKind::Domain,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: cbor_encode(&err),
                }),
            }
        })
    }
}

pub async fn read_next_chunk(body: &mut HttpBody) -> Option<String> {
    let frame = body.frame().await?.expect("infallible HttpBody frame");
    let data = frame.into_data().expect("data frame");
    Some(String::from_utf8(data.to_vec()).expect("valid utf-8 HTML chunk"))
}

pub async fn collect_all_html(body: HttpBody) -> String {
    let bytes = body.collect().await.expect("collect HttpBody").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("valid utf-8 HTML")
}

/// Single `<QuerySuspense>` shell around a `GetPanel("slow")` query (cancellation and drain
/// scenarios gate or hang the `slow` key).
#[component]
pub fn SlowPanelApp() -> Element {
    let client = dioxus_core::consume_context::<ErasedQueryClient>();
    rsx! {
        QueryClientProvider {
            client: client,
            view {
                text { "Disconnect Shell" }
                QuerySuspense {
                    id: Some("slow-rpc".into()),
                    fallback: rsx! { text { "Waiting on slow RPC..." } },
                    SlowPanelInner {}
                }
            }
        }
    }
}

#[component]
pub fn SlowPanelInner() -> Element {
    let q = use_query_with::<GetPanel>("slow".to_owned(), QueryOptions::default().max_retries(0));
    let data = q.suspend()?;
    rsx! { text { "{data.body}" } }
}

/// 3P semantic-tag override for tags no `nve-web-html` element covers (`<select>`): the
/// streaming placement hygiene must honor tags supplied by any `WebTableSystem`.
#[derive(NveTable)]
#[nve_table(name = "test.ssr_tag_override", storage = "sparse", flags = "publish")]
pub struct TagOverride {
    #[effect(realize)]
    tag: Str,
}

struct TagOverrideSystem;

impl WebTableSystem for TagOverrideSystem {
    fn slot(&self) -> TableSlot {
        TagOverride::slot()
    }

    fn tag_name<'a>(&self, store: &'a NodeStore, node: NodeId) -> Option<&'a str> {
        store.try_row::<TagOverride>(node).map(|r| r.tag())
    }
}

/// [`web_backend`] plus the [`TagOverride`] system.
pub fn web_backend_with_tag_override() -> WebBackend {
    let mut backend = web_backend();
    backend.register_table_system(TagOverrideSystem);
    backend
}

/// The app's per-request SSR backend: core tables plus `nve-web-html` plugins (`<form>` from
/// `HtmlForm`, semantic tags), matching what the client registers.
pub fn web_backend() -> WebBackend {
    let mut backend = WebBackend::new();
    nve_web_html_web::register_web_html_systems(backend.table_registry_mut());
    backend
}

#[derive(Clone)]
struct LoopbackSsrService {
    pool: SsrLocalPool,
    transport: ControlledTransport,
    keepalive: Duration,
}

impl hyper::service::Service<Request<Incoming>> for LoopbackSsrService {
    type Response = Response<HttpBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, _req: Request<Incoming>) -> Self::Future {
        let pool = self.pool.clone();
        let transport = self.transport.clone();
        let keepalive = self.keepalive;
        Box::pin(async move {
            let client = erased_ssr_query_client(transport);
            let plan = StreamPlan::default()
                .with_total_deadline(Duration::from_secs(10))
                .with_keepalive_interval(keepalive);
            let resp = pool
                .render_streaming(SsrPageOptions::new("Disconnect Test", web_backend), plan, client, |c| {
                    DioxusSurface::new(1, SlowPanelApp).with_context(c)
                })
                .await
                .expect("streaming SSR response");
            Ok(resp)
        })
    }
}

/// Serves `SlowPanelApp` over real loopback HTTP/1.1 (`keepalive` heartbeats, or none at all
/// when `Duration::ZERO`), disconnects the client mid-stream and asserts the in-flight RPC future
/// is dropped within 250ms.
pub async fn assert_tcp_disconnect_cancels_in_flight_rpc(keepalive: Duration) {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");
    let transport = ControlledTransport::default();
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    transport.hang_with_drop_guard("slow", started_tx, dropped_tx);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
    let addr = listener.local_addr().expect("loopback addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let svc = LoopbackSsrService {
        pool: pool.clone(),
        transport: transport.clone(),
        keepalive,
    };
    let server_task = tokio::spawn(async move {
        serve_service_with_shutdown(listener, svc, async {
            let _ = shutdown_rx.await;
        })
        .await
        .expect("loopback server");
    });

    let mut tcp = TcpStream::connect(addr).await.expect("connect loopback");
    tcp.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .expect("write request");

    // Read until initial shell + seq=0 ack arrives over real HTTP/1.1 chunked TCP stream.
    let mut acc = Vec::new();
    let needle = b"data-nve-q-ack=\"0\"";
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut buf = [0u8; 1024];
        while !acc.windows(needle.len()).any(|w| w == needle) {
            let n = tcp.read(&mut buf).await.expect("tcp read");
            assert!(n > 0, "unexpected EOF before shell arrived");
            acc.extend_from_slice(&buf[..n]);
        }
    })
    .await
    .expect("timed out reading shell over loopback TCP");

    let shell_wire = String::from_utf8_lossy(&acc);
    let chunked = b"transfer-encoding: chunked";
    assert!(
        acc.windows(chunked.len()).any(|w| w.eq_ignore_ascii_case(chunked)),
        "Hyper must chunk-frame streaming SSR over HTTP/1.1: {shell_wire}"
    );

    // Wait until the slow RPC future is definitely in flight before disconnecting.
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("slow RPC must start")
        .expect("started sender");

    // Abruptly close the client TCP socket and assert in-flight RPC drop guard fires in < 250ms!
    let drop_start = Instant::now();
    drop(tcp);

    let dropped_at = tokio::time::timeout(Duration::from_millis(250), dropped_rx)
        .await
        .expect("in-flight RPC future MUST be cancelled within 250ms of TCP disconnect")
        .expect("drop guard must send timestamp");
    let elapsed = dropped_at.saturating_duration_since(drop_start);
    assert!(
        elapsed < Duration::from_millis(250),
        "cancellation took {elapsed:?} (>= 250ms)"
    );

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task");
}
