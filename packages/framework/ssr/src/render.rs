//! Core pre-headers shell pass, adaptive Light-DOM fast path, and cancellation-safe out-of-order
//! Declarative Shadow DOM streaming loop executed on a `LocalSet` worker.

use core::pin::Pin;
use core::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use http::{Response, StatusCode};
use nve_backend_web::{
    push_async_boot_script, push_modulepreload_link, push_query_payload_tag,
    push_stylesheet_links, streaming_http_headers, PendingStreamSlot, SsrDocumentOptions,
    StreamSlotState, StreamingDocumentOptions, StreamingShellOutput, StreamingSsrSession,
    WebBackend, NVE_STREAM_KEEPALIVE_COMMENT, PRE_HYDRATION_REPLAY_SCRIPT_TAG,
};
use nve_dioxus::DioxusSurface;
use nve_http_hyper::HttpBody;
use nve_query::{nveq_entry_count, DehydrateCursor};
use nve_query_dioxus::{ErasedQueryClient, SsrBoundaryRegistry};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit};

use crate::options::{SsrPageOptions, StreamPlan, FAST_PATH_HTTP_HEADERS};
use crate::SsrError;

const FINAL_TRAILER_GRACE: Duration = Duration::from_millis(50);
const MAX_SSR_SYNC_FOLLOWUP_PASSES: usize = 16;

/// Drains immediately ready synchronous follow-ups (e.g., a cached/in-memory waterfall step
/// resolving on first poll before re-suspending on an async step, or a `SuspenseBoundary`
/// mounting its fallback) strictly within `nve-ssr` without leaking `wait_for_work()` polling into
/// `DioxusSurface::flush_into` on interactive hosts.
fn flush_ssr_until_quiescent(
    surface: &mut DioxusSurface,
    backend: &mut WebBackend,
    frame_id: &mut u64,
) -> Result<(), SsrError> {
    surface
        .flush_into(*frame_id, backend)
        .map_err(|err| SsrError::Render(err.to_string()))?;
    let noop = core::task::Waker::noop();
    for _ in 0..MAX_SSR_SYNC_FOLLOWUP_PASSES {
        if !surface.poll_work(noop) {
            break;
        }
        *frame_id = frame_id.saturating_add(1);
        surface
            .flush_into(*frame_id, backend)
            .map_err(|err| SsrError::Render(err.to_string()))?;
    }
    Ok(())
}

fn build_http_response(
    status: StatusCode,
    headers: &[(&str, &str)],
    body: HttpBody,
) -> Result<Response<HttpBody>, SsrError> {
    let mut builder = Response::builder().status(status);
    for &(name, value) in headers {
        builder = builder.header(name, value);
    }
    builder
        .body(body)
        .map_err(|err| SsrError::Http(err.to_string()))
}

fn render_fast_path_response(
    page: &SsrPageOptions,
    backend: &WebBackend,
    client: &ErasedQueryClient,
) -> Result<Response<HttpBody>, SsrError> {
    let dehydrated = client.dehydrate_bytes()?;

    let mut head_extra = String::new();
    push_stylesheet_links(&page.stylesheet_links, &mut head_extra);
    if let Some(extra) = page.head_extra.as_deref().filter(|s| !s.is_empty()) {
        head_extra.push_str(extra);
        if !extra.ends_with('\n') {
            head_extra.push('\n');
        }
    }
    push_modulepreload_link(page.modulepreload_src.as_deref(), &mut head_extra);

    let mut body_scripts = String::with_capacity(dehydrated.len().saturating_mul(2) + 160);
    if nveq_entry_count(&dehydrated).unwrap_or(0) > 0 {
        let b64 = BASE64_STANDARD.encode(&dehydrated);
        push_query_payload_tag(0, None, &b64, &mut body_scripts);
    }
    body_scripts.push_str("<template data-nve-stream-end></template>\n");
    push_async_boot_script(page.async_boot_src.as_deref(), &mut body_scripts);
    if let Some(extra) = page.body_scripts.as_deref().filter(|s| !s.is_empty()) {
        body_scripts.push_str(extra);
        if !extra.ends_with('\n') {
            body_scripts.push('\n');
        }
    }

    let doc_opts = SsrDocumentOptions {
        title: page.title.clone(),
        description: page.description.clone(),
        lang: page.lang.clone(),
        head_extra: (!head_extra.is_empty()).then_some(head_extra),
        body_scripts: Some(body_scripts),
        enable_event_replay: page.enable_event_replay,
        style_nonce: page.style_nonce.clone(),
        ..SsrDocumentOptions::default()
    };

    let html = backend.render_ssr_document(&doc_opts);
    build_http_response(
        page.status,
        &FAST_PATH_HTTP_HEADERS,
        HttpBody::full(Bytes::from(html)),
    )
}

