use std::ffi::c_void;

use nve_abi::{
    CommandBufferBuilder, CommandDecoder, ComponentKind, NveAppHandle, NveBufferSlice,
    NveComponentApi, NveDevApi, NveDirectEvent, NveEventBatch, NveGestureDisposition,
    NveIntrinsicSize, NveLayoutApi, NveMapEntry, NvePropsRef, NveSizeConstraints, NveSurfaceApi,
    NveValue, SurfaceId, NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
    ROOT_NODE_ID,
};

use super::*;

// ---- Test Plugin V1: full capability plugin (SURFACE + LAYOUT + COMPONENT + DEV) ----
struct MockPluginState {
    surface_id: SurfaceId,
    version_tag: &'static str,
    counter: u32,
    frame_buf: Vec<u8>,
    state_buf: Vec<u8>,
    out_events: Vec<NveDirectEvent>,
}

unsafe extern "C" fn p1_create(surface_id: u32) -> NveAppHandle {
    Box::into_raw(Box::new(MockPluginState {
        surface_id,
        version_tag: "v1",
        counter: 1,
        frame_buf: Vec::new(),
        state_buf: Vec::new(),
        out_events: Vec::new(),
    })) as NveAppHandle
}

unsafe extern "C" fn p2_create(surface_id: u32) -> NveAppHandle {
    Box::into_raw(Box::new(MockPluginState {
        surface_id,
        version_tag: "v2",
        counter: 1,
        frame_buf: Vec::new(),
        state_buf: Vec::new(),
        out_events: Vec::new(),
    })) as NveAppHandle
}

unsafe extern "C" fn p_destroy(handle: NveAppHandle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle as *mut MockPluginState)) };
    }
}

unsafe extern "C" fn p_render(handle: NveAppHandle, frame_id: u64) -> NveBufferSlice {
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    let mut b = CommandBufferBuilder::new(st.surface_id, frame_id);
    b.create_node(2, ComponentKind::Text, None);
    b.set_text(2, &format!("{}:count={}", st.version_tag, st.counter));
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    st.frame_buf = b.into_bytes();
    NveBufferSlice::from_slice(&st.frame_buf)
}

unsafe extern "C" fn p_dispatch(handle: NveAppHandle, _events: NveEventBatch) -> u32 {
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

unsafe extern "C" fn p_set_props(
    handle: NveAppHandle,
    entries: *const NveMapEntry,
    count: u32,
) -> u32 {
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    let props = unsafe { NvePropsRef::from_raw_parts(entries, count) };
    if let Some(c) = props.get("counter").and_then(|v| v.as_i64()) {
        st.counter = c as u32;
        return 1;
    }
    0
}

unsafe extern "C" fn p_drain_events(handle: NveAppHandle) -> NveEventBatch {
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    st.out_events = vec![NveDirectEvent::press(st.surface_id, st.counter, 0, 0.0, 0.0)];
    NveEventBatch::from_slice(&st.out_events)
}

unsafe extern "C" fn p_hot_patch(_: NveAppHandle, _: NveBufferSlice) -> u32 {
    1
}

unsafe extern "C" fn p_snapshot(handle: NveAppHandle) -> NveBufferSlice {
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    st.state_buf = st.counter.to_le_bytes().to_vec();
    NveBufferSlice::from_slice(&st.state_buf)
}

unsafe extern "C" fn p_restore(handle: NveAppHandle, state: NveBufferSlice) {
    let st = unsafe { &mut *(handle as *mut MockPluginState) };
    let raw = unsafe { state.as_slice() };
    if raw.len() == 4 {
        st.counter = u32::from_le_bytes(raw.try_into().unwrap());
    }
}

static P1_SURFACE: NveSurfaceApi = NveSurfaceApi {
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

unsafe extern "C" fn plugin_v1_get_interface(id: u32) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &P1_SURFACE as *const _ as *const c_void,
        NVE_IFACE_LAYOUT => &P_LAYOUT as *const _ as *const c_void,
        NVE_IFACE_COMPONENT => &P_COMPONENT as *const _ as *const c_void,
        NVE_IFACE_DEV => &P_DEV as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

unsafe extern "C" fn plugin_v2_get_interface(id: u32) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &P2_SURFACE as *const _ as *const c_void,
        NVE_IFACE_LAYOUT => &P_LAYOUT as *const _ as *const c_void,
        NVE_IFACE_COMPONENT => &P_COMPONENT as *const _ as *const c_void,
        NVE_IFACE_DEV => &P_DEV as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

unsafe extern "C" fn surface_only_get_interface(id: u32) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &P1_SURFACE as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

unsafe extern "C" fn broken_missing_surface_get_interface(_id: u32) -> *const c_void {
    std::ptr::null()
}

unsafe extern "C" fn null_handle_create(_surface_id: u32) -> NveAppHandle {
    std::ptr::null_mut()
}

static BROKEN_NULL_HANDLE_SURFACE: NveSurfaceApi = NveSurfaceApi {
    create: null_handle_create,
    destroy: p_destroy,
    render: p_render,
    dispatch_events: p_dispatch,
};

unsafe extern "C" fn broken_null_handle_get_interface(id: u32) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => &BROKEN_NULL_HANDLE_SURFACE as *const _ as *const c_void,
        _ => std::ptr::null(),
    }
}

