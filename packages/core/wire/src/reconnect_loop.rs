//! Worker event loop, fair session pump, bounded inbound overflow handling, and stop-aware
//! backoff sleep for [`crate::ReconnectingWireClient`].

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TrySendError};
use nve_wire_core::{NveWireKind, NveWireUri};
use tokio::sync::mpsc;

use crate::client::ClientInboundEvent;
use crate::reconnect::{encode_transport, ReconnectConfig, SharedState, WireInboundItem};
use crate::{NveWireClient, NveWireClientConfig, NveWireTransportError};

const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) enum OutboundCmd {
    Events { session_id: u64, bytes: Vec<u8> },
    RequestKeyframe { session_id: u64 },
}

/// Sleeps for `wait` unless `shared.stop` fires first. Returns `true` if stopped.
async fn sleep_or_stop(shared: &SharedState, wait: Duration) -> bool {
    let stop_fut = shared.stop_notify.notified();
    tokio::pin!(stop_fut);
    stop_fut.as_mut().enable();
    if shared.stop.load(Ordering::Acquire) {
        return true;
    }
    tokio::select! {
        biased;
        _ = &mut stop_fut => true,
        _ = tokio::time::sleep(wait) => shared.stop.load(Ordering::Acquire),
    }
}

/// Pushes an item into the bounded inbound queue. On `Full`, drains stale items, emits
/// [`WireInboundItem::Overflowed`], and preserves `SessionStarted` or `SceneKeyframe` items.
/// Returns `false` if the receiver side was disconnected.
fn push_inbound_item(
    tx: &Sender<WireInboundItem>,
    drain_rx: &Receiver<WireInboundItem>,
    item: WireInboundItem,
    session_id: u64,
    shared: &SharedState,
) -> bool {
    match tx.try_send(item) {
        Ok(()) => true,
        Err(TrySendError::Disconnected(_)) => false,
        Err(TrySendError::Full(full_item)) => {
            let mut dropped = 0u64;
            while drain_rx.try_recv().is_ok() {
                dropped += 1;
            }
            let _ = tx.try_send(WireInboundItem::Overflowed { session_id });
            let keep = match &full_item {
                WireInboundItem::SessionStarted { .. } => true,
                WireInboundItem::Frame { frame, .. } => {
                    frame.kind == NveWireKind::SceneKeyframe
                }
                WireInboundItem::Overflowed { .. } => false,
            };
            if keep {
                if tx.try_send(full_item).is_err() {
                    dropped += 1;
                }
            } else {
                dropped += 1;
            }
            shared.dropped_inbound.fetch_add(dropped, Ordering::Relaxed);
            true
        }
    }
}

fn push_inbound_overflow(
    tx: &Sender<WireInboundItem>,
    drain_rx: &Receiver<WireInboundItem>,
    session_id: u64,
    shared: &SharedState,
) -> bool {
    let mut dropped = 0u64;
    while drain_rx.try_recv().is_ok() {
        dropped += 1;
    }
    shared.dropped_inbound.fetch_add(dropped, Ordering::Relaxed);
    !matches!(
        tx.try_send(WireInboundItem::Overflowed { session_id }),
        Err(TrySendError::Disconnected(_))
    )
}