fn prepare_streaming_session(
    page: &SsrPageOptions,
    shell_out: &StreamingShellOutput,
    registry: &SsrBoundaryRegistry,
    client: &ErasedQueryClient,
) -> Result<(StreamingSsrSession, DehydrateCursor, Bytes), SsrError> {
    let mut doc_opts = StreamingDocumentOptions::from_streaming_shell(&page.title, shell_out);
    doc_opts.lang = page.lang.clone();
    doc_opts.description = page.description.clone();
    doc_opts.stylesheet_links = page.stylesheet_links.clone();
    doc_opts.head_extra = page.head_extra.clone();
    doc_opts.body_scripts = page.body_scripts.clone();
    doc_opts.style_nonce = page.style_nonce.clone();

    let mut head_scripts = String::new();
    push_modulepreload_link(page.modulepreload_src.as_deref(), &mut head_scripts);

    let expected_slots: Vec<String> = shell_out
        .pending_slots
        .iter()
        .map(|s| s.slot_id.clone())
        .collect();
    let mut session = StreamingSsrSession::new(doc_opts, expected_slots);

    let mut initial = session.begin_shell_with_head_scripts(&shell_out.ssr.html, &head_scripts)?;
    // The replay snippet resolves `document.getElementById('app')` synchronously, so it must run
    // only once the parser has opened `<div id="app">` (never from `<head>`).
    if page.enable_event_replay {
        initial.push_str(PRE_HYDRATION_REPLAY_SCRIPT_TAG);
        initial.push('\n');
    }

    // Always emit `seq=0` + `data-nve-q-ack="0"` on the streaming path so `async_boot_src` is
    // strictly ordered after `ack=0` for deterministic mid-stream WASM hydration.
    let mut cursor = DehydrateCursor::new();
    let seq0 = client.dehydrate_initial_stream_chunk(&registry.pending_handles(), &mut cursor)?;
    let seq0_b64 = BASE64_STANDARD.encode(&seq0);
    initial.push_str(&session.emit_query_payload(0, &seq0_b64)?);

    if let Some(src) = page.async_boot_src.as_deref() {
        initial.push_str(&session.emit_async_boot_script(src)?);
    }

    Ok((session, cursor, Bytes::from(initial)))
}

async fn send_chunk_until_deadline(
    tx: &mpsc::Sender<Bytes>,
    deadline: &mut Pin<&mut tokio::time::Sleep>,
    chunk: Bytes,
) -> bool {
    tokio::select! {
        biased;
        res = tx.send(chunk) => res.is_ok(),
        _ = &mut *deadline => false,
    }
}