#[test]
fn capability_checks_isolated_surfaces_and_transactional_hotswap() {
    let mut host = NvePluginHost::with_surface_base(20_000);

    // 1. Reject plugin missing NVE_IFACE_SURFACE.
    assert_eq!(
        host.register_in_process(
            "bad-no-surface",
            broken_missing_surface_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::MissingSurfaceInterface)
    );

    // 2. Reject surface-only plugin when caller requires NVE_IFACE_COMPONENT.
    assert_eq!(
        host.register_in_process(
            "needs-component",
            surface_only_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT
        ),
        Err(PluginError::MissingRequiredInterface {
            interface_name: "NVE_IFACE_COMPONENT",
            interface_id: NVE_IFACE_COMPONENT,
        })
    );

    // 3. Surface-only plugin mounts cleanly under SURFACE_ONLY requirement.
    let p_min = host
        .register_in_process(
            "minimal-widget",
            surface_only_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    let min_slot = host.get(p_min).unwrap();
    assert_eq!(min_slot.surface_id(), 20_000);
    assert!(min_slot.capabilities().surface);
    assert!(!min_slot.capabilities().component);
    assert!(!min_slot.capabilities().layout);

    // 4. Full-featured plugin gets a distinct isolated SurfaceId (20_001).
    let p_full = host
        .register_in_process(
            "studio-inspector-plugin",
            plugin_v1_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    assert_eq!(host.active_count(), 2);

    let slot = host.get_mut(p_full).unwrap();
    assert_eq!(slot.surface_id(), 20_001);
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
    assert!(slot.dispatch_events(&[]).unwrap()); // counter -> 43
    let f1 = slot.render(1).unwrap().to_vec();
    let (hdr1, cmds1) = CommandDecoder::new(&f1).unwrap().decode_all().unwrap();
    assert_eq!(hdr1.surface_id, 20_001);
    assert!(format!("{cmds1:?}").contains("v1:count=43"));

    // 5. Failed hot-swap (broken plugin returning null handle or missing surface)
    // must NOT crash or destroy v1!
    assert_eq!(
        host.hot_swap_in_process(
            p_full,
            broken_null_handle_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::NullHandleReturned)
    );
    assert_eq!(
        host.hot_swap_in_process(
            p_full,
            broken_missing_surface_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::MissingSurfaceInterface)
    );
    // Original v1 slot is still live at generation 1 with counter=43!
    let still_v1 = host.get_mut(p_full).unwrap();
    assert_eq!(still_v1.generation(), 1);
    let f_after_fail = still_v1.render(2).unwrap().to_vec();
    let (_, cmds_fail) = CommandDecoder::new(&f_after_fail).unwrap().decode_all().unwrap();
    assert!(format!("{cmds_fail:?}").contains("v1:count=43"));

    // 6. Valid hot-swap v1 -> v2 preserves isolated surface_id (20_001) and counter=43!
    let gen2 = host
        .hot_swap_in_process(
            p_full,
            plugin_v2_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    assert_eq!(gen2, 2);

    let v2_slot = host.get_mut(p_full).unwrap();
    assert_eq!(v2_slot.surface_id(), 20_001);
    assert_eq!(v2_slot.generation(), 2);
    let f2 = v2_slot.render(3).unwrap().to_vec();
    let (hdr2, cmds2) = CommandDecoder::new(&f2).unwrap().decode_all().unwrap();
    assert_eq!(hdr2.surface_id, 20_001);
    assert!(format!("{cmds2:?}").contains("v2:count=43"));

    // 7. Duplicate name rejected; unload cleans up cleanly.
    assert!(matches!(
        host.register_in_process(
            "studio-inspector-plugin",
            plugin_v1_get_interface,
            PluginRequirement::SURFACE_ONLY
        ),
        Err(PluginError::DuplicatePluginName(_))
    ));
    let unloaded = host.unload(p_full).unwrap();
    assert_eq!(unloaded.generation, 2);
    assert_eq!(host.active_count(), 1);
    assert!(host.find_by_name("studio-inspector-plugin").is_none());
}
