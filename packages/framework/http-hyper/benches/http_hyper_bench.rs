use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use nve_http_hyper::{serve_rpc_with_shutdown, HyperTransport};
use nve_rpc::{call, procedures};
use nve_server::{AnonymousAuth, CsrfPolicy, Cx, RpcRegistry, Server, ServerConfig};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

procedures! {
    query LoopPing = "bench.v1.Hyper/Ping" (u64) -> u64, ();
}

fn bench_hyper_loopback_rpc(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    let (addr, tx, handle) = rt.block_on(async {
        let mut reg = RpcRegistry::<(), ()>::builder();
        reg.register::<LoopPing, _, _>(|_cx: Cx<(), ()>, n: u64| async move {
            Ok(n.wrapping_add(1))
        });
        let server = Server::new(
            (),
            reg.build(),
            AnonymousAuth,
            CsrfPolicy::allow_origins(["http://127.0.0.1"]),
            ServerConfig::default(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let (tx, rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = serve_rpc_with_shutdown(listener, server, async {
                let _ = rx.await;
            })
            .await;
        });
        (addr, tx, handle)
    });

    let transport = HyperTransport::new(format!("http://{addr}"));
    c.bench_function("nve_http_hyper_loopback_cbor_rpc", |b| {
        b.iter(|| {
            let reply = rt
                .block_on(call::<LoopPing>(&transport, black_box(&41u64)))
                .expect("loopback rpc");
            black_box(reply.value)
        });
    });

    let _ = tx.send(());
    let _ = rt.block_on(handle);
}

criterion_group!(benches, bench_hyper_loopback_rpc);
criterion_main!(benches);
