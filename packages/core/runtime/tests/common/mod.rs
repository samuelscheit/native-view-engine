//! Helpers shared by the runtime integration tests (each test binary compiles its own copy).
#![allow(dead_code)]

use nve_abi::{
    NveBufferSlice, NveRuntimeApi, NveRuntimeHandle, NveRuntimeSceneView, NveRuntimeStatus,
    NVE_RUNTIME_ABI_VERSION,
};
use nve_components::tables::{Layout, TextContent};
use nve_runtime::nve_runtime_get_interface;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

pub const OK: i32 = NveRuntimeStatus::Ok as i32;
pub const INVALID: i32 = NveRuntimeStatus::InvalidArg as i32;
pub const UNSUPPORTED: i32 = NveRuntimeStatus::Unsupported as i32;
pub const BUSY: i32 = NveRuntimeStatus::Busy as i32;
pub const NOT_FOUND: i32 = NveRuntimeStatus::NotFound as i32;

pub fn api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable returned for the matching ABI version.
    unsafe { *ptr }
}

/// RAII C-ABI surface for the duration of one test.
pub struct Surface {
    pub api: NveRuntimeApi,
    pub h: NveRuntimeHandle,
}

impl Surface {
    pub fn new() -> Self {
        let api = api();
        // SAFETY: NULL config selects defaults.
        let h = unsafe { (api.create)(std::ptr::null()) };
        assert!(!h.is_null());
        Self { api, h }
    }

    pub fn submit(&self, frame: &[u8]) {
        // SAFETY: Live handle; `frame` outlives the call.
        assert_eq!(unsafe { (self.api.submit_nve2)(self.h, NveBufferSlice::from_slice(frame)) }, OK);
        self.tick();
    }

    pub fn tick(&self) {
        // SAFETY: Live handle; null out slot allowed.
        assert_eq!(unsafe { (self.api.tick)(self.h, 0.0, std::ptr::null_mut()) }, OK);
    }

    pub fn acquire(&self) -> NveRuntimeSceneView {
        let mut view = NveRuntimeSceneView::default();
        // SAFETY: Live handle and valid out slot.
        assert_eq!(unsafe { (self.api.scene_acquire)(self.h, &mut view) }, OK);
        view
    }

    pub fn release(&self) {
        // SAFETY: Live handle.
        assert_eq!(unsafe { (self.api.scene_release)(self.h) }, OK);
    }

    pub fn rect_height(&self, node: NodeId) -> f32 {
        let view = self.acquire();
        // SAFETY: Reader lock held; index checked against `node_capacity`.
        let h = unsafe {
            assert!(node.index() < view.node_capacity as usize);
            (*view.rects.add(node.index())).height
        };
        self.release();
        h
    }

    pub fn resource(&self, id: u32) -> (i32, u64, Vec<u8>) {
        let (mut kind, mut payload) = (0u64, NveBufferSlice::EMPTY);
        // SAFETY: Live handle and valid out slots.
        let st = unsafe { (self.api.resource_payload)(self.h, id, &mut kind, &mut payload) };
        // SAFETY: On OK the payload is readable while the caller's scene reader is held.
        let bytes = unsafe { payload.try_as_slice() }.unwrap_or_default().to_vec();
        (st, kind, bytes)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: Handle created by `create` and destroyed once on the owner thread.
        unsafe { (self.api.destroy)(self.h) };
    }
}

pub fn encode(store: &NodeStore, changes: &ChangeSet, enc: &mut WireEncoder) -> Vec<u8> {
    let mut out = Vec::new();
    enc.encode(store, changes, &mut out);
    out
}

/// Root column with one 300pt-wide text leaf (`id = 2`).
pub fn text_frame() -> (Vec<u8>, NodeId) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let text = tx.create();
    tx.edit::<Layout>(text).width(Length::points(300.0));
    tx.edit::<TextContent>(text).text("hello native text");
    tx.set_children(NodeId::ROOT, &[text]);
    tx.commit_into(&mut changes);
    (encode(&store, &changes, &mut WireEncoder::default()), text)
}