pub(crate) fn activate_session(
    client: &NveWireClient,
    reconnected: bool,
    shared: &SharedState,
    inbound_tx: &Sender<WireInboundItem>,
    inbound_drain: &Receiver<WireInboundItem>,
) -> u64 {
    let sid = shared.next_session_seq.fetch_add(1, Ordering::AcqRel);
    shared.set_last_error(None);
    shared
        .transport
        .store(encode_transport(client.transport_kind()), Ordering::Release);
    shared
        .server_session_id
        .store(client.session_id(), Ordering::Release);
    shared.active_session_id.store(sid, Ordering::Release);
    shared.connected.store(true, Ordering::Release);
    if reconnected {
        shared.reconnect_count.fetch_add(1, Ordering::Release);
        let _ = client.request_keyframe();
    }
    let _ = push_inbound_item(
        inbound_tx,
        inbound_drain,
        WireInboundItem::SessionStarted {
            session_id: sid,
            reconnected,
        },
        sid,
        shared,
    );
    shared.update_metrics_from(client);
    shared.fire_wake();
    sid
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn reconnect_loop(
    mut initial: Option<(NveWireClient, u64)>,
    uri: NveWireUri,
    client_cfg: NveWireClientConfig,
    reconnect_cfg: ReconnectConfig,
    shared: Arc<SharedState>,
    inbound_tx: Sender<WireInboundItem>,
    inbound_drain: Receiver<WireInboundItem>,
    mut outbound_rx: mpsc::Receiver<OutboundCmd>,
) {
    let initial_backoff = reconnect_cfg.initial_backoff.max(Duration::from_millis(10));
    let max_backoff = reconnect_cfg.max_backoff.max(initial_backoff);
    let mut backoff = initial_backoff;
    let mut had_prior_session = initial.is_some();

    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }

        let (client, session_id) = if let Some(pair) = initial.take() {
            pair
        } else {
            let stop_fut = shared.stop_notify.notified();
            tokio::pin!(stop_fut);
            stop_fut.as_mut().enable();
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            let conn_res = tokio::select! {
                biased;
                _ = &mut stop_fut => break,
                res = NveWireClient::connect_uri_async(&uri, &client_cfg) => res,
            };
            match conn_res {
                Ok(c) => {
                    backoff = initial_backoff;
                    let sid = activate_session(
                        &c,
                        had_prior_session,
                        &shared,
                        &inbound_tx,
                        &inbound_drain,
                    );
                    had_prior_session = true;
                    (c, sid)
                }
                Err(NveWireTransportError::Auth(msg)) => {
                    // Server authentication / schema rejection is terminal; do not spin forever.
                    shared.set_last_error(Some(msg));
                    shared.auth_rejections.fetch_add(1, Ordering::Release);
                    shared.active_session_id.store(0, Ordering::Release);
                    shared.connected.store(false, Ordering::Release);
                    shared.fire_wake();
                    break;
                }
                Err(err) => {
                    shared.set_last_error(Some(err.to_string()));
                    shared.active_session_id.store(0, Ordering::Release);
                    shared.connected.store(false, Ordering::Release);
                    let wait = backoff;
                    backoff = backoff.saturating_mul(2).min(max_backoff);
                    if sleep_or_stop(&shared, wait).await {
                        break;
                    }
                    continue;
                }
            }
        };

        let stopped = drive_session(
            &client,
            session_id,
            &shared,
            &inbound_tx,
            &inbound_drain,
            &mut outbound_rx,
        )
        .await;

        shared.update_metrics_from(&client);
        shared.checkpoint_metrics();
        shared.active_session_id.store(0, Ordering::Release);
        shared.connected.store(false, Ordering::Release);
        client.close_graceful(GRACEFUL_CLOSE_TIMEOUT).await;
        drain_stale_outbound(&mut outbound_rx, &shared);

        if stopped || shared.stop.load(Ordering::Acquire) {
            break;
        }
        shared.fire_wake();

        let wait = backoff;
        backoff = backoff.saturating_mul(2).min(max_backoff);
        if sleep_or_stop(&shared, wait).await {
            break;
        }
    }

    shared.active_session_id.store(0, Ordering::Release);
    shared.connected.store(false, Ordering::Release);
}

/// Drives one active session fairly across inbound frames, outbound events, and shutdown.
/// Returns `true` when stopped by shutdown/receiver drop (as opposed to transport disconnect).
async fn drive_session(
    client: &NveWireClient,
    session_id: u64,
    shared: &SharedState,
    inbound_tx: &Sender<WireInboundItem>,
    inbound_drain: &Receiver<WireInboundItem>,
    outbound_rx: &mut mpsc::Receiver<OutboundCmd>,
) -> bool {
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return true;
        }
        let stop_fut = shared.stop_notify.notified();
        tokio::pin!(stop_fut);
        stop_fut.as_mut().enable();
        if shared.stop.load(Ordering::Acquire) {
            return true;
        }
        // Fair (non-biased) select between inbound and outbound so sustained outbound traffic
        // cannot starve inbound frames or disconnect detection; stop is checked every iteration.
        tokio::select! {
            _ = &mut stop_fut => return true,
            cmd = outbound_rx.recv() => {
                match cmd {
                    Some(OutboundCmd::Events { session_id: cmd_sid, bytes }) => {
                        if cmd_sid != session_id {
                            shared.dropped_outbound.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        if client.send_events(bytes).is_err() {
                            return false;
                        }
                        shared.update_metrics_from(client);
                    }
                    Some(OutboundCmd::RequestKeyframe { session_id: cmd_sid }) => {
                        if cmd_sid != session_id {
                            continue;
                        }
                        if client.request_keyframe().is_err() {
                            return false;
                        }
                        shared.update_metrics_from(client);
                    }
                    None => return true,
                }
            }
            ev = client.next_inbound_event() => {
                match ev {
                    Some(ClientInboundEvent::Frame(frame)) => {
                        shared.update_metrics_from(client);
                        let item = WireInboundItem::Frame { session_id, frame };
                        if !push_inbound_item(inbound_tx, inbound_drain, item, session_id, shared) {
                            return true;
                        }
                        shared.fire_wake();
                    }
                    Some(ClientInboundEvent::Overflowed) => {
                        if !push_inbound_overflow(inbound_tx, inbound_drain, session_id, shared) {
                            return true;
                        }
                        shared.fire_wake();
                    }
                    None => return false,
                }
            }
        }
    }
}

fn drain_stale_outbound(outbound_rx: &mut mpsc::Receiver<OutboundCmd>, shared: &SharedState) {
    while let Ok(cmd) = outbound_rx.try_recv() {
        if matches!(cmd, OutboundCmd::Events { .. }) {
            shared.dropped_outbound.fetch_add(1, Ordering::Relaxed);
        }
    }
}
