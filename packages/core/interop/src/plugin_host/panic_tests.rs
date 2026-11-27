use std::ffi::c_void;

use nve_abi::{
    EventBatchBuf, FrameId, InterfaceId, NveGestureDisposition, NveIntrinsicSize, NveMapEntry,
    NveSizeConstraints, SurfaceId, TimestampUs, ValidatedEvents,
};
use nve_components::tables::TextContent;
use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::error::InteropError;

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
    _surface_id: SurfaceId,
    counter: u32,
    frame_buf: Vec<u8>,
    out_events: EventBatchBuf,
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
        let mut out_events = EventBatchBuf::new();
        out_events.push_press(surface_id, NodeId::ROOT, TimestampUs(0), 0.0, 0.0);
        Self {
            _surface_id: surface_id,
            counter: 10,
            frame_buf: Vec::new(),
            out_events,
            state_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if get_host_panic_mode() == HostPanicMode::PanicRender {
            panic!("flaky plugin exploded in render frame {frame_id}");
        }
        self.frame_buf = encode_text_frame(&format!("flaky:{}", self.counter));
        &self.frame_buf
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
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

    fn drain_emitted_events(&mut self) -> ValidatedEvents<'_> {
        if get_host_panic_mode() == HostPanicMode::PanicDrainEvents {
            panic!("flaky plugin exploded in drain_emitted_events");
        }
        self.out_events.as_validated()
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
    _surface_id: SurfaceId,
    ticks: u32,
    frame_buf: Vec<u8>,
}

