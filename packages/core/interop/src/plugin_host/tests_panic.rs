use std::ffi::c_void;

use nve_abi::{
    CommandBufferBuilder, CommandDecoder, ComponentKind, FrameId, NveDirectEvent,
    NveGestureDisposition, NveIntrinsicSize, NveMapEntry, NveSizeConstraints, SurfaceId,
    ROOT_NODE_ID,
};

use crate::error::InteropError;

use super::*;

// ---- End-to-end panicking `export_nve_app!` plugin survival verification ----
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostPanicMode {
    Healthy,
    PanicCreate,
    PanicRender,
    PanicDispatchEvents,
    PanicSetProps,
    PanicMeasure,
    PanicHitTest,
    PanicDrainEvents,
    PanicHotPatch,
    PanicSnapshot,
    PanicRestore,
    PanicDestroy,
}

thread_local! {
    static HOST_PANIC_MODE: std::cell::Cell<HostPanicMode> =
        const { std::cell::Cell::new(HostPanicMode::Healthy) };
}

fn set_host_panic_mode(mode: HostPanicMode) {
    HOST_PANIC_MODE.with(|c| c.set(mode));
}

fn get_host_panic_mode() -> HostPanicMode {
    HOST_PANIC_MODE.with(std::cell::Cell::get)
}

struct ExportedFlakyPlugin {
    surface_id: SurfaceId,
    counter: u32,
    frame_buf: Vec<u8>,
    out_events: Vec<NveDirectEvent>,
    state_buf: Vec<u8>,
}

impl Drop for ExportedFlakyPlugin {
    fn drop(&mut self) {
        if get_host_panic_mode() == HostPanicMode::PanicDestroy {
            panic!("flaky plugin exploded in destroy");
        }
    }
}

impl nve_engine::NveApp for ExportedFlakyPlugin {
    fn create(surface_id: SurfaceId) -> Self {
        if get_host_panic_mode() == HostPanicMode::PanicCreate {
            panic!("flaky plugin exploded in create");
        }
        Self {
            surface_id,
            counter: 10,
            frame_buf: Vec::new(),
            out_events: vec![NveDirectEvent::press(surface_id, 1, 0, 0.0, 0.0)],
            state_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if get_host_panic_mode() == HostPanicMode::PanicRender {
            panic!("flaky plugin exploded in render frame {frame_id}");
        }
        let mut b = CommandBufferBuilder::new(self.surface_id, frame_id);
        b.create_node(2, ComponentKind::Text, None);
        b.set_text(2, &format!("flaky:{}", self.counter));
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        self.frame_buf = b.into_bytes();
        &self.frame_buf
    }

    fn dispatch_events(&mut self, _events: nve_abi::NveEventBatchRef<'_>) -> bool {
        if get_host_panic_mode() == HostPanicMode::PanicDispatchEvents {
            panic!("flaky plugin exploded in dispatch_events");
        }
        self.counter += 1;
        true
    }

    fn measure(&mut self, _constraints: NveSizeConstraints) -> NveIntrinsicSize {
        if get_host_panic_mode() == HostPanicMode::PanicMeasure {
            panic!("flaky plugin exploded in measure");
        }
        NveIntrinsicSize {
            width: 111.0,
            height: 22.0,
        }
    }

    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        if get_host_panic_mode() == HostPanicMode::PanicHitTest {
            panic!("flaky plugin exploded in hit_test");
        }
        NveGestureDisposition::Claim
    }

    fn set_props(&mut self, _props: nve_abi::NvePropsRef<'_>) -> bool {
        if get_host_panic_mode() == HostPanicMode::PanicSetProps {
            panic!("flaky plugin exploded in set_props");
        }
        self.counter += 5;
        true
    }

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        if get_host_panic_mode() == HostPanicMode::PanicDrainEvents {
            panic!("flaky plugin exploded in drain_emitted_events");
        }
        &self.out_events
    }

    fn apply_hot_patch(&mut self, _patch: &nve_engine::HotPatchMsg) -> bool {
        if get_host_panic_mode() == HostPanicMode::PanicHotPatch {
            panic!("flaky plugin exploded in apply_hot_patch");
        }
        true
    }

    fn snapshot_state(&mut self) -> &[u8] {
        if get_host_panic_mode() == HostPanicMode::PanicSnapshot {
            panic!("flaky plugin exploded in snapshot_state");
        }
        self.state_buf = self.counter.to_le_bytes().to_vec();
        &self.state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        if get_host_panic_mode() == HostPanicMode::PanicRestore {
            panic!("flaky plugin exploded in restore_state");
        }
        if state.len() == 4 {
            self.counter = u32::from_le_bytes(state.try_into().unwrap());
        }
    }
}

