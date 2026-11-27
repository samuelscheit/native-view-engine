//! Integration tests for `nve-rpc` split across `codec_tests` and `protocol_tests` to stay
//! focused and well below the 650-line file limit while sharing a single `CountingAlloc` binary.

mod codec_tests;
mod protocol_tests;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_rpc::{procedures, DomainStatus, FieldErrors, RpcStatus, Tag64};

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
pub(crate) struct IssueV1 {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub title: String,
    #[n(2)]
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub(crate) struct IssueV2 {
    #[n(0)]
    pub id: u64,
    #[n(1)]
    pub title: String,
    #[n(2)]
    pub open: bool,
    #[n(3)]
    pub priority: Option<u8>,
    #[n(4)]
    pub labels: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
pub(crate) enum DemoError {
    #[n(0)]
    NotFound,
    #[n(1)]
    Invalid(#[n(0)] FieldErrors),
}

impl DomainStatus for DemoError {
    fn rpc_status(&self) -> RpcStatus {
        match self {
            Self::NotFound => RpcStatus::NotFound,
            Self::Invalid(_) => RpcStatus::Unprocessable,
        }
    }
}

procedures! {
    pub(crate) query GetIssue = "issues.v1.Issues/Get" (u64) -> IssueV2, DemoError,
        tags |id, sink| {
            sink.push(Tag64::keyed("issue", *id));
        };
    pub(crate) action RenameIssue = "issues.v1.Issues/Rename" (IssueV1) -> IssueV2, DemoError,
        invalidates |req, sink| {
            sink.push(Tag64::of("issues.list"));
            sink.push(Tag64::keyed("issue", req.id));
        };
}