/// Runs one render on its `LocalSet` worker, holding `_render_slot` (the worker's pool capacity
/// permit) until the response completes, hits its deadline, or the client disconnects.
pub(crate) async fn execute_render_streaming<F>(
    page: SsrPageOptions,
    plan: StreamPlan,
    client: ErasedQueryClient,
    build_surface: F,
    headers_tx: oneshot::Sender<Result<Response<HttpBody>, SsrError>>,
    _render_slot: OwnedSemaphorePermit,
) where
    F: FnOnce(ErasedQueryClient) -> DioxusSurface + Send + 'static,
{
    let registry = SsrBoundaryRegistry::new();
    let mut surface = build_surface(client.clone()).with_context(registry.clone());
    let mut backend = WebBackend::new();
    let mut frame_id: u64 = 1;

    if let Err(err) = flush_ssr_until_quiescent(&mut surface, &mut backend, &mut frame_id) {
        let _ = headers_tx.send(Err(err));
        return;
    }

    let mut shell_out = match backend.render_ssr_streaming_shell() {
        Ok(out) => out,
        Err(err) => {
            let _ = headers_tx.send(Err(SsrError::Streaming(err)));
            return;
        }
    };

    // Adaptive Fast Path: zero pending streaming slots and zero unresolved suspense boundaries ->
    // plain Light-DOM `HttpBody::Full` response.
    let has_unresolved_boundary = registry.boundaries().iter().any(|e| e.is_pending());
    if shell_out.pending_slots.is_empty() && !has_unresolved_boundary {
        let resp = render_fast_path_response(&page, &backend, &client);
        let _ = headers_tx.send(resp);
        return;
    }

    // Streaming Path: build initial DSD shell + `seq=0` + optional async boot script pre-headers.
    let (mut session, mut cursor, initial_bytes) =
        match prepare_streaming_session(&page, &shell_out, &registry, &client) {
            Ok(parts) => parts,
            Err(err) => {
                let _ = headers_tx.send(Err(err));
                return;
            }
        };

    let (tx, rx) = mpsc::channel::<Bytes>(plan.channel_capacity.max(1));
    if tx.try_send(initial_bytes).is_err() {
        let _ = headers_tx.send(Err(SsrError::PoolClosed));
        return;
    }

    let stream_headers = streaming_http_headers();
    let resp = match build_http_response(
        page.status,
        &stream_headers,
        HttpBody::stream(rx),
    ) {
        Ok(r) => r,
        Err(err) => {
            let _ = headers_tx.send(Err(err));
            return;
        }
    };

    // If caller dropped the `render_streaming` future before headers arrived, abort immediately.
    if headers_tx.send(Ok(resp)).is_err() {
        return;
    }

    let mut pending_slots: Vec<PendingStreamSlot> = core::mem::take(&mut shell_out.pending_slots);
    let mut next_seq: u64 = 1;

    let deadline = tokio::time::sleep(plan.total_deadline);
    tokio::pin!(deadline);

    let mut keepalive = (!plan.keepalive_interval.is_zero()).then(|| {
        let mut iv = tokio::time::interval_at(
            tokio::time::Instant::now() + plan.keepalive_interval,
            plan.keepalive_interval,
        );
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        iv
    });

    while !pending_slots.is_empty() {
        tokio::select! {
            biased;
            _ = tx.closed() => return,
            _ = &mut deadline => break,
            _ = async {
                match keepalive.as_mut() {
                    Some(iv) => {
                        iv.tick().await;
                    }
                    None => core::future::pending::<()>().await,
                }
            } => {
                if !send_chunk_until_deadline(
                    &tx,
                    &mut deadline,
                    Bytes::from_static(NVE_STREAM_KEEPALIVE_COMMENT.as_bytes()),
                )
                .await
                {
                    if tx.is_closed() {
                        return;
                    }
                    break;
                }
            }
            _ = surface.wait_for_work() => {
                frame_id = frame_id.saturating_add(1);
                if flush_ssr_until_quiescent(&mut surface, &mut backend, &mut frame_id).is_err() {
                    break;
                }

                let mut still_pending = Vec::with_capacity(pending_slots.len());
                let mut stream_aborted = false;
                let mut should_break = false;

                for slot in pending_slots.drain(..) {
                    if stream_aborted || should_break {
                        still_pending.push(slot);
                        continue;
                    }
                    match backend.stream_slot_state(slot.parent_node_id, &slot.slot_id) {
                        // The suspense parent unmounted mid-stream: nothing left to flush, so
                        // stop waiting on it (the client renders whatever replaced it).
                        StreamSlotState::Unmounted => continue,
                        StreamSlotState::Pending => {
                            still_pending.push(slot);
                            continue;
                        }
                        StreamSlotState::Resolved => {}
                    }
                    let reg_ready = registry
                        .entry(&slot.slot_id)
                        .is_none_or(|entry| entry.is_ready_to_flush());
                    if !reg_ready {
                        still_pending.push(slot);
                        continue;
                    }

                    registry.mark_flushed(&slot.slot_id);
                    let handles = registry
                        .entry(&slot.slot_id)
                        .map(|entry| entry.query_handles())
                        .unwrap_or_default();
                    let delta_nveq = match client.dehydrate_handles_delta(&handles, &mut cursor) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            should_break = true;
                            continue;
                        }
                    };
                    let delta_dom = backend.render_ssr_subtree_delta(
                        slot.parent_node_id,
                        &mut shell_out.style_tracker,
                    );

                    // One atomic chunk: slot-bound `NVEQ` strictly BEFORE `<div slot>` + ack.
                    let b64 = (nveq_entry_count(&delta_nveq).unwrap_or(0) > 0)
                        .then(|| BASE64_STANDARD.encode(&delta_nveq));
                    let payload = b64.as_deref().map(|b64| (next_seq, b64));
                    let chunk = match session.resolve_slot_delta(&slot.slot_id, &delta_dom, payload)
                    {
                        Ok(chunk) => chunk,
                        Err(_) => {
                            should_break = true;
                            continue;
                        }
                    };
                    if payload.is_some() {
                        next_seq = next_seq.saturating_add(1);
                    }

                    if !send_chunk_until_deadline(&tx, &mut deadline, Bytes::from(chunk)).await {
                        if tx.is_closed() {
                            stream_aborted = true;
                        } else {
                            should_break = true;
                        }
                    }
                }

                if stream_aborted {
                    return;
                }
                pending_slots = still_pending;
                if should_break {
                    break;
                }
            }
        }
    }

    if tx.is_closed() {
        return;
    }

    if let Ok(trailer) = session.finish() {
        // Always grant the trailer at least `FINAL_TRAILER_GRACE`, measured once (no
        // check-then-await race against a deadline that elapses in between).
        let wait = deadline
            .deadline()
            .saturating_duration_since(tokio::time::Instant::now())
            .max(FINAL_TRAILER_GRACE);
        let _ = tokio::time::timeout(wait, tx.send(Bytes::from(trailer))).await;
    }
}
