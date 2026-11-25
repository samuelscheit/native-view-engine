//! Per-connection peer handlers for dual-ALPN QUIC (`nve-wire-2` + `h3` WebTransport) and
//! TLS WebSocket (`wss://` + loopback `ws://`) peers.

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use web_transport_quinn::generic::SendStream as _;

use nve_wire_core::{
    NveWireFrame, NveWireHello, NveWireKind, NveWireRejectReason, NveWireTransportKind,
};

use crate::framing::{read_stream_frame, read_wt_frame, write_stream_raw, write_wt_raw};
use crate::server::{OutboundCommand, SharedServerState};
use crate::tls::{NVE_WIRE_QUIC_ALPN_BYTES, WEBTRANSPORT_ALPN};

fn negotiated_alpn(conn: &quinn::Connection) -> Option<Vec<u8>> {
    let data = conn.handshake_data()?;
    if let Ok(hd) = data.downcast::<web_transport_quinn::crypto::HandshakeData>() {
        return hd.protocol;
    }
    None
}

pub(crate) async fn handle_quic_or_wt_connection(
    conn: quinn::Connection,
    state: Arc<SharedServerState>,
) {
    let alpn = negotiated_alpn(&conn);
    if alpn.as_deref() == Some(WEBTRANSPORT_ALPN) {
        handle_webtransport_connection(conn, state).await;
    } else if alpn.as_deref() == Some(NVE_WIRE_QUIC_ALPN_BYTES) {
        handle_raw_quic_connection(conn, state).await;
    } else {
        conn.close(1u32.into(), b"unsupported ALPN");
    }
}

async fn handle_raw_quic_connection(conn: quinn::Connection, state: Arc<SharedServerState>) {
    let Ok((mut send, mut recv)) = conn.accept_bi().await else {
        return;
    };
    let Ok(Some(first)) = read_stream_frame(&mut recv, state.max_frame_bytes).await else {
        return;
    };
    if first.kind != NveWireKind::Hello {
        let _ = write_stream_raw(
            &mut send,
            &SharedServerState::make_reject(NveWireRejectReason::Unauthorized, "expected Hello"),
        )
        .await;
        return;
    }
    let hello = match NveWireHello::decode(&first.payload) {
        Ok(h) => h,
        Err(_) => {
            let _ = write_stream_raw(
                &mut send,
                &SharedServerState::make_reject(
                    NveWireRejectReason::Unauthorized,
                    "malformed Hello",
                ),
            )
            .await;
            return;
        }
    };
    if let Err((reason, msg)) = state.verify_hello(&hello) {
        let _ = write_stream_raw(&mut send, &SharedServerState::make_reject(reason, msg)).await;
        let _ = send.finish();
        let _ = tokio::time::timeout(std::time::Duration::from_millis(250), send.stopped()).await;
        return;
    }

    let (session_id, mut out_rx) = state.register_peer(NveWireTransportKind::Quic);
    if write_stream_raw(&mut send, &state.make_hello_ack(session_id))
        .await
        .is_err()
    {
        state.unregister_peer(session_id);
        return;
    }
    let Some(reply_tx) = state.peer_sender(session_id) else {
        return;
    };

    let conn_writer = conn.clone();
    let writer_task = tokio::spawn(async move {
        while let Some(cmd) = out_rx.recv().await {
            match cmd {
                OutboundCommand::Stream(bytes) => {
                    if write_stream_raw(&mut send, &bytes).await.is_err() {
                        break;
                    }
                }
                OutboundCommand::Datagram(bytes) => {
                    let _ = conn_writer.send_datagram(bytes);
                }
            }
        }
    });

    let conn_dgram = conn.clone();
    let st_dgram = state.clone();
    let reply_dgram = reply_tx.clone();
    let dgram_task = tokio::spawn(async move {
        while let Ok(raw) = conn_dgram.read_datagram().await {
            if let Ok(frame) = NveWireFrame::decode(&raw, st_dgram.max_frame_bytes)
                && !st_dgram.handle_inbound_frame(
                    session_id,
                    NveWireTransportKind::Quic,
                    frame,
                    &reply_dgram,
                )
            {
                break;
            }
        }
    });

    while let Ok(Some(frame)) = read_stream_frame(&mut recv, state.max_frame_bytes).await {
        if !state.handle_inbound_frame(session_id, NveWireTransportKind::Quic, frame, &reply_tx) {
            break;
        }
    }

    writer_task.abort();
    dgram_task.abort();
    state.unregister_peer(session_id);
}