struct ExportedHealthyPlugin {
    surface_id: SurfaceId,
    ticks: u32,
    frame_buf: Vec<u8>,
}

impl nve_engine::NveApp for ExportedHealthyPlugin {
    fn create(surface_id: SurfaceId) -> Self {
        Self {
            surface_id,
            ticks: 0,
            frame_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let mut b = CommandBufferBuilder::new(self.surface_id, frame_id);
        b.create_node(2, ComponentKind::Text, None);
        b.set_text(2, &format!("healthy:ticks={}", self.ticks));
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        self.frame_buf = b.into_bytes();
        &self.frame_buf
    }

    fn dispatch_events(&mut self, _events: nve_abi::NveEventBatchRef<'_>) -> bool {
        self.ticks += 1;
        true
    }
}

nve_engine::export_nve_app!(ExportedFlakyPlugin, symbol = nve_test_flaky_plugin_iface);
nve_engine::export_nve_app!(
    ExportedHealthyPlugin,
    symbol = nve_test_healthy_plugin_iface
);

unsafe extern "C" {
    fn nve_test_flaky_plugin_iface(interface_id: u32) -> *const c_void;
    fn nve_test_healthy_plugin_iface(interface_id: u32) -> *const c_void;
}

#[test]
fn export_nve_app_plugin_panics_return_interop_error_and_keep_sibling_plugins_healthy() {
    set_host_panic_mode(HostPanicMode::Healthy);
    let mut host = NvePluginHost::with_surface_base(30_000);

    // Mount healthy sibling plugin first.
    let healthy_id = host
        .register_in_process(
            "healthy-sibling",
            nve_test_healthy_plugin_iface,
            PluginRequirement::SURFACE_ONLY,
        )
        .expect("mount healthy sibling");

    // 1. Panicking `create` fails registration cleanly with InteropError::Plugin(...)
    // instead of aborting the process.
    set_host_panic_mode(HostPanicMode::PanicCreate);
    let create_err: InteropError = host
        .register_in_process(
            "flaky",
            nve_test_flaky_plugin_iface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap_err()
        .into();
    match create_err {
        InteropError::Plugin(msg) => {
            assert!(msg.contains("NveSurfaceApi::create"), "got: {msg}");
            assert!(msg.contains("flaky plugin exploded in create"), "got: {msg}");
        }
        other => panic!("expected InteropError::Plugin, got {other:?}"),
    }
    assert_eq!(host.active_count(), 1);

    // 2. Mount flaky plugin in healthy mode so we can trigger panics on each runtime method.
    set_host_panic_mode(HostPanicMode::Healthy);
    let flaky_id = host
        .register_in_process(
            "flaky",
            nve_test_flaky_plugin_iface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .expect("mount flaky plugin");
    assert_eq!(host.active_count(), 2);

    // 3. Panicking `render` returns Err(InteropError::Plugin(...)).
    set_host_panic_mode(HostPanicMode::PanicRender);
    let render_err: InteropError = host
        .get_mut(flaky_id)
        .unwrap()
        .render(7)
        .unwrap_err()
        .into();
    assert!(matches!(
        render_err,
        InteropError::Plugin(ref m) if m.contains("flaky plugin exploded in render frame 7")
    ));

    // 4. Panicking `dispatch_events`.
    set_host_panic_mode(HostPanicMode::PanicDispatchEvents);
    let direct_ev = [NveDirectEvent::press(30_001, 2, 100, 1.0, 2.0)];
    let ev_err: InteropError = host
        .get_mut(flaky_id)
        .unwrap()
        .dispatch_events(&direct_ev)
        .unwrap_err()
        .into();
    assert!(matches!(
        ev_err,
        InteropError::Plugin(ref m) if m.contains("flaky plugin exploded in dispatch_events")
    ));

    // 5. Panicking `set_props`.
    set_host_panic_mode(HostPanicMode::PanicSetProps);
    let entries = [NveMapEntry::new("k", nve_abi::NveValue::i64(1))];
    let props_err: InteropError = host
        .get_mut(flaky_id)
        .unwrap()
        .set_props(&entries)
        .unwrap_err()
        .into();
    assert!(matches!(
        props_err,
        InteropError::Plugin(ref m) if m.contains("flaky plugin exploded in set_props")
    ));

    // 6. Optional layout/dev/component methods return safe fallbacks without leaking TLS panic.
    set_host_panic_mode(HostPanicMode::PanicMeasure);
    assert_eq!(
        host.get_mut(flaky_id)
            .unwrap()
            .measure(NveSizeConstraints::loose(200.0, 100.0)),
        None
    );

    set_host_panic_mode(HostPanicMode::PanicHitTest);
    assert_eq!(
        host.get_mut(flaky_id).unwrap().hit_test(1.0, 2.0, 0),
        NveGestureDisposition::PassThrough
    );

    set_host_panic_mode(HostPanicMode::PanicDrainEvents);
    assert!(host.get_mut(flaky_id).unwrap().drain_emitted_events().is_empty());

    set_host_panic_mode(HostPanicMode::PanicHotPatch);
    let patch = nve_engine::HotPatchMsg::DevStatus { message: "x".into(), latency_ms: 0 }.to_bytes();
    assert!(!host.get_mut(flaky_id).unwrap().apply_hot_patch(&patch));

    set_host_panic_mode(HostPanicMode::PanicSnapshot);
    assert!(host.get_mut(flaky_id).unwrap().snapshot_state().is_empty());

    set_host_panic_mode(HostPanicMode::PanicRestore);
    assert!(!host.get_mut(flaky_id).unwrap().restore_state(&[1, 2, 3, 4]));

    // 7. Transactional hot-swap onto a plugin whose `restore_state` panics fails cleanly and
    // preserves the existing mounted plugin!
    set_host_panic_mode(HostPanicMode::Healthy);
    assert!(host.get_mut(flaky_id).unwrap().dispatch_events(&direct_ev).unwrap()); // counter -> 11
    set_host_panic_mode(HostPanicMode::PanicRestore);
    let swap_err = host
        .hot_swap_in_process(
            flaky_id,
            nve_test_flaky_plugin_iface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .unwrap_err();
    assert!(matches!(
        swap_err,
        PluginError::PluginPanicked(ref m) if m.contains("flaky plugin exploded in restore_state")
    ));
    set_host_panic_mode(HostPanicMode::Healthy);
    let still_live = host.get_mut(flaky_id).unwrap();
    assert_eq!(still_live.generation(), 1);
    let (_, flaky_cmds) = CommandDecoder::new(still_live.render(8).unwrap())
        .unwrap()
        .decode_all()
        .unwrap();
    assert!(format!("{flaky_cmds:?}").contains("flaky:11"));

    // 8. Healthy sibling plugin stayed 100% healthy throughout all flaky panics!
    let sibling = host.get_mut(healthy_id).unwrap();
    assert!(sibling.dispatch_events(&direct_ev).unwrap());
    assert!(sibling.dispatch_events(&direct_ev).unwrap());
    let sibling_frame = sibling.render(1).unwrap().to_vec();
    let (hdr, sibling_cmds) = CommandDecoder::new(&sibling_frame)
        .unwrap()
        .decode_all()
        .unwrap();
    assert_eq!(hdr.surface_id, 30_000);
    assert!(format!("{sibling_cmds:?}").contains("healthy:ticks=2"));

    // 9. Panicking `destroy` during `unload` survives cleanly and leaves sibling intact.
    set_host_panic_mode(HostPanicMode::PanicDestroy);
    host.unload(flaky_id).expect("unload panicking plugin");
    set_host_panic_mode(HostPanicMode::Healthy);
    assert_eq!(host.active_count(), 1);
    assert!(host.get_mut(healthy_id).unwrap().render(2).is_ok());
}
