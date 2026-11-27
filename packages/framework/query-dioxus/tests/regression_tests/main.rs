#![cfg(not(target_arch = "wasm32"))]
//! Permanent adversarial regression guards for `nve-query-dioxus` and `nve-ui` `<Form>`, split
//! across focused submodules so every file stays well under 650 lines.

mod dedup_retry_tests;
mod external_fetch_tests;
mod lifecycle_opt_tests;
mod query_form_tests;
mod ssr_registry_tests;
mod suspense_retry_tests;
mod suspense_ssr_recovery_tests;
mod suspense_tests;

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::task::{Poll, Waker};
use std::alloc::System;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use nve_backend_web::WebBackend;
use nve_dioxus::DioxusSurface;
use nve_engine::RenderBackend;
use nve_rpc::{
    procedures, BoxFuture, CborCodec, DomainStatus, FieldErrors, FormDecode, FormPairs, Procedure,
    RawCall, RawReply, ReplyKind, RpcCodec, RpcStatus, Tag64, Transport, TransportError,
};

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

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().saturating_add(1)));
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().saturating_add(1)));
        unsafe { System.realloc(ptr, layout, new_size) }
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
pub(crate) struct Item {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub(crate) struct RenameItemInput {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub name: String,
}

impl FormDecode for RenameItemInput {
    fn from_form(form: &FormPairs) -> Result<Self, FieldErrors> {
        let mut errs = FieldErrors::new();
        let id = form
            .get("id")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1);
        let name = form.get("name").unwrap_or("").trim().to_owned();
        if name.is_empty() {
            errs.push("name", "name required");
        }
        errs.into_result(Self { id, name })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
pub(crate) enum ItemErr {
    #[n(0)]
    Denied,
}

impl DomainStatus for ItemErr {
    fn rpc_status(&self) -> RpcStatus {
        RpcStatus::PermissionDenied
    }
}

procedures! {
    pub(crate) query GetItem = "reg.v1.Items/Get" (u64) -> Item, ItemErr,
        tags |id, sink| {
            sink.push(Tag64::keyed("item", *id));
        };
    pub(crate) action RenameItem = "reg.v1.Items/Rename" (RenameItemInput) -> Item, ItemErr,
        invalidates |inp, sink| {
            sink.push(Tag64::keyed("item", inp.id));
        };
}

#[derive(Clone, Default)]
pub(crate) struct RegTransport {
    pub inner: Arc<Mutex<RegState>>,
}

#[derive(Default)]
pub(crate) struct RegState {
    pub fail_queries: bool,
    pub transient_failures_left: usize,
    pub defer_queries: bool,
    pub defer_actions: bool,
    pub query_calls: usize,
    pub action_calls: usize,
    pub name: String,
    pub wakers: Vec<Waker>,
}

impl RegTransport {
    pub fn new(name: &str) -> Self {
        let t = Self::default();
        t.inner.lock().unwrap().name = name.to_owned();
        t
    }

    pub fn release_all(&self) {
        let wakers = {
            let mut g = self.inner.lock().unwrap();
            g.defer_queries = false;
            g.defer_actions = false;
            core::mem::take(&mut g.wakers)
        };
        for w in wakers {
            w.wake();
        }
    }
}

impl Transport for RegTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            if req.name == GetItem::NAME {
                let id: u64 = CborCodec::default().decode_bounded(&req.body).unwrap();
                core::future::poll_fn(|cx| {
                    let mut g = inner.lock().unwrap();
                    if g.defer_queries {
                        g.wakers.push(cx.waker().clone());
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
                let mut g = inner.lock().unwrap();
                g.query_calls += 1;
                if g.transient_failures_left > 0 {
                    g.transient_failures_left -= 1;
                    return Err(TransportError::timeout("transient timeout"));
                }
                if g.fail_queries {
                    return Err(TransportError::network("server down"));
                }
                let mut body = Vec::new();
                CborCodec::default()
                    .encode_into(
                        &Item {
                            id,
                            name: g.name.clone(),
                        },
                        &mut body,
                    )
                    .unwrap();
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            if req.name == RenameItem::NAME {
                let inp: RenameItemInput = CborCodec::default().decode_bounded(&req.body).unwrap();
                core::future::poll_fn(|cx| {
                    let mut g = inner.lock().unwrap();
                    if g.defer_actions {
                        g.wakers.push(cx.waker().clone());
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
                let mut g = inner.lock().unwrap();
                g.action_calls += 1;
                g.name = inp.name.clone();
                let mut body = Vec::new();
                CborCodec::default()
                    .encode_into(
                        &Item {
                            id: inp.id,
                            name: inp.name,
                        },
                        &mut body,
                    )
                    .unwrap();
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: vec![Tag64::keyed("item", inp.id)],
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            Err(TransportError::config("unknown proc"))
        })
    }
}

pub(crate) struct NoopFlagWaker(pub AtomicBool);

impl std::task::Wake for NoopFlagWaker {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }
}

pub(crate) fn pump_settled(
    surface: &mut DioxusSurface,
    web: &mut WebBackend,
    waker: &Waker,
    frame: &mut u64,
) -> usize {
    let mut n = 0;
    for _ in 0..24 {
        if !surface.poll_work(waker) {
            return n;
        }
        *frame += 1;
        let (_, _, buf) = surface.render_frame(*frame);
        web.submit_frame(&buf).unwrap();
        n += 1;
    }
    assert!(
        !surface.poll_work(waker),
        "DioxusSurface failed to settle within 24 async pumps!"
    );
    n
}

pub(crate) fn render_once(surface: &mut DioxusSurface, web: &mut WebBackend, frame: &mut u64) -> String {
    *frame += 1;
    let (_, _, buf) = surface.render_frame(*frame);
    web.submit_frame(&buf).unwrap();
    web.render_html()
}