async fn handle_webtransport_connection(conn: quinn::Connection, state: Arc<SharedServerState>) {
    let Ok(request) = web_transport_quinn::Request::accept(conn).await else {
        return;
    };
    let Ok(session) = request.ok().await else {
        return;
    };
    let Ok((mut send, mut recv)) = session.accept_bi().await else {
        return;
    };
    let Ok(Some(first)) = read_wt_frame(&mut recv, state.max_frame_bytes).await else {
        return;
    };
    if first.kind != NveWireKind::Hello {
        let _ = write_wt_raw(
            &mut send,
            &SharedServerState::make_reject(NveWireRejectReason::Unauthorized, "expected Hello"),
        )
        .await;
        return;
    }
    let hello = match NveWireHello::decode(&first.payload) {
        Ok(h) => h,
        Err(_) => {
            let _ = write_wt_raw(
                &mut send,
                &SharedServerState::make_reject(
                    NveWireRejectReason::Unauthorized,
                    "malformed Hello",
                ),
            )
            .await;
            return;
        }
    };
    if let Err((reason, msg)) = state.verify_hello(&hello) {
        let _ = write_wt_raw(&mut send, &SharedServerState::make_reject(reason, msg)).await;
        let _ = send.finish();
        let _ = tokio::time::timeout(std::time::Duration::from_millis(250), send.closed()).await;
        return;
    }

    let (session_id, mut out_rx) = state.register_peer(NveWireTransportKind::WebTransport);
    if write_wt_raw(&mut send, &state.make_hello_ack(session_id))
        .await
        .is_err()
    {
        state.unregister_peer(session_id);
        return;
    }
    let Some(reply_tx) = state.peer_sender(session_id) else {
        return;
    };

    let sess_writer = session.clone();
    let writer_task = tokio::spawn(async move {
        while let Some(cmd) = out_rx.recv().await {
            match cmd {
                OutboundCommand::Stream(bytes) => {
                    if write_wt_raw(&mut send, &bytes).await.is_err() {
                        break;
                    }
                }
                OutboundCommand::Datagram(bytes) => {
                    let _ = sess_writer.send_datagram(bytes);
                }
            }
        }
    });

    let sess_dgram = session.clone();
    let st_dgram = state.clone();
    let reply_dgram = reply_tx.clone();
    let dgram_task = tokio::spawn(async move {
        while let Ok(raw) = sess_dgram.read_datagram().await {
            if let Ok(frame) = NveWireFrame::decode(&raw, st_dgram.max_frame_bytes)
                && !st_dgram.handle_inbound_frame(
                    session_id,
                    NveWireTransportKind::WebTransport,
                    frame,
                    &reply_dgram,
                )
            {
                break;
            }
        }
    });

    while let Ok(Some(frame)) = read_wt_frame(&mut recv, state.max_frame_bytes).await {
        if !state.handle_inbound_frame(
            session_id,
            NveWireTransportKind::WebTransport,
            frame,
            &reply_tx,
        ) {
            break;
        }
    }

    writer_task.abort();
    dgram_task.abort();
    state.unregister_peer(session_id);
}

pub(crate) async fn handle_tcp_ws_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    tls_acceptor: TlsAcceptor,
    allow_plain_loopback_ws: bool,
    state: Arc<SharedServerState>,
) {
    let _ = stream.set_nodelay(true);
    let mut peek_byte = [0u8; 1];
    let Ok(1) = stream.peek(&mut peek_byte).await else {
        return;
    };

    if peek_byte[0] == 0x16 {
        let Ok(tls_stream) = tls_acceptor.accept(stream).await else {
            return;
        };
        let Ok(ws) = tokio_tungstenite::accept_async(tls_stream).await else {
            return;
        };
        run_ws_peer(ws, NveWireTransportKind::Wss, state).await;
    } else if allow_plain_loopback_ws && peer_addr.ip().is_loopback() {
        let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        run_ws_peer(ws, NveWireTransportKind::Ws, state).await;
    }
}

async fn run_ws_peer<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    transport: NveWireTransportKind,
    state: Arc<SharedServerState>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();
    let Some(Ok(Message::Binary(first_bytes))) = stream.next().await else {
        return;
    };
    let Ok(first) = NveWireFrame::decode(&first_bytes, state.max_frame_bytes) else {
        return;
    };
    if first.kind != NveWireKind::Hello {
        let _ = sink
            .send(Message::Binary(SharedServerState::make_reject(
                NveWireRejectReason::Unauthorized,
                "expected Hello",
            )))
            .await;
        return;
    }
    let hello = match NveWireHello::decode(&first.payload) {
        Ok(h) => h,
        Err(_) => {
            let _ = sink
                .send(Message::Binary(SharedServerState::make_reject(
                    NveWireRejectReason::Unauthorized,
                    "malformed Hello",
                )))
                .await;
            return;
        }
    };
    if let Err((reason, msg)) = state.verify_hello(&hello) {
        let _ = sink
            .send(Message::Binary(SharedServerState::make_reject(reason, msg)))
            .await;
        let _ = sink.close().await;
        return;
    }

    let (session_id, mut out_rx) = state.register_peer(transport);
    if sink
        .send(Message::Binary(state.make_hello_ack(session_id)))
        .await
        .is_err()
    {
        state.unregister_peer(session_id);
        return;
    }
    let Some(reply_tx) = state.peer_sender(session_id) else {
        return;
    };

    let writer_task = tokio::spawn(async move {
        while let Some(cmd) = out_rx.recv().await {
            let bytes = match cmd {
                OutboundCommand::Stream(b) | OutboundCommand::Datagram(b) => b,
            };
            if sink.send(Message::Binary(bytes)).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Binary(raw) => {
                if let Ok(frame) = NveWireFrame::decode(&raw, state.max_frame_bytes)
                    && !state.handle_inbound_frame(session_id, transport, frame, &reply_tx)
                {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    writer_task.abort();
    state.unregister_peer(session_id);
}
