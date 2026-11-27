#![cfg(feature = "server")]
//! Unit and real loopback HTTP/1.1 TCP streaming tests for [`ChannelBody`] and [`HttpBody`].

use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::time::Duration;
use std::sync::Arc;

use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Body, Incoming};
use nve_http_hyper::{serve_service_with_config, ChannelBody, HttpBody, ServeConfig};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

#[tokio::test]
async fn channel_body_and_http_body_unit_semantics() {
    // 1. `HttpBody::Full` preserves exact size_hint & zero-overhead `From` conversions.
    let full = HttpBody::full(Bytes::from_static(b"hello full"));
    assert_eq!(full.size_hint().exact(), Some(10));
    assert!(!full.is_end_stream());
    let collected = full.collect().await.unwrap().to_bytes();
    assert_eq!(collected, Bytes::from_static(b"hello full"));

    let empty = HttpBody::from(Full::new(Bytes::new()));
    assert_eq!(empty.size_hint().exact(), Some(0));
    assert!(empty.is_end_stream());

    let from_bytes: HttpBody = Bytes::from_static(b"from-bytes").into();
    assert_eq!(from_bytes.size_hint().exact(), Some(10));

    // 2. `ChannelBody` / `HttpBody::Stream` reports unknown length, skips empty chunks, and ends cleanly.
    let (tx, rx) = mpsc::channel(4);
    let mut stream = HttpBody::stream(rx);
    assert_eq!(stream.size_hint().lower(), 0);
    assert_eq!(stream.size_hint().upper(), None);
    assert!(!stream.is_end_stream());

    tx.send(Bytes::new()).await.unwrap();
    tx.send(Bytes::from_static(b"chunk-1")).await.unwrap();
    tx.send(Bytes::new()).await.unwrap();
    tx.send(Bytes::from_static(b"chunk-2")).await.unwrap();
    drop(tx);

    let f1 = stream.frame().await.unwrap().unwrap().into_data().unwrap();
    let f2 = stream.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(f1, Bytes::from_static(b"chunk-1"));
    assert_eq!(f2, Bytes::from_static(b"chunk-2"));
    assert!(stream.frame().await.is_none());
    assert!(stream.is_end_stream());

    // Direct `ChannelBody` -> `HttpBody` conversion.
    let (_tx2, rx2) = mpsc::channel(1);
    let body2: HttpBody = ChannelBody::new(rx2).into();
    assert_eq!(body2.size_hint().upper(), None);
}

