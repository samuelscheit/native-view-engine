use std::ffi::c_void;

use nve_abi::{
    EventBatchBuf, FrameId, InterfaceId, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi,
    NveEventBatch, NveGestureDisposition, NveIntrinsicSize, NveLayoutApi, NveMapEntry, NvePropsRef,
    NveSizeConstraints, NveSurfaceApi, NveValue, SurfaceId, TimestampUs, ValidatedEvents,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use nve_tables::tables::TextContent;
use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{ChangeSet, NodeId, NodeStore};

use super::*;

fn encode_text_frame(label: &str) -> Vec<u8> {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let n = tx.create();
    tx.edit::<TextContent>(n).text(label);
    tx.set_children(NodeId::ROOT, &[n]);
    tx.commit_into(&mut changes);
    let mut enc = WireEncoder::default();
    let mut out = Vec::new();
    enc.encode(&store, &changes, &mut out);
    out
}

fn decode_first_text(bytes: &[u8]) -> String {
    let mut replica = NodeStore::replica();
    let mut dec = WireDecoder::default();
    let mut changes = ChangeSet::default();
    let mut tx = replica.tx();
    dec.apply(&mut tx, bytes).expect("decode nve2");
    tx.commit_into(&mut changes);
    let child = replica.children(NodeId::ROOT)[0];
    replica
        .try_row::<TextContent>(child)
        .expect("TextContent attached")
        .text()
        .to_owned()
}

// ---- Test Plugin V1: full capability plugin (SURFACE + LAYOUT + COMPONENT + DEV) ----
pub(super) struct MockPluginState {
    pub(super) surface_id: SurfaceId,
    pub(super) version_tag: &'static str,
    pub(super) counter: u32,
    pub(super) frame_buf: Vec<u8>,
    pub(super) state_buf: Vec<u8>,
    pub(super) out_events: EventBatchBuf,
}

unsafe extern "C" fn p1_create(surface_id: SurfaceId) -> NveAppHandle {
    Box::into_raw(Box::new(MockPluginState {
        surface_id,
        version_tag: "v1",
        counter: 1,
        frame_buf: Vec::new(),
        state_buf: Vec::new(),
        out_events: EventBatchBuf::new(),
    })) as NveAppHandle
}

unsafe extern "C" fn p2_create(surface_id: SurfaceId) -> NveAppHandle {
    Box::into_raw(Box::new(MockPluginState {
        surface_id,
        version_tag: "v2",
        counter: 1,
        frame_buf: Vec::new(),
        state_buf: Vec::new(),
        out_events: EventBatchBuf::new(),
    })) as NveAppHandle
}

unsafe extern "C" fn p_destroy(handle: NveAppHandle) {
    if !handle.is_null() {
        // SAFETY: `handle` was allocated by `p1_create`/`p2_create` via `Box::into_raw`.
        unsafe { drop(Box::from_raw(handle as *mut MockPluginState)) };
    }
}

unsafe extern "C" fn p_render(handle: NveAppHandle, _frame_id: FrameId) -> NveBufferSlice {
    // SAFETY: `NvePluginHost` passes a live exclusive handle from `p1_create`/`p2_create`.
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    st.frame_buf = encode_text_frame(&format!("{}:count={}", st.version_tag, st.counter));
    NveBufferSlice::from_slice(&st.frame_buf)
}

unsafe extern "C" fn p_dispatch(handle: NveAppHandle, _events: NveEventBatch) -> u32 {
    // SAFETY: `NvePluginHost` passes a live exclusive handle from `p1_create`/`p2_create`.
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    st.counter += 1;
    1
}

unsafe extern "C" fn p_measure(
    _handle: NveAppHandle,
    c: NveSizeConstraints,
) -> NveIntrinsicSize {
    NveIntrinsicSize {
        width: c.max_w.min(240.0),
        height: c.max_h.min(80.0),
    }
}

unsafe extern "C" fn p_hit_test(_: NveAppHandle, _: f32, _: f32, _: u8) -> u8 {
    NveGestureDisposition::Claim as u8
}

pub(super) unsafe extern "C" fn p_set_props(
    handle: NveAppHandle,
    entries: *const NveMapEntry,
    count: u32,
) -> u32 {
    // SAFETY: `NvePluginHost` passes a live exclusive handle and readable `entries[..count]`.
    let (st, props) = unsafe {
        (
            &mut *(handle as *mut MockPluginState),
            NvePropsRef::from_raw_parts(entries, count),
        )
    };
    if let Some(c) = props.get("counter").and_then(|v| v.as_i64()) {
        st.counter = c as u32;
        return 1;
    }
    0
}

unsafe extern "C" fn p_drain_events(handle: NveAppHandle) -> NveEventBatch {
    // SAFETY: `NvePluginHost` passes a live exclusive handle from `p1_create`/`p2_create`.
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    st.out_events.clear();
    st.out_events.push_press(
        st.surface_id,
        NodeId::new(st.counter, 0),
        TimestampUs(0),
        0.0,
        0.0,
    );
    st.out_events.as_raw_batch()
}

unsafe extern "C" fn p_hot_patch(_: NveAppHandle, _: NveBufferSlice) -> u32 {
    1
}

unsafe extern "C" fn p_snapshot(handle: NveAppHandle) -> NveBufferSlice {
    // SAFETY: `NvePluginHost` passes a live exclusive handle from `p1_create`/`p2_create`.
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    st.state_buf = st.counter.to_le_bytes().to_vec();
    NveBufferSlice::from_slice(&st.state_buf)
}

unsafe extern "C" fn p_restore(handle: NveAppHandle, state: NveBufferSlice) {
    // SAFETY: `NvePluginHost` passes a live exclusive handle and readable `state` slice.
    let (st, raw) = unsafe { (&mut *(handle as *mut MockPluginState), state.as_slice()) };
    if let Ok(bytes) = <[u8; 4]>::try_from(raw) {
        st.counter = u32::from_le_bytes(bytes);
    }
}

pub(super) static P1_SURFACE: NveSurfaceApi = NveSurfaceApi {
    create: p1_create,
    destroy: p_destroy,
    render: p_render,
    dispatch_events: p_dispatch,
};
static P2_SURFACE: NveSurfaceApi = NveSurfaceApi {
    create: p2_create,
    destroy: p_destroy,
    render: p_render,
    dispatch_events: p_dispatch,
};
static P_LAYOUT: NveLayoutApi = NveLayoutApi {
    measure: p_measure,
    hit_test: p_hit_test,
};
static P_COMPONENT: NveComponentApi = NveComponentApi {
    set_props: p_set_props,
    drain_emitted_events: p_drain_events,
};
static P_DEV: NveDevApi = NveDevApi {
    apply_hot_patch: p_hot_patch,
    snapshot_state: p_snapshot,
    restore_state: p_restore,
};

pub(super) unsafe extern "C" fn plugin_v1_get_interface(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &P1_SURFACE as *const _ as *const c_void,
        NVE_IFACE_LAYOUT => &P_LAYOUT as *const _ as *const c_void,
        NVE_IFACE_COMPONENT => &P_COMPONENT as *const _ as *const c_void,
        NVE_IFACE_DEV => &P_DEV as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

pub(super) unsafe extern "C" fn plugin_v2_get_interface(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &P2_SURFACE as *const _ as *const c_void,
        NVE_IFACE_LAYOUT => &P_LAYOUT as *const _ as *const c_void,
        NVE_IFACE_COMPONENT => &P_COMPONENT as *const _ as *const c_void,
        NVE_IFACE_DEV => &P_DEV as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

pub(super) unsafe extern "C" fn surface_only_get_interface(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &P1_SURFACE as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

unsafe extern "C" fn broken_missing_surface_get_interface(_id: InterfaceId) -> *const c_void {
    std::ptr::null()
}

unsafe extern "C" fn null_handle_create(_surface_id: SurfaceId) -> NveAppHandle {
    std::ptr::null_mut()
}

static BROKEN_NULL_HANDLE_SURFACE: NveSurfaceApi = NveSurfaceApi {
    create: null_handle_create,
    destroy: p_destroy,
    render: p_render,
    dispatch_events: p_dispatch,
};

unsafe extern "C" fn broken_null_handle_get_interface(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &BROKEN_NULL_HANDLE_SURFACE as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

#[test]
fn capability_checks_isolated_surfaces_and_transactional_hotswap() {
    let mut runtime = crate::InteropRuntime::new();

    // Register an application root surface first (`SurfaceId::ROOT`) — plugins mounted on the
    // same orchestrator must NEVER collide with `SurfaceId::ROOT`!
    let app_root = runtime
        .surfaces
        .register_root(
            crate::FrameworkKind::Flutter,
            "AppRoot",
            crate::SurfaceEnvironment::default(),
        )
        .unwrap();
    assert_eq!(app_root, SurfaceId::ROOT);
    let _ = runtime.surfaces.drain_events();

    // 1. Reject plugin missing NVE_IFACE_SURFACE without burning surface slots or emitting events.
    assert_eq!(
        runtime.register_plugin_in_process(
            "bad-no-surface",
            broken_missing_surface_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::MissingSurfaceInterface)
    );

    // 2. Reject surface-only plugin when caller requires NVE_IFACE_COMPONENT.
    assert_eq!(
        runtime.register_plugin_in_process(
            "needs-component",
            surface_only_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT
        ),
        Err(PluginError::MissingRequiredInterface {
            interface_name: "NVE_IFACE_COMPONENT",
            interface_id: NVE_IFACE_COMPONENT,
        })
    );
    assert!(runtime.surfaces.drain_events().is_empty());

    // 3. Surface-only plugin mounts cleanly under SURFACE_ONLY requirement with distinct SurfaceId.
    let p_min = runtime
        .register_plugin_in_process(
            "minimal-widget",
            surface_only_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    let min_slot = runtime.plugins.get(p_min).unwrap();
    let min_sid = min_slot.surface_id();
    assert!(!min_sid.is_null());
    assert_ne!(min_sid, app_root);
    assert!(runtime.surfaces.get(app_root).is_some());
    assert!(runtime.surfaces.get(min_sid).is_some());
    assert!(min_slot.capabilities().surface);
    assert!(!min_slot.capabilities().component);
    assert!(!min_slot.capabilities().layout);

    // 4. Full-featured plugin gets a distinct generational SurfaceId visible in shared orchestrator.
    let p_full = runtime
        .register_plugin_in_process(
            "studio-inspector-plugin",
            plugin_v1_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    assert_eq!(runtime.plugins.active_count(), 2);
    assert_eq!(runtime.surfaces.active_count(), 3);
    assert_eq!(runtime.surfaces.drain_events().len(), 2);

    let slot = runtime.plugins.get_mut(p_full).unwrap();
    let full_sid = slot.surface_id();
    assert_ne!(full_sid, min_sid);
    assert_ne!(full_sid, app_root);
    assert!(slot.capabilities().component && slot.capabilities().dev && slot.capabilities().layout);
    assert_eq!(
        slot.measure(NveSizeConstraints::loose(400.0, 200.0)),
        Some(NveIntrinsicSize {
            width: 240.0,
            height: 80.0
        })
    );
    assert_eq!(slot.hit_test(10.0, 10.0, 0), NveGestureDisposition::Claim);

    // Mutate plugin state via props + events before hot-swap.
    let props = [NveMapEntry::new("counter", NveValue::i64(42))];
    assert!(slot.set_props(&props).unwrap());
    assert!(slot.dispatch_events(ValidatedEvents::EMPTY).unwrap()); // counter -> 43
    let f1 = slot.render(FrameId(1)).unwrap().to_vec();
    assert_eq!(decode_first_text(&f1), "v1:count=43");

    // 5. Failed hot-swap (broken plugin returning null handle or missing surface)
    // must NOT crash or destroy v1!
    assert_eq!(
        runtime.hot_swap_plugin_in_process(
            p_full,
            broken_null_handle_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::NullHandleReturned)
    );
    assert_eq!(
        runtime.hot_swap_plugin_in_process(
            p_full,
            broken_missing_surface_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::MissingSurfaceInterface)
    );
    // Original v1 slot is still live at generation 1 with counter=43!
    let still_v1 = runtime.plugins.get_mut(p_full).unwrap();
    assert_eq!(still_v1.generation(), 1);
    let f_after_fail = still_v1.render(FrameId(2)).unwrap().to_vec();
    assert_eq!(decode_first_text(&f_after_fail), "v1:count=43");

    // 6. Valid hot-swap v1 -> v2 preserves isolated surface_id and counter=43!
    let gen2 = runtime
        .hot_swap_plugin_in_process(
            p_full,
            plugin_v2_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    assert_eq!(gen2, 2);

    let v2_slot = runtime.plugins.get_mut(p_full).unwrap();
    assert_eq!(v2_slot.surface_id(), full_sid);
    assert_eq!(v2_slot.generation(), 2);
    let f2 = v2_slot.render(FrameId(3)).unwrap().to_vec();
    assert_eq!(decode_first_text(&f2), "v2:count=43");

    // 7. Duplicate name rejected; unload destroys the generational surface so re-registering
    // never aliases the unloaded SurfaceId!
    assert!(matches!(
        runtime.register_plugin_in_process(
            "studio-inspector-plugin",
            plugin_v1_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::DuplicatePluginName(_))
    ));
    let unloaded = runtime.unload_plugin(p_full).unwrap();
    assert_eq!(unloaded.generation, 2);
    assert!(runtime.surfaces.get(full_sid).is_none());
    assert_eq!(runtime.plugins.active_count(), 1);
    assert!(runtime.plugins.find_by_name("studio-inspector-plugin").is_none());

    let p_reborn = runtime
        .register_plugin_in_process(
            "studio-inspector-plugin",
            plugin_v1_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    let reborn_sid = runtime.plugins.get(p_reborn).unwrap().surface_id();
    assert_ne!(reborn_sid, full_sid);
    assert!(reborn_sid.generation() > 0 || reborn_sid.index() != full_sid.index());
    assert!(runtime.surfaces.get(full_sid).is_none());
}

#[test]
fn failed_pre_create_plugin_loads_burn_zero_generations_and_emit_zero_events() {
    let mut surfaces = crate::SurfaceOrchestrator::new();
    let mut host = NvePluginHost::new();

    for i in 0..100u32 {
        assert_eq!(
            host.register_in_process(
                &mut surfaces,
                format!("missing-surface-{i}"),
                broken_missing_surface_get_interface,
                PluginRequirement::SURFACE_ONLY,
            ),
            Err(PluginError::MissingSurfaceInterface)
        );
        assert_eq!(
            host.register_in_process(
                &mut surfaces,
                format!("missing-component-{i}"),
                surface_only_get_interface,
                PluginRequirement::EMBEDDED_COMPONENT,
            ),
            Err(PluginError::MissingRequiredInterface {
                interface_name: "NVE_IFACE_COMPONENT",
                interface_id: NVE_IFACE_COMPONENT,
            })
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
            let missing_path = std::env::temp_dir()
                .join(format!("nve_nonexistent_plugin_test_{i}.so"));
            assert!(matches!(
                host.load_dynamic(
                    &mut surfaces,
                    format!("missing-dylib-{i}"),
                    &missing_path,
                    PluginRequirement::SURFACE_ONLY,
                ),
                Err(PluginError::Io(_) | PluginError::LibraryLoadFailed(_))
            ));
        }
    }

    assert_eq!(host.active_count(), 0);
    assert_eq!(surfaces.active_count(), 0);
    assert!(
        surfaces.drain_events().is_empty(),
        "300 failed pre-create loads must emit zero surface events"
    );

    // Next valid plugin mount must still receive slot 1, generation 0 (0 burned generations!).
    let pid = host
        .register_in_process(
            &mut surfaces,
            "first-valid",
            surface_only_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    assert_eq!(host.get(pid).unwrap().surface_id(), SurfaceId::new(1, 0));

    // Exhausting u32 PluginId fails cleanly before allocating a surface.
    host.set_next_plugin_id_for_test(u32::MAX);
    let _ = surfaces.drain_events();
    assert_eq!(
        host.register_in_process(
            &mut surfaces,
            "overflow-id",
            surface_only_get_interface,
            PluginRequirement::SURFACE_ONLY,
        ),
        Err(PluginError::PluginIdsExhausted)
    );
    assert!(surfaces.drain_events().is_empty());
    assert_eq!(surfaces.active_count(), 1);
}

