//! Cascade teardown, `GenerationExhausted`, canonicalizing event drain, and `Destroyed`
//! lifecycle propagation tests for `NvePluginHost` / `InteropRuntime` (split from
//! `lifecycle_tests.rs` to stay under `[INV:max-650-lines]`).

use std::ffi::c_void;

use nve_abi::{
    InterfaceId, NveAppHandle, NveComponentApi, NveDirectEvent, NveEventBatch, SurfaceId,
    NVE_IFACE_COMPONENT, NVE_IFACE_SURFACE,
};
use nve_store::NodeId;

use super::lifecycle_tests::{
    p_set_props, plugin_v1_get_interface, plugin_v2_get_interface, surface_only_get_interface,
    MockPluginState, P1_SURFACE,
};
use super::*;

#[test]
fn unload_plugin_cascades_subsystems_and_child_plugins() {
    let mut runtime = crate::InteropRuntime::new();

    let parent_pid = runtime
        .register_plugin_in_process(
            "parent-plugin",
            plugin_v1_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    let child_pid = runtime
        .register_plugin_in_process(
            "child-plugin",
            surface_only_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    let parent_sid = runtime.plugins().get(parent_pid).unwrap().surface_id();
    let child_sid = runtime.plugins().get(child_pid).unwrap().surface_id();

    runtime.reparent_surface(child_sid, Some(parent_sid)).unwrap();
    #[cfg(feature = "gesture")]
    {
        runtime.gestures.set_capabilities(
            parent_sid,
            crate::GestureCapabilities::vertical_scroll(crate::FrameworkKind::Nve),
        );
        runtime.gestures.set_capabilities(
            child_sid,
            crate::GestureCapabilities::vertical_scroll(crate::FrameworkKind::Nve),
        );
        let down = crate::PointerSample {
            pointer_id: 7,
            phase: crate::PointerPhase::Down,
            x: 10.0,
            y: 10.0,
            delta_x: 0.0,
            delta_y: 0.0,
            timestamp_us: nve_abi::TimestampUs(100),
        };
        let _ = runtime.arbitrate_pointer(child_sid, down).unwrap();
        assert_eq!(runtime.gestures.active_pointer_count(), 1);
    }
    #[cfg(feature = "channel")]
    {
        let _ = runtime
            .channels
            .subscribe("plugin.topic", Some(parent_sid), false, |_| {})
            .unwrap();
        let _ = runtime
            .channels
            .subscribe("plugin.topic", Some(child_sid), false, |_| {})
            .unwrap();
        assert_eq!(runtime.channels.subscriber_count("plugin.topic"), 2);
    }

    let meta = runtime.unload_plugin(parent_pid).unwrap();
    assert_eq!(meta.surface_id, parent_sid);
    assert_eq!(runtime.plugins().active_count(), 0);
    assert_eq!(runtime.surfaces().active_count(), 0);
    assert!(runtime.plugins().find_by_surface(parent_sid).is_none());
    assert!(runtime.plugins().find_by_surface(child_sid).is_none());
    #[cfg(feature = "gesture")]
    {
        assert_eq!(runtime.gestures.active_pointer_count(), 0);
        assert!(!runtime.gestures.has_surface_state(parent_sid));
        assert!(!runtime.gestures.has_surface_state(child_sid));
    }
    #[cfg(feature = "channel")]
    assert_eq!(runtime.channels.subscriber_count("plugin.topic"), 0);
}

#[test]
fn hot_swap_returns_generation_exhausted_at_u64_max() {
    let mut runtime = crate::InteropRuntime::new();
    let gen_pid = runtime
        .register_plugin_in_process(
            "gen-limit-plugin",
            plugin_v1_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    runtime
        .plugin_mut(gen_pid)
        .unwrap()
        .set_generation_for_test(u64::MAX);
    assert_eq!(
        runtime.hot_swap_plugin_in_process(
            gen_pid,
            plugin_v2_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        ),
        Err(PluginError::GenerationExhausted(gen_pid))
    );
    let _ = runtime.unload_plugin(gen_pid).unwrap();
    assert_eq!(runtime.plugins().active_count(), 0);
    assert_eq!(runtime.surfaces().active_count(), 0);
}

#[test]
fn drain_emitted_events_canonicalizes_non_canonical_c_pod_handles() {
    unsafe extern "C" fn p_drain_corrupt_events(handle: NveAppHandle) -> NveEventBatch {
        // SAFETY: `handle` was allocated by `p1_create` as `Box<MockPluginState>`.
        let st = unsafe { &mut *(handle as *mut MockPluginState) };
        st.out_events.clear();
        st.out_events.push(NveDirectEvent {
            // SAFETY: `SurfaceId` is `#[repr(transparent)]` over `u32`; simulating raw C POD bits.
            surface_id: unsafe { core::mem::transmute::<u32, SurfaceId>(0x0500_0000) },
            // SAFETY: `NodeId` is `#[repr(transparent)]` over `u32`; simulating raw C POD bits.
            node_id: unsafe { core::mem::transmute::<u32, NodeId>(0x0700_0000) },
            timestamp_us: 42,
            kind: 1,
            payload_tag: 0,
            modifiers: 0,
            event_flags: 0,
            key_code: 0,
            name: nve_abi::NveStrSlice::EMPTY,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            text: nve_abi::NveStrSlice::EMPTY,
            value: nve_abi::NveValue::null(),
        });
        NveEventBatch::from_slice(&st.out_events)
    }
    static CORRUPT_COMP: NveComponentApi = NveComponentApi {
        set_props: p_set_props,
        drain_emitted_events: p_drain_corrupt_events,
    };
    unsafe extern "C" fn corrupt_events_get_interface(id: InterfaceId) -> *const c_void {
        match id {
            NVE_IFACE_SURFACE => &P1_SURFACE as *const _ as *const c_void,
            NVE_IFACE_COMPONENT => &CORRUPT_COMP as *const _ as *const c_void,
            _ => std::ptr::null(),
        }
    }

    let mut runtime = crate::InteropRuntime::new();
    let corrupt_pid = runtime
        .register_plugin_in_process(
            "corrupt-events-plugin",
            corrupt_events_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    {
        let mut slot = runtime.plugin_mut(corrupt_pid).unwrap();
        let batch = slot.drain_emitted_events();
        assert_eq!(batch.len(), 1);
        let ev = batch.get(0).unwrap();
        assert_eq!(ev.surface_id(), SurfaceId::NULL);
        assert_eq!(ev.node_id(), NodeId::NULL);
        assert_eq!(ev.timestamp_us(), 42);
    }
    let _ = runtime.unload_plugin(corrupt_pid).unwrap();
}

#[test]
fn destroy_surface_cascade_survives_panicking_leaf_hook_and_propagates_destroyed() {
    let mut runtime = crate::InteropRuntime::new();
    let p_cas = runtime
        .register_plugin_in_process(
            "cascade-plugin",
            plugin_v1_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    let p_cas_sid = runtime.plugins().get(p_cas).unwrap().surface_id();
    let leaf_sid = runtime
        .register_child(p_cas_sid, crate::FrameworkKind::SwiftUI, "Leaf")
        .unwrap();
    #[cfg(feature = "channel")]
    let _ = runtime
        .channels
        .subscribe("cascade.topic", Some(p_cas_sid), false, |_| {})
        .unwrap();
    let root_hook_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let root_hook_flag = std::sync::Arc::clone(&root_hook_ran);
    runtime
        .on_surface_teardown(leaf_sid, |_| panic!("leaf cascade hook boom"))
        .unwrap();
    runtime
        .on_surface_teardown(p_cas_sid, move |_| {
            root_hook_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap();

    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = runtime.destroy_surface_cascade(p_cas_sid);
    }));
    assert!(res.is_err(), "panicking leaf hook must resume unwind");
    assert!(
        root_hook_ran.load(std::sync::atomic::Ordering::SeqCst),
        "parent teardown hook must still execute after leaf hook panic"
    );
    assert_eq!(runtime.plugins().active_count(), 0);
    assert_eq!(runtime.surfaces().active_count(), 0);
    #[cfg(feature = "channel")]
    assert_eq!(runtime.channels.subscriber_count("cascade.topic"), 0);

    // Single-surface `transition_surface_state(..., Destroyed)` detaches mounted plugin.
    let p_tr = runtime
        .register_plugin_in_process(
            "tr-plugin",
            plugin_v1_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    let tr_sid = runtime.plugins().get(p_tr).unwrap().surface_id();
    runtime
        .transition_surface_state(tr_sid, crate::LifecycleState::Destroyed)
        .unwrap();
    assert_eq!(runtime.plugins().active_count(), 0);
    assert_eq!(runtime.surfaces().active_count(), 0);

    // Subtree `propagate_lifecycle(parent, Destroyed)` tears down post-order (child before parent)
    // and detaches plugins, channels, and gestures across the entire subtree.
    let prop_parent = runtime
        .register_plugin_in_process(
            "prop-parent-plugin",
            plugin_v1_get_interface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap();
    let prop_child = runtime
        .register_plugin_in_process(
            "prop-child-plugin",
            surface_only_get_interface,
            PluginRequirement::SURFACE_ONLY,
        )
        .unwrap();
    let pp_sid = runtime.plugins().get(prop_parent).unwrap().surface_id();
    let pc_sid = runtime.plugins().get(prop_child).unwrap().surface_id();
    runtime.reparent_surface(pc_sid, Some(pp_sid)).unwrap();
    #[cfg(feature = "channel")]
    let _ = runtime
        .channels
        .subscribe("prop.topic", Some(pc_sid), false, |_| {})
        .unwrap();

    let destroyed = runtime
        .propagate_lifecycle(pp_sid, crate::LifecycleState::Destroyed)
        .unwrap();
    assert_eq!(destroyed, vec![pc_sid, pp_sid], "Destroyed propagates post-order");
    assert_eq!(runtime.plugins().active_count(), 0);
    assert_eq!(runtime.surfaces().active_count(), 0);
    #[cfg(feature = "channel")]
    assert_eq!(runtime.channels.subscriber_count("prop.topic"), 0);
}