#[derive(Clone)]
struct StreamingTestService {
    release_second_chunk: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
    disconnect_cancelled: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl hyper::service::Service<Request<Incoming>> for StreamingTestService {
    type Response = Response<HttpBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let release_slot = Arc::clone(&self.release_second_chunk);
        let cancel_slot = Arc::clone(&self.disconnect_cancelled);
        Box::pin(async move {
            match path.as_str() {
                "/full" => {
                    let resp = Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/plain")
                        .body(HttpBody::full(Bytes::from_static(b"buffered-ok")))
                        .expect("valid HTTP response");
                    Ok(resp)
                }
                "/stream" => {
                    let (tx, rx) = mpsc::channel(4);
                    let gate = release_slot.lock().await.take();
                    tokio::spawn(async move {
                        let _ = tx.send(Bytes::from_static(b"SHELL-CHUNK-1\n")).await;
                        if let Some(rx_gate) = gate {
                            let _ = rx_gate.await;
                        }
                        let _ = tx.send(Bytes::from_static(b"SLOT-CHUNK-2\n")).await;
                        let _ = tx.send(Bytes::from_static(b"TRAILER-CHUNK-3\n")).await;
                    });
                    let resp = Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/html; charset=utf-8")
                        .header("cache-control", "private, no-store, no-transform")
                        .header("x-accel-buffering", "no")
                        .body(HttpBody::stream(rx))
                        .expect("valid HTTP response");
                    Ok(resp)
                }
                "/keepalive" => {
                    let (tx, rx) = mpsc::channel(2);
                    let notify = cancel_slot.lock().await.take();
                    tokio::spawn(async move {
                        if tx.send(Bytes::from_static(b"FIRST-FRAME\n")).await.is_err() {
                            if let Some(done) = notify {
                                let _ = done.send(());
                            }
                            return;
                        }
                        let mut tick = tokio::time::interval(Duration::from_millis(15));
                        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        loop {
                            tokio::select! {
                                _ = tx.closed() => break,
                                _ = tick.tick() => {
                                    if tx.send(Bytes::from_static(b"<!-- nve-keepalive -->\n")).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        if let Some(done) = notify {
                            let _ = done.send(());
                        }
                    });
                    let resp = Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/html; charset=utf-8")
                        .body(HttpBody::stream(rx))
                        .expect("valid HTTP response");
                    Ok(resp)
                }
                _ => Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(HttpBody::default())
                    .expect("valid HTTP response")),
            }
        })
    }
}

async fn read_until_contains(
    stream: &mut TcpStream,
    acc: &mut Vec<u8>,
    needle: &[u8],
    timeout: Duration,
) {
    tokio::time::timeout(timeout, async {
        let mut buf = [0u8; 512];
        while !acc.windows(needle.len()).any(|w| w == needle) {
            let n = stream.read(&mut buf).await.expect("tcp read");
            assert!(n > 0, "unexpected EOF while waiting for {:?}", String::from_utf8_lossy(needle));
            acc.extend_from_slice(&buf[..n]);
        }
    })
    .await
    .expect("timed out waiting for streamed bytes");
}

#[tokio::test]
async fn hyper_http1_streams_incrementally_and_cancels_producer_on_client_drop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (release_tx, release_rx) = oneshot::channel();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let svc = StreamingTestService {
        release_second_chunk: Arc::new(Mutex::new(Some(release_rx))),
        disconnect_cancelled: Arc::new(Mutex::new(Some(cancel_tx))),
    };
    let config = ServeConfig {
        graceful_shutdown_timeout: Duration::from_secs(2),
        ..ServeConfig::default()
    };
    let server_task = tokio::spawn(async move {
        serve_service_with_config(listener, svc, config, async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap();
    });

    // 1. Buffered `HttpBody::Full` sends exact Content-Length (no chunked framing).
    {
        let mut tcp = TcpStream::connect(addr).await.unwrap();
        tcp.write_all(b"GET /full HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut resp = Vec::new();
        tcp.read_to_end(&mut resp).await.unwrap();
        let text = String::from_utf8(resp).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
        assert!(contains_ignore_ascii_case(&text, "content-length: 11"), "{text}");
        assert!(!contains_ignore_ascii_case(&text, "transfer-encoding: chunked"), "{text}");
        assert!(text.ends_with("\r\n\r\nbuffered-ok"), "{text}");
    }

    // 2. Streaming `HttpBody::Stream` flushes chunk 1 BEFORE chunk 2 is even generated!
    {
        let mut tcp = TcpStream::connect(addr).await.unwrap();
        tcp.write_all(b"GET /stream HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();

        let mut received = Vec::new();
        read_until_contains(&mut tcp, &mut received, b"SHELL-CHUNK-1\n", Duration::from_secs(2)).await;
        let partial = String::from_utf8_lossy(&received);
        assert!(
            contains_ignore_ascii_case(&partial, "transfer-encoding: chunked"),
            "Hyper must frame HttpBody::Stream as chunked over HTTP/1.1: {partial}"
        );
        assert!(
            !partial.contains("SLOT-CHUNK-2"),
            "chunk 2 must not exist before gate release: {partial}"
        );

        // Release chunk 2 & 3, then read cleanly to EOF (verifying chunk terminator `0\r\n\r\n`).
        release_tx.send(()).unwrap();
        tcp.read_to_end(&mut received).await.unwrap();
        let full = String::from_utf8(received).unwrap();
        assert!(full.contains("SLOT-CHUNK-2\n"), "{full}");
        assert!(full.contains("TRAILER-CHUNK-3\n"), "{full}");
        assert!(full.ends_with("0\r\n\r\n"), "missing HTTP/1.1 chunk terminator: {full}");
    }

    // 3. Dropping the client TcpStream mid-stream cancels the producer promptly via keepalive write.
    {
        let mut tcp = TcpStream::connect(addr).await.unwrap();
        tcp.write_all(b"GET /keepalive HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut received = Vec::new();
        read_until_contains(&mut tcp, &mut received, b"FIRST-FRAME\n", Duration::from_secs(2)).await;
        // Abort client connection abruptly mid-stream.
        drop(tcp);
        tokio::time::timeout(Duration::from_secs(2), cancel_rx)
            .await
            .expect("producer must detect dropped TCP client and cancel promptly")
            .expect("cancel sender must report producer exit");
    }

    let _ = shutdown_tx.send(());
    server_task.await.unwrap();
}
