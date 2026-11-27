//! TLS WebSocket (`wss://` / `ws://`) and URI-based dispatch helpers for [`NveWireClient`].

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use crossbeam_channel::bounded;
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;

use nve_wire_core::{NveWireFrame, NveWireTransportKind, NveWireUri};

use crate::client::{
    decode_hello_response, encode_hello, push_inbound, ClientOut, ClientStats, NveWireClient,
    CLIENT_INBOUND_CAP, CLIENT_OUTBOUND_CAP,
};
use crate::framing::{read_wt_frame, write_wt_raw};
use crate::tls::{
    build_pinned_client_tls_config, build_quic_transport_config, ensure_crypto_provider,
};
use crate::{NveWireClientConfig, NveWireTransportError};

async fn resolve_socket_addr(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<SocketAddr, NveWireTransportError> {
    let mut addrs = tokio::time::timeout(timeout, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| NveWireTransportError::Connect("DNS lookup timed out".to_owned()))?
        .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;
    addrs
        .next()
        .ok_or_else(|| NveWireTransportError::Connect("could not resolve host".to_owned()))
}

impl NveWireClient {
    /// Connects over real HTTP/3 WebTransport (`h3` ALPN) with 13-day ECDSA SHA-256 cert pinning.
    pub async fn connect_webtransport(
        url_str: &str,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let _ = ensure_crypto_provider();
        let cert_hash = config.cert_sha256.ok_or_else(|| {
            NveWireTransportError::Tls("WebTransport connect requires pinned cert_sha256".to_owned())
        })?;
        let parsed_url = url::Url::parse(url_str)
            .map_err(|e| NveWireTransportError::Connect(format!("invalid WT url: {e}")))?;

        let mut transport = quinn::TransportConfig::default();
        transport.datagram_receive_buffer_size(Some(65_536));
        transport.datagram_send_buffer_size(65_536);
        let _ = build_quic_transport_config();

        let client = web_transport_quinn::ClientBuilder::new()
            .with_transport_config(transport)
            .with_server_certificate_hashes(vec![cert_hash.to_vec()])
            .map_err(|e| NveWireTransportError::Tls(format!("WT client builder failed: {e}")))?;

        let handshake_fut = async {
            let session = client.connect(parsed_url).await.map_err(|e| {
                NveWireTransportError::Connect(format!("WebTransport connect failed: {e}"))
            })?;
            let (mut send, mut recv) = session
                .open_bi()
                .await
                .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;
            let hello_bytes = encode_hello(config)?;
            write_wt_raw(&mut send, &hello_bytes).await?;
            let ack = match read_wt_frame(&mut recv, config.max_frame_bytes).await? {
                Some(f) => decode_hello_response(f)?,
                None => {
                    return Err(NveWireTransportError::Auth(
                        "server closed WebTransport stream before HelloAck".to_owned(),
                    ))
                }
            };
            Ok((session, send, recv, ack))
        };
        let (session, mut send, mut recv, ack) =
            tokio::time::timeout(config.connect_timeout, handshake_fut)
                .await
                .map_err(|_| {
                    NveWireTransportError::Connect("WebTransport handshake timed out".to_owned())
                })??;

        let (inbound_tx, inbound_rx) = bounded(CLIENT_INBOUND_CAP);
        let inbound_notify = Arc::new(tokio::sync::Notify::new());
        let writer_done = Arc::new(tokio::sync::Notify::new());
        let writer_exited = Arc::new(AtomicBool::new(false));
        let (outbound_tx, mut out_rx) = mpsc::channel(CLIENT_OUTBOUND_CAP);
        let stats = Arc::new(ClientStats::new());

        let sess_write = session.clone();
        let stats_w = stats.clone();
        let notify_w = inbound_notify.clone();
        let done_w = writer_done.clone();
        let exited_w = writer_exited.clone();
        let writer_jh = tokio::spawn(async move {
            while let Some(cmd) = out_rx.recv().await {
                match cmd {
                    ClientOut::Stream(bytes) => {
                        if write_wt_raw(&mut send, &bytes).await.is_err() {
                            break;
                        }
                    }
                    ClientOut::Datagram(bytes) => {
                        let _ = sess_write.send_datagram(bytes);
                    }
                    ClientOut::Close => {
                        sess_write.close(0, b"bye");
                        break;
                    }
                }
            }
            stats_w.connected.store(false, Ordering::Release);
            exited_w.store(true, Ordering::Release);
            done_w.notify_waiters();
            notify_w.notify_waiters();
        });

        let sess_dgram = session.clone();
        let max_len = config.max_frame_bytes;
        let in_tx_d = inbound_tx.clone();
        let drain_d = inbound_rx.clone();
        let notify_d = inbound_notify.clone();
        let stats_d = stats.clone();
        let dgram_jh = tokio::spawn(async move {
            while let Ok(raw) = sess_dgram.read_datagram().await {
                if let Ok(frame) = NveWireFrame::decode(&raw, max_len) {
                    push_inbound(&in_tx_d, &drain_d, frame, &stats_d, &notify_d);
                }
            }
        });

        let drain_r = inbound_rx.clone();
        let stats_r = stats.clone();
        let notify_r = inbound_notify.clone();
        let reader_jh = tokio::spawn(async move {
            while let Ok(Some(frame)) = read_wt_frame(&mut recv, max_len).await {
                push_inbound(&inbound_tx, &drain_r, frame, &stats_r, &notify_r);
            }
            stats_r.connected.store(false, Ordering::Release);
            notify_r.notify_waiters();
        });

        Ok(Self::from_parts(
            ack.session_id,
            NveWireTransportKind::WebTransport,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            writer_done,
            writer_exited,
            stats,
            vec![
                writer_jh.abort_handle(),
                dgram_jh.abort_handle(),
                reader_jh.abort_handle(),
            ],
        ))
    }

    /// Connects over TLS WebSocket (`wss://`) with SHA-256 cert pinning, or local `ws://`.
    ///
    /// Wraps the entire TCP dial, TLS handshake, WebSocket upgrade, and `Hello`/`HelloAck`
    /// exchange inside `config.connect_timeout`.
    pub async fn connect_ws(
        url_str: &str,
        config: &NveWireClientConfig,
    ) -> Result<Self, NveWireTransportError> {
        let parsed = url::Url::parse(url_str)
            .map_err(|e| NveWireTransportError::Connect(format!("invalid WS url: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| NveWireTransportError::Connect("WS url missing host".to_owned()))?
            .to_owned();
        let is_tls = parsed.scheme() == "wss";
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| NveWireTransportError::Connect("WS url missing port".to_owned()))?;

        let handshake_fut = async move {
            let tcp = TcpStream::connect((host.as_str(), port))
                .await
                .map_err(|e| NveWireTransportError::Connect(e.to_string()))?;
            let _ = tcp.set_nodelay(true);

            if is_tls {
                let cert_hash = config.cert_sha256.ok_or_else(|| {
                    NveWireTransportError::Tls("WSS connect requires pinned cert_sha256".to_owned())
                })?;
                let tls_cfg =
                    build_pinned_client_tls_config(cert_hash, vec![b"http/1.1".to_vec()])?;
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
        };

        tokio::time::timeout(config.connect_timeout, handshake_fut)
            .await
            .map_err(|_| NveWireTransportError::Connect("WS handshake timed out".to_owned()))?
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

        let (inbound_tx, inbound_rx) = bounded(CLIENT_INBOUND_CAP);
        let inbound_notify = Arc::new(tokio::sync::Notify::new());
        let writer_done = Arc::new(tokio::sync::Notify::new());
        let writer_exited = Arc::new(AtomicBool::new(false));
        let (outbound_tx, mut out_rx) = mpsc::channel(CLIENT_OUTBOUND_CAP);
        let stats = Arc::new(ClientStats::new());

        let stats_w = stats.clone();
        let notify_w = inbound_notify.clone();
        let done_w = writer_done.clone();
        let exited_w = writer_exited.clone();
        let writer_jh = tokio::spawn(async move {
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
            stats_w.connected.store(false, Ordering::Release);
            exited_w.store(true, Ordering::Release);
            done_w.notify_waiters();
            notify_w.notify_waiters();
        });

        let max_len = config.max_frame_bytes;
        let drain_r = inbound_rx.clone();
        let stats_r = stats.clone();
        let notify_r = inbound_notify.clone();
        let reader_jh = tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                match msg {
                    Message::Binary(raw) => {
                        if let Ok(frame) = NveWireFrame::decode(&raw, max_len) {
                            push_inbound(&inbound_tx, &drain_r, frame, &stats_r, &notify_r);
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            stats_r.connected.store(false, Ordering::Release);
            notify_r.notify_waiters();
        });

        Ok(Self::from_parts(
            ack.session_id,
            transport_kind,
            outbound_tx,
            inbound_rx,
            inbound_notify,
            writer_done,
            writer_exited,
            stats,
            vec![writer_jh.abort_handle(), reader_jh.abort_handle()],
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

    /// Connects according to `uri.transport` (and available QUIC/WSS ports) using async DNS
    /// resolution bounded by `config.connect_timeout`.
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
        let timeout = cfg.connect_timeout;

        let connect_fut = async {
            match uri.transport {
                NveWireTransportKind::Quic => {
                    let port = uri.quic_port.or(uri.wss_port).ok_or_else(|| {
                        NveWireTransportError::Connect("missing QUIC port".to_owned())
                    })?;
                    let addr = resolve_socket_addr(host, port, timeout).await?;
                    Self::connect_quic(addr, &cfg).await
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
                            let addr = resolve_socket_addr(host, qp, timeout).await?;
                            Self::connect_happy_eyeballs(addr, &ws_url, &cfg).await
                        } else {
                            Self::connect_ws(&ws_url, &cfg).await
                        }
                    }
                    (Some(qp), None) => {
                        let addr = resolve_socket_addr(host, qp, timeout).await?;
                        Self::connect_quic(addr, &cfg).await
                    }
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
        };

        tokio::time::timeout(timeout, connect_fut)
            .await
            .map_err(|_| NveWireTransportError::Connect("wire connect timed out".to_owned()))?
    }
}
