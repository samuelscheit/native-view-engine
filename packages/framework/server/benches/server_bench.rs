use core::future::Future;
use core::task::{Context, Poll, Waker};
use std::hint::black_box;

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use http::header::{CONTENT_TYPE, COOKIE, ORIGIN};
use http::{HeaderMap, Method, Request};
use nve_rpc::{procedures, CborCodec, Procedure, RpcCodec, CONTENT_TYPE_CBOR};
use nve_server::{
    extract_credential, AuthCredential, AuthError, AuthResolver, CsrfPolicy, Cx, RpcRegistry,
    Server, ServerConfig,
};

fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let ready = match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => Some(out),
        Poll::Pending => None,
    };
    ready.expect("sans-I/O bench future unexpectedly yielded Poll::Pending")
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct BenchIssueReq {
    #[n(0)]
    id: u64,
    #[n(1)]
    title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct BenchIssueRes {
    #[n(0)]
    id: u64,
    #[n(1)]
    title: String,
    #[n(2)]
    actor_id: u64,
}

procedures! {
    query BenchGetIssue = "bench.v1.Issues/Get" (BenchIssueReq) -> BenchIssueRes, ();
}

#[derive(Clone)]
struct BenchCookieAuth;

impl AuthResolver<(), u64> for BenchCookieAuth {
    #[inline]
    fn try_resolve_sync(&self, _state: &(), headers: &HeaderMap) -> Option<Result<u64, AuthError>> {
        Some(match extract_credential(headers, "nve_session") {
            AuthCredential::Cookie("bench-session-token") => Ok(42),
            _ => Err(AuthError::unauthenticated("unauthorized")),
        })
    }

    fn resolve<'a>(
        &'a self,
        state: &'a (),
        headers: &'a HeaderMap,
    ) -> nve_rpc::BoxFuture<'a, Result<u64, AuthError>> {
        let res = self.try_resolve_sync(state, headers).expect("sync auth");
        Box::pin(async move { res })
    }
}

fn bench_server_dispatch(c: &mut Criterion) {
    let mut reg = RpcRegistry::<(), u64>::builder();
    reg.register::<BenchGetIssue, _, _>(|cx: Cx<(), u64>, req: BenchIssueReq| async move {
        Ok(BenchIssueRes {
            id: req.id,
            title: req.title,
            actor_id: cx.principal,
        })
    });
    let registry = reg.build();

    c.bench_function("nve_server_registry_lookup", |b| {
        b.iter(|| black_box(registry.lookup(black_box(BenchGetIssue::NAME)).is_some()));
    });

    let server = Server::new(
        (),
        registry,
        BenchCookieAuth,
        CsrfPolicy::allow_origins(["https://bench.example.com"]),
        ServerConfig::default(),
    );
    let mut arg_bytes = Vec::new();
    CborCodec::default()
        .encode_into(
            &BenchIssueReq {
                id: 7,
                title: "Benchmark CSRF + Cookie + CBOR struct".into(),
            },
            &mut arg_bytes,
        )
        .expect("encode arg");
    let payload = Bytes::from(arg_bytes);

    // Measure ONLY `Server::handle_rpc_request` (setup runs outside the timed region via `iter_batched`).
    c.bench_function("nve_server_handle_authenticated_cbor_struct", |b| {
        b.iter_batched(
            || {
                Request::builder()
                    .method(Method::POST)
                    .uri("/rpc/bench.v1.Issues/Get")
                    .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
                    .header(ORIGIN, "https://bench.example.com")
                    .header("sec-fetch-site", "same-origin")
                    .header(COOKIE, "nve_session=bench-session-token")
                    .body(payload.clone())
                    .expect("build request")
            },
            |req| {
                let resp = block_on(server.handle_rpc_request(black_box(req)));
                black_box(resp.status().as_u16())
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_server_dispatch);
criterion_main!(benches);
