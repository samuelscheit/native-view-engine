//! Shared harness, test procedures, and `CountingAlloc` global allocator for `nve-server`
//! integration tests (split across `csrf_auth_tests` and `dispatch_tests` to stay well under
//! the 650-line file cap).

mod csrf_auth_tests;
mod dispatch_tests;

use core::future::Future;
use core::task::{Context, Poll, Waker};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use http::HeaderMap;
use nve_rpc::{procedures, DomainStatus, FieldErrors, FormDecode, FormPairs, RpcStatus, Tag64};
use nve_server::{
    extract_cookie_value, extract_credential, AuthCredential, AuthError, AuthResolver, CsrfPolicy,
    Cx, RpcRegistry, Server, ServerConfig,
};

pub(crate) fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("sans-I/O test future unexpectedly yielded Poll::Pending"),
    }
}

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().saturating_add(1)));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

pub(crate) fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (out, after.saturating_sub(before))
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub(crate) struct RenameReq {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub title: String,
}

impl FormDecode for RenameReq {
    fn from_form(form: &FormPairs) -> Result<Self, FieldErrors> {
        let mut errs = FieldErrors::new();
        let id = form
            .get("id")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or_else(|| {
                errs.push("id", "valid numeric id required");
                0
            });
        let title = form.get("title").unwrap_or("").trim().to_owned();
        if title.is_empty() {
            errs.push("title", "title must not be empty");
        }
        errs.into_result(Self { id, title })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub(crate) struct RenameRes {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub title: String,
    #[n(2)]
    pub actor: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
pub(crate) enum DemoErr {
    #[n(0)]
    NotFound,
    #[n(1)]
    BadStatusOk,
}

impl DomainStatus for DemoErr {
    fn rpc_status(&self) -> RpcStatus {
        match self {
            Self::NotFound => RpcStatus::NotFound,
            Self::BadStatusOk => RpcStatus::Ok, // intentionally buggy: server must clamp to 422!
        }
    }
}

procedures! {
    pub(crate) query Ping = "demo.v1.Health/Ping" (u64) -> u64, DemoErr;
    pub(crate) action Rename = "demo.v1.Issues/Rename" (RenameReq) -> RenameRes, DemoErr,
        invalidates |req, sink| {
            sink.push(Tag64::of("issues.list"));
            sink.push(Tag64::keyed("issue", req.id));
        };
    pub(crate) action BulkTouch = "demo.v1.Issues/BulkTouch" (u64) -> u64, DemoErr;
}

#[derive(Clone)]
pub(crate) struct DemoAuth;

impl AuthResolver<(), String> for DemoAuth {
    fn try_resolve_sync(
        &self,
        _state: &(),
        headers: &HeaderMap,
    ) -> Option<Result<String, AuthError>> {
        Some(match extract_credential(headers, "nve_session") {
            AuthCredential::Bearer("native-token") => Ok("native-user".to_owned()),
            AuthCredential::Cookie("web-cookie") => Ok("web-user".to_owned()),
            AuthCredential::None => Err(AuthError::unauthenticated("missing credentials")),
            _ => Err(AuthError::unauthenticated("invalid credentials")),
        })
    }

    fn resolve<'a>(
        &'a self,
        state: &'a (),
        headers: &'a HeaderMap,
    ) -> nve_rpc::BoxFuture<'a, Result<String, AuthError>> {
        let res = self.try_resolve_sync(state, headers).unwrap();
        Box::pin(async move { res })
    }
}

/// Hostile cookie-first resolver proving `Server` physically strips `Cookie` before calling
/// `AuthResolver` whenever `Authorization: Bearer` bypassed browser CSRF checks.
#[derive(Clone)]
pub(crate) struct CookieFirstBuggyAuth;

impl AuthResolver<(), String> for CookieFirstBuggyAuth {
    fn resolve<'a>(
        &'a self,
        _state: &'a (),
        headers: &'a HeaderMap,
    ) -> nve_rpc::BoxFuture<'a, Result<String, AuthError>> {
        let res = if let Some(c) = extract_cookie_value(headers, "nve_session") {
            Ok(format!("cookie:{c}"))
        } else {
            Err(AuthError::unauthenticated("no cookie visible"))
        };
        Box::pin(async move { res })
    }
}

pub(crate) fn build_test_server() -> Server<(), String, DemoAuth> {
    let mut reg = RpcRegistry::<(), String>::builder();
    reg.register::<Ping, _, _>(|_cx: Cx<(), String>, n: u64| async move {
        if n == 0 {
            Err(DemoErr::NotFound)
        } else if n == 999 {
            Err(DemoErr::BadStatusOk)
        } else {
            Ok(n + 1)
        }
    });
    reg.register_with_form::<Rename, _, _>(|cx: Cx<(), String>, req: RenameReq| async move {
        Ok(RenameRes {
            id: req.id,
            title: req.title,
            actor: cx.principal,
        })
    });
    reg.register::<BulkTouch, _, _>(|cx: Cx<(), String>, count: u64| async move {
        if count == 0 {
            cx.invalidate_all();
        } else {
            cx.invalidate_many((0..count).map(|i| Tag64::keyed("bulk", i)));
        }
        Ok(count)
    });
    Server::new(
        (),
        reg.build(),
        DemoAuth,
        CsrfPolicy::allow_origins(["https://APP.example.com:443/"]),
        ServerConfig {
            max_body_bytes: 512,
            max_form_fields: 16,
            ..ServerConfig::default()
        }
        .with_form_route::<Rename>("/forms/rename", "/issues/7"),
    )
}