impl nve_engine::NveApp for ExportedHealthyPlugin {
    fn create(surface_id: SurfaceId) -> Self {
        Self {
            _surface_id: surface_id,
            ticks: 0,
            frame_buf: Vec::new(),
        }
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        self.frame_buf = encode_text_frame(&format!("healthy:ticks={}", self.ticks));
        &self.frame_buf
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
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
    fn nve_test_flaky_plugin_iface(interface_id: InterfaceId) -> *const c_void;
    fn nve_test_healthy_plugin_iface(interface_id: InterfaceId) -> *const c_void;
}

#[test]
fn export_nve_app_plugin_panics_return_interop_error_and_keep_sibling_plugins_healthy() {
    set_host_panic_mode(HostPanicMode::Healthy);
    let mut runtime = crate::InteropRuntime::new();

    // Mount healthy sibling plugin first.
    let healthy_id = runtime
        .register_plugin_in_process(
            "healthy-sibling",
            nve_test_healthy_plugin_iface,
            PluginRequirement::SURFACE_ONLY,
        )
        .expect("mount healthy sibling");

    // 1. Panicking `create` fails registration cleanly with InteropError::Plugin(...)
    // instead of aborting the process.
    set_host_panic_mode(HostPanicMode::PanicCreate);
    let create_err: InteropError = runtime
        .register_plugin_in_process(
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
    assert_eq!(runtime.plugins().active_count(), 1);
    assert_eq!(runtime.surfaces().active_count(), 1);

    // 2. Mount flaky plugin in healthy mode so we can trigger panics on each runtime method.
    set_host_panic_mode(HostPanicMode::Healthy);
    let flaky_id = runtime
        .register_plugin_in_process(
            "flaky",
            nve_test_flaky_plugin_iface,
            PluginRequirement::EMBEDDED_COMPONENT,
        )
        .expect("mount flaky plugin");
    assert_eq!(runtime.plugins().active_count(), 2);
    assert_eq!(runtime.surfaces().active_count(), 2);

    // 3. Panicking `render` returns Err(InteropError::Plugin(...)).
    set_host_panic_mode(HostPanicMode::PanicRender);
    let render_err: InteropError = runtime
        .plugin_mut(flaky_id)
        .unwrap()
        .render(FrameId(7))
        .unwrap_err()
        .into();
    assert!(matches!(
        render_err,
        InteropError::Plugin(ref m) if m.contains("flaky plugin exploded in render frame 7")
    ));

    // 4. Panicking `dispatch_events`.
    let flaky_sid = runtime.plugins().get(flaky_id).unwrap().surface_id();
    let healthy_sid = runtime.plugins().get(healthy_id).unwrap().surface_id();
    set_host_panic_mode(HostPanicMode::PanicDispatchEvents);
    let mut ev_buf = EventBatchBuf::new();
    ev_buf.push_press(flaky_sid, NodeId::new(2, 0), TimestampUs(100), 1.0, 2.0);
    let ev_err: InteropError = runtime
        .plugin_mut(flaky_id)
        .unwrap()
        .dispatch_events(ev_buf.as_validated())
        .unwrap_err()
        .into();
    assert!(matches!(
        ev_err,
        InteropError::Plugin(ref m) if m.contains("flaky plugin exploded in dispatch_events")
    ));

    // 5. Panicking `set_props`.
    set_host_panic_mode(HostPanicMode::PanicSetProps);
    let entries = [NveMapEntry::new("k", nve_abi::NveValue::i64(1))];
    let props_err: InteropError = runtime
        .plugin_mut(flaky_id)
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
        runtime
            .plugin_mut(flaky_id)
            .unwrap()
            .measure(NveSizeConstraints::loose(200.0, 100.0)),
        None
    );

    set_host_panic_mode(HostPanicMode::PanicHitTest);
    assert_eq!(
        runtime.plugin_mut(flaky_id).unwrap().hit_test(1.0, 2.0, 0),
        NveGestureDisposition::PassThrough
    );

    set_host_panic_mode(HostPanicMode::PanicDrainEvents);
    assert!(runtime.plugin_mut(flaky_id).unwrap().drain_emitted_events().is_empty());

    set_host_panic_mode(HostPanicMode::PanicHotPatch);
    let patch = nve_engine::HotPatchMsg::DevStatus { message: "x".into(), latency_ms: 0 }
        .to_bytes()
        .unwrap();
    assert!(!runtime.plugin_mut(flaky_id).unwrap().apply_hot_patch(&patch));

    set_host_panic_mode(HostPanicMode::PanicSnapshot);
    assert!(runtime.plugin_mut(flaky_id).unwrap().snapshot_state().is_empty());

    set_host_panic_mode(HostPanicMode::PanicRestore);
    assert!(!runtime.plugin_mut(flaky_id).unwrap().restore_state(&[1, 2, 3, 4]));

    // 7. Transactional hot-swap onto a plugin whose `restore_state` panics fails cleanly and
    // preserves the existing mounted plugin!
    set_host_panic_mode(HostPanicMode::Healthy);
    assert!(runtime.plugin_mut(flaky_id).unwrap().dispatch_events(ev_buf.as_validated()).unwrap()); // counter -> 11
    set_host_panic_mode(HostPanicMode::PanicRestore);
    let swap_err = runtime
        .hot_swap_plugin_in_process(
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
    let mut still_live = runtime.plugin_mut(flaky_id).unwrap();
    assert_eq!(still_live.generation(), 1);
    assert_eq!(decode_first_text(still_live.render(FrameId(8)).unwrap()), "flaky:11");

    // 8. Healthy sibling plugin stayed 100% healthy throughout all flaky panics!
    let mut sibling = runtime.plugin_mut(healthy_id).unwrap();
    assert!(sibling.dispatch_events(ev_buf.as_validated()).unwrap());
    assert!(sibling.dispatch_events(ev_buf.as_validated()).unwrap());
    let sibling_frame = sibling.render(FrameId(1)).unwrap().to_vec();
    assert_eq!(sibling.surface_id(), healthy_sid);
    assert_eq!(decode_first_text(&sibling_frame), "healthy:ticks=2");

    // 9. Panicking `destroy` during `unload_plugin` survives cleanly and leaves sibling intact.
    set_host_panic_mode(HostPanicMode::PanicDestroy);
    runtime
        .unload_plugin(flaky_id)
        .expect("unload panicking plugin");
    set_host_panic_mode(HostPanicMode::Healthy);
    assert_eq!(runtime.plugins().active_count(), 1);
    assert_eq!(runtime.surfaces().active_count(), 1);
    assert!(runtime.plugin_mut(healthy_id).unwrap().render(FrameId(2)).is_ok());
}
