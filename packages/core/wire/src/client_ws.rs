//! TLS WebSocket (`wss://` / `ws://`) and URI-based dispatch helpers for [`NveWireClient`].

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;
use crossbeam_channel::unbounded;
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;

use nve_wire_core::{NveWireFrame, NveWireTransportKind, NveWireUri};

use crate::client::{decode_hello_response, encode_hello, ClientOut, ClientStats, NveWireClient};
use crate::tls::build_pinned_client_tls_config;
use crate::{NveWireClientConfig, NveWireTransportError};

impl NveWireClient {
    /// Connects over TLS WebSocket (`wss://`) with SHA-256 cert pinning, or local `ws://`.
    pub async fn connect_ws(
        url_str: &str,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let parsed = url::Url::parse(url_str)
            .map_err(|e| NveWireTransportError::Connect(format!("invalid WS url: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| NveWireTransportError::Connect("WS url missing host".to_owned()))?;
        let is_tls = parsed.scheme() == "wss";
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| NveWireTransportError::Connect("WS url missing port".to_owned()))?;
        let tcp = tokio::time::timeout(config.connect_timeout, TcpStream::connect((host, port)))
            .await
            .map_err(|_| NveWireTransportError::Connect("TCP connect timed out".to_owned()))?
            .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;
        let _ = tcp.set_nodelay(true);

        if is_tls {
            let cert_hash = config.cert_sha256.ok_or_else(|| {
                NveWireTransportError::Tls("WSS connect requires pinned cert_sha256".to_owned())
            })?;
            let tls_cfg = build_pinned_client_tls_config(cert_hash, vec![b"http/1.1".to_vec()])?;
            let connector = TlsConnector::from(Arc::new(tls_cfg));
            let sni = ServerName::try_from(config.server_name.clone())
                .map_err(|e| NveWireTransportError::Tls(e.to_string()))?;
            let tls_stream = connector
                .connect(sni, tcp)
                .await
                .map_err(|e| NveWireTransportError::Tls(e.to_string()))?;
            let (ws, _) = tokio_tungstenite::client_async(url_str, tls_stream)
                .await
                .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;
            Self::finish_ws_handshake(ws, NveWireTransportKind::Wss, config).await
        } else {
            let (ws, _) = tokio_tungstenite::client_async(url_str, tcp)
                .await
                .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;
            Self::finish_ws_handshake(ws, NveWireTransportKind::Ws, config).await
        }
    }

    async fn finish_ws_handshake<S>(
        ws: tokio_tungstenite::WebSocketStream<S>,
        transport_kind: NveWireTransportKind,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sink, mut stream) = ws.split();
        let hello_bytes = encode_hello(config)?;
        sink.send(Message::Binary(Bytes::from(hello_bytes)))
            .await
            .map_err(|e| NveWireTransportError::Io(e.to_string()))?;

        let ack = match stream.next().await {
            Some(Ok(Message::Binary(raw))) => {
                let frame = NveWireFrame::decode(&raw, config.max_frame_bytes)?;
                decode_hello_response(frame)?
            }
            Some(Ok(_)) | None => {
                return Err(NveWireTransportError::Auth(
                    "WS peer closed before HelloAck".to_owned(),
                ))
            }
            Some(Err(e)) => return Err(NveWireTransportError::Io(e.to_string())),
        };

        let (inbound_tx, inbound_rx) = unbounded();
        let inbound_notify = Arc::new(tokio::sync::Notify::new());
        let (outbound_tx, mut out_rx) = mpsc::unbounded_channel();
        let stats = Arc::new(ClientStats::new());

        let stats_w = stats.clone();
        tokio::spawn(async move {
            while let Some(cmd) = out_rx.recv().await {
                match cmd {
                    ClientOut::Stream(bytes) | ClientOut::Datagram(bytes) => {
                        if sink.send(Message::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                    ClientOut::Close => {
                        let _ = sink.close().await;
                        break;
                    }
                }
            }
            stats_w.connected.store(false, Ordering::Relaxed);
        });

        let max_len = config.max_frame_bytes;
        let stats_r = stats.clone();
        let notify_r = inbound_notify.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                match msg {
                    Message::Binary(raw) => {
                        if let Ok(frame) = NveWireFrame::decode(&raw, max_len) {
                            stats_r.record_inbound(&frame);
                            let _ = inbound_tx.send(frame);
                            notify_r.notify_waiters();
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            stats_r.connected.store(false, Ordering::Relaxed);
        });

        Ok(Self::from_parts(
            ack.session_id,
            transport_kind,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            stats,
        ))
    }

    /// Synchronous blocking helper that creates an internal Tokio runtime and connects via `uri`.
    pub fn connect_uri_sync(
        uri: &NveWireUri,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| NveWireTransportError::Io(e.to_string()))?,
        );
        let mut client = rt.block_on(Self::connect_uri_async(uri, config))?;
        client.set_runtime(rt);
        Ok(client)
    }

    /// Connects according to `uri.transport` (and available QUIC/WSS ports).
    pub async fn connect_uri_async(
        uri: &NveWireUri,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let mut cfg = config.clone();
        if cfg.bearer_token.is_empty() && !uri.token.is_empty() {
            cfg.bearer_token = uri.token.clone();
        }
        if cfg.cert_sha256.is_none() {
            cfg.cert_sha256 = uri.cert_sha256;
        }
        let host = &uri.host;
        let path = if uri.path.starts_with('/') {
            uri.path.as_str()
        } else {
            "/nve-wire"
        };
        let resolve_addr = |port: u16| -> Result<SocketAddr, NveWireTransportError> {
            use std::net::ToSocketAddrs;
            (host.as_str(), port)
                .to_socket_addrs()
                .map_err(|e| NveWireTransportError::Connect(e.to_string()))?
                .next()
                .ok_or_else(|| NveWireTransportError::Connect("could not resolve host".to_owned()))
        };

        match uri.transport {
            NveWireTransportKind::Quic => {
                let port = uri.quic_port.or(uri.wss_port).ok_or_else(|| {
                    NveWireTransportError::Connect("missing QUIC port".to_owned())
                })?;
                Self::connect_quic(resolve_addr(port)?, &cfg).await
            }
            NveWireTransportKind::WebTransport => {
                let port = uri.quic_port.or(uri.wss_port).ok_or_else(|| {
                    NveWireTransportError::Connect("missing WebTransport port".to_owned())
                })?;
                let url = format!("https://{host}:{port}{path}");
                Self::connect_webtransport(&url, &cfg).await
            }
            NveWireTransportKind::Wss => {
                let port = uri.wss_port.or(uri.quic_port).ok_or_else(|| {
                    NveWireTransportError::Connect("missing WSS port".to_owned())
                })?;
                let url = format!("wss://{host}:{port}{path}");
                Self::connect_ws(&url, &cfg).await
            }
            NveWireTransportKind::Ws => {
                let port = uri.wss_port.or(uri.quic_port).ok_or_else(|| {
                    NveWireTransportError::Connect("missing WS port".to_owned())
                })?;
                let url = format!("ws://{host}:{port}{path}");
                Self::connect_ws(&url, &cfg).await
            }
            NveWireTransportKind::Auto => match (uri.quic_port, uri.wss_port) {
                (Some(qp), Some(wp)) => {
                    let scheme = if cfg.cert_sha256.is_some() { "wss" } else { "ws" };
                    let ws_url = format!("{scheme}://{host}:{wp}{path}");
                    if cfg.cert_sha256.is_some() {
                        Self::connect_happy_eyeballs(resolve_addr(qp)?, &ws_url, &cfg).await
                    } else {
                        Self::connect_ws(&ws_url, &cfg).await
                    }
                }
                (Some(qp), None) => Self::connect_quic(resolve_addr(qp)?, &cfg).await,
                (None, Some(wp)) => {
                    let scheme = if cfg.cert_sha256.is_some() { "wss" } else { "ws" };
                    let ws_url = format!("{scheme}://{host}:{wp}{path}");
                    Self::connect_ws(&ws_url, &cfg).await
                }
                (None, None) => Err(NveWireTransportError::Connect(
                    "URI specifies neither QUIC nor WSS port".to_owned(),
                )),
            },
        }
    }
}
