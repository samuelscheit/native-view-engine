//! Multi-file integration, adversarial, property, and zero-allocation test suite for `nve-query`.

mod adversarial_tests;
mod core_tests;
mod regression_tests;
mod ssr_stream_tests;
mod stream_pending_tests;

use core::future::Future;
use core::task::{Context, Poll, Waker};
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use nve_query::Shared;
use nve_rpc::{
    procedures, BoxFuture, CborCodec, DomainStatus, Procedure, RawCall, RawReply, ReplyKind,
    RpcCodec, Tag64, Transport, TransportError,
};

#[cfg(not(target_arch = "wasm32"))]
mod alloc_guard {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    struct CountingAlloc;

    thread_local! {
        static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
    }

    // SAFETY: Delegates directly to `std::alloc::System` while incrementing a thread-local counter.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().saturating_add(1)));
            // SAFETY: Forwards caller-validated `layout` to the global `System` allocator.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: Forwards caller-validated `(ptr, layout)` allocated by `System`.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static GLOBAL: CountingAlloc = CountingAlloc;

    pub fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
        let before = THREAD_ALLOCS.with(Cell::get);
        let out = f();
        let after = THREAD_ALLOCS.with(Cell::get);
        (out, after.saturating_sub(before))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use alloc_guard::count_allocs;

pub fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let ready = match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => Some(out),
        Poll::Pending => None,
    };
    ready.expect("sans-I/O query test future unexpectedly yielded Poll::Pending")
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct Item {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub title: String,
    #[n(2)]
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct RenameInput {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
pub enum DemoErr {
    #[n(0)]
    Rejected,
}

impl DomainStatus for DemoErr {}

procedures! {
    query ListItems = "demo.v1.Items/List" (u64) -> Vec<Item>, DemoErr,
        tags |_page, sink| {
            sink.push(Tag64::of("items.list"));
        };
    query GetItem = "demo.v1.Items/Get" (u64) -> Item, DemoErr,
        tags |id, sink| {
            sink.push(Tag64::keyed("item", *id));
        };
    query ManyTagQuery = "demo.v1.Items/ManyTags" (u64) -> Item, DemoErr,
        tags |id, sink| {
            for i in 0..140u64 {
                sink.push(Tag64::keyed("many", id * 1000 + i));
            }
        };
    action RenameItem = "demo.v1.Items/Rename" (RenameInput) -> Item, DemoErr,
        invalidates |input, sink| {
            sink.push(Tag64::of("items.list"));
            sink.push(Tag64::keyed("item", input.id));
        };
}

#[derive(Clone, Default)]
pub struct ScriptedTransport {
    pub calls: Shared<AtomicUsize>,
    pub fail_first_n: Shared<AtomicUsize>,
    pub action_fail: Shared<AtomicUsize>,
}

impl Transport for ScriptedTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        let fail_n = self.fail_first_n.load(Ordering::Relaxed);
        let action_fail = self.action_fail.load(Ordering::Relaxed);
        Box::pin(async move {
            if req.name == ListItems::NAME {
                if n < fail_n {
                    return Err(TransportError::network("transient disconnect"));
                }
                let items = vec![
                    Item {
                        id: 1,
                        title: format!("Alpha v{n}"),
                        closed: false,
                    },
                    Item {
                        id: 2,
                        title: "Beta".into(),
                        closed: false,
                    },
                ];
                let mut body = Vec::new();
                CborCodec::default().encode_into(&items, &mut body).expect("encode items");
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            if req.name == GetItem::NAME {
                let id: u64 = CborCodec::default().decode_bounded(&req.body).expect("decode id");
                let item = Item {
                    id,
                    title: format!("Item {id} v{n}"),
                    closed: false,
                };
                let mut body = Vec::new();
                CborCodec::default().encode_into(&item, &mut body).expect("encode item");
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            if req.name == RenameItem::NAME {
                if action_fail == 1 {
                    let mut body = Vec::new();
                    CborCodec::default()
                        .encode_into(&DemoErr::Rejected, &mut body)
                        .expect("encode err");
                    return Ok(RawReply {
                        http_status: 422,
                        kind: ReplyKind::Domain,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        body: Bytes::from(body),
                    });
                }
                if action_fail == 2 {
                    return Err(TransportError::timeout("gateway timeout"));
                }
                let input: RenameInput = CborCodec::default().decode_bounded(&req.body).expect("decode rename");
                let updated = Item {
                    id: input.id,
                    title: input.title,
                    closed: false,
                };
                let mut body = Vec::new();
                CborCodec::default().encode_into(&updated, &mut body).expect("encode updated");
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: vec![
                        Tag64::of("items.list"),
                        Tag64::keyed("item", input.id),
                    ],
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            Err(TransportError::config("unexpected procedure"))
        })
    }
}
