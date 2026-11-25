//! Universal Framework Compatibility Orchestration Layer for Native View Engine (NVE).
//!
//! Enables embedding NVE components natively inside any UI framework (React Native Fabric,
//! Flutter, SwiftUI/UIKit, Jetpack Compose, Web DOM) AND hosting foreign frameworks as lazy
//! `<Island>`s inside NVE with unified lifecycle, layout negotiation, gesture arbitration,
//! shared state channels, and GPU/native view handoff.

pub mod channel;
pub mod crdt;
pub mod error;
pub mod gesture;
pub mod handoff;
pub mod layout;
pub mod native_view;
pub mod plugin_host;
pub mod surface;
pub mod sync_delta;
pub mod sync_transport;
pub mod traits;
pub mod window;

pub use channel::{
    CHANNEL_ID_AUTH, CHANNEL_ID_LOCALE, CHANNEL_ID_NAVIGATION, CHANNEL_ID_THEME, ChannelId,
    ChannelMessage, ChannelPayload, NveChannelBus, PayloadEncoding, SubscriptionId, TOPIC_AUTH,
    TOPIC_LOCALE, TOPIC_NAVIGATION, TOPIC_THEME,
};
pub use error::InteropError;
pub use gesture::{
    ArbitrationDecision, ArbitrationReason, GestureArbitrator, GestureCapabilities,
    GestureDisposition, PointerPhase, PointerSample, ScrollAxes, ScrollEdgeState,
};
pub use handoff::{
    ComposeIslandSpec, FlutterEngineGroupConfig, FlutterIslandSpec, GpuTextureFormat,
    IslandMountPolicy, IslandSpec, NativeViewHandle, NativeViewHandleKind, RenderHandoff,
    RnIslandSpec, SharedGpuTexture, SharedGpuTextureKind, SwiftUIIslandSpec,
};
pub use layout::{
    BaselineMetrics, IntrinsicSize, LayoutNegotiator, LayoutProposal, LayoutResult,
    MeasurableSurface, SizeConstraints,
};
pub use plugin_host::{
    DEFAULT_PLUGIN_SURFACE_BASE, NvePluginHost, NvePluginSlot, PluginCapabilities, PluginError,
    PluginId, PluginMetadata, PluginRequirement, PluginSourceKind,
};
pub use surface::{
    ColorScheme, EnvironmentOverrides, FrameworkKind, LifecycleState, SafeAreaInsets,
    SurfaceDescriptor, SurfaceEnvironment, SurfaceEvent, SurfaceOrchestrator, SurfacePresentation,
    SurfaceRecord, TeardownRecord, ThemeContext,
};
pub use window::{
    ManagedWindow, NVW1_HEADER_SIZE, NVW1_MAGIC, NVW1_VERSION, WindowCodecError, WindowCommand,
    WindowDescriptor, WindowError, WindowFlags, WindowHandleId, WindowLifecycleEvent,
    WindowLifecycleState, WindowManager, WindowRect, WindowState, decode_window_commands,
    decode_window_events, encode_window_commands, encode_window_events,
};
pub use sync_delta::{
    DEFAULT_SYNC_WINDOW, DELTA_HEADER_SIZE, DeltaPacketHeader, DeltaPacketRef, FLAG_RETRANSMIT,
    MAX_DECODED_FRAME_BYTES, PacketKind, ReceiveOutcome, SYNC_DELTA_MAGIC, SYNC_DELTA_VERSION,
    SyncDeltaError, SyncDeltaSession, SyncMetrics, decode_command_op_delta, decode_xor_rle_delta,
    decode_xor_rle_delta_into, encode_command_op_delta, encode_packet, encode_xor_rle_delta,
    encode_xor_rle_delta_into, fnv1a_64, fnv1a_64_pair, write_packet_into,
};
pub use sync_transport::{
    DEFAULT_MAX_WIRE_FRAME_BYTES, DEFAULT_SCHEMA_VERSION, FRAG_SUBHEADER_SIZE,
    MAX_ALLOWED_WIRE_FRAME_BYTES, MAX_INPUT_EVENT_BYTES, MAX_UDP_DATAGRAM_BYTES,
    MAX_UDP_FRAG_CHUNK, MAX_UDP_SINGLE_PAYLOAD, PairingConfig, PairingMetrics, RejectReason,
    StreamFraming, SyncTransportError, TRANSPORT_HEADER_SIZE, TRANSPORT_MAGIC,
    TRANSPORT_PROTOCOL_VERSION, TcpDeltaClient, TcpDeltaServer, TransportError, TransportMsgKind,
    UdpDeltaClient, UdpDeltaServer, WsFrame, WsOpcode, compute_ws_accept_key,
    decode_ws_frame_slice, derive_session_id, derive_token_proof, encode_ws_frame,
    packetize_udp_datagrams,
};
pub use native_view::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodeNativeCommand, DecodeNativeProps,
    DecodedWebElementProps, EncodeNativeCommand, EncodeNativeProps, MAX_NATIVE_CMD_ARGS,
    MAX_NATIVE_PROP_FIELDS, NATIVE_CMD_HEADER_SIZE, NATIVE_CMD_MAGIC, NATIVE_CMD_VERSION,
    NATIVE_PROPS_HEADER_SIZE, NATIVE_PROPS_MAGIC, NATIVE_PROPS_VERSION, NATIVE_VIEW_KIND_PROP_KEY,
    NATIVE_VIEW_PROPS_PROP_KEY, NativeCommandDecoder, NativeCommandEncoder, NativePropDecoder,
    NativePropEncoder, NativeValueRef, NativeViewCodecError, NativeViewHandle as TypedNativeViewHandle,
    NativeViewHandleBinder, NativeViewRegistry, NativeViewSpec, NoCommand, QueuedNativeCommand,
    VideoPlayer, VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView,
    WebViewCommand, WebViewProps,
};
pub use traits::{
    BUILTIN_TRAIT_COUNT, BuiltinTraitSlot, BuiltinTraits, CanHover,
    ColorScheme as TraitColorScheme, ContainerSize, HasHardwareKeyboard, HighContrast, HostToolkit,
    Locale, Orientation, PlatformOs, PointerKind, ReducedMotion, ScaleFactor, SizeClass, TextScale,
    TraitEnvironment, TraitKey, TraitOverrides, TraitResolver, TraitStore, UiDensity,
    VerticalSizeClass, ViewportChangeMask,
};

use nve_abi::SurfaceId;

/// Unified facade coordinating [`SurfaceOrchestrator`], [`LayoutNegotiator`],
/// [`GestureArbitrator`], and [`NveChannelBus`] so teardown and arbitration stay
/// strictly synchronized with zero leaked subscriptions or gesture locks.
#[derive(Default)]
pub struct InteropRuntime {
    pub surfaces: SurfaceOrchestrator,
    pub layout: LayoutNegotiator,
    pub gestures: GestureArbitrator,
    pub channels: NveChannelBus,
    pub windows: WindowManager,
}

impl InteropRuntime {
    pub fn new() -> Self {
        Self {
            surfaces: SurfaceOrchestrator::new(),
            layout: LayoutNegotiator::new(),
            gestures: GestureArbitrator::new(),
            channels: NveChannelBus::new(),
            windows: WindowManager::new(),
        }
    }

    /// Arbitrates a pointer event starting at `innermost_surface` by resolving its live
    /// ancestor chain directly from [`SurfaceOrchestrator`].
    pub fn arbitrate_pointer(
        &mut self,
        innermost_surface: SurfaceId,
        sample: PointerSample,
    ) -> Result<ArbitrationDecision, InteropError> {
        let chain = self.surfaces.ancestor_chain(innermost_surface)?;
        Ok(self.gestures.arbitrate(&chain, sample))
    }

    /// Negotiates layout for `surface_id` using its inherited display scale factor,
    /// updating the surface's recorded viewport size upon success.
    pub fn negotiate_layout<S: MeasurableSurface + ?Sized>(
        &mut self,
        surface_id: SurfaceId,
        surface: &S,
        constraints: SizeConstraints,
    ) -> Result<LayoutResult, InteropError> {
        let scale_factor = self
            .surfaces
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .scale_factor;
        let proposal = LayoutProposal::new(constraints, scale_factor);
        let result = self
            .layout
            .negotiate_surface(surface_id, surface, proposal)?;
        self.surfaces.set_viewport_size(surface_id, result.size)?;
        Ok(result)
    }

    /// Destroys `surface_id` and its entire subtree in bottom-up order, automatically cleaning up
    /// gesture capabilities/locks, cached layouts, and channel subscriptions for every torn-down surface.
    pub fn destroy_surface_cascade(
        &mut self,
        surface_id: SurfaceId,
    ) -> Result<Vec<TeardownRecord>, InteropError> {
        let torn_down = self.surfaces.destroy_surface(surface_id)?;
        for rec in &torn_down {
            self.gestures.unregister_surface(rec.surface_id);
            self.layout.invalidate_surface(rec.surface_id);
            self.channels.unsubscribe_surface(rec.surface_id);
            if let Some(handle) = self.windows.handle_for_surface(rec.surface_id) {
                let _ = self.windows.close_window(handle);
            }
        }
        Ok(torn_down)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde::{Deserialize, Serialize};

    use super::*;

    struct TextBlockSurface {
        chars: usize,
        char_width: f32,
        line_height: f32,
    }

    impl MeasurableSurface for TextBlockSurface {
        fn measure(&self, constraints: SizeConstraints) -> IntrinsicSize {
            let single_line_w = self.chars as f32 * self.char_width;
            let width = if constraints.has_bounded_width() {
                single_line_w.min(constraints.max_w).max(constraints.min_w)
            } else {
                single_line_w.max(constraints.min_w)
            };
            let chars_per_line = (width / self.char_width).floor().max(1.0) as usize;
            let lines = self.chars.div_ceil(chars_per_line).max(1);
            let height = lines as f32 * self.line_height;
            IntrinsicSize::new(width, height)
        }

        fn baseline_metrics(&self, _size: IntrinsicSize) -> BaselineMetrics {
            BaselineMetrics {
                first_baseline: Some(self.line_height * 0.8),
                last_baseline: Some(self.line_height * 0.8),
            }
        }
    }

    #[test]
    fn nested_multi_framework_tree_env_propagation_and_cascading_teardown() {
        let mut runtime = InteropRuntime::new();
        let root_env = SurfaceEnvironment {
            theme: ThemeContext::default(),
            safe_area: SafeAreaInsets::new(47.0, 0.0, 34.0, 0.0),
            scale_factor: 3.0,
            text_scale_factor: 1.0,
            locale: "en-US".into(),
            reduced_motion: false,
        };

        // Flutter Root -> NVE Card -> React Native Fabric Island -> SwiftUI Island
        let flutter_root = runtime
            .surfaces
            .register_root(FrameworkKind::Flutter, "FlutterAppRoot", root_env)
            .unwrap();

        let nve_surface = runtime
            .surfaces
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "NveCheckoutCard")
                    .with_parent(flutter_root)
                    .with_handoff(RenderHandoff::SharedGpuTexture(SharedGpuTexture {
                        width: 1170,
                        height: 600,
                        scale_factor: 3.0,
                        registry_texture_id: 42,
                        handle: SharedGpuTextureKind::IoSurface {
                            surface_id: 9001,
                            pixel_format: GpuTextureFormat::Bgra8Unorm,
                        },
                    })),
            )
            .unwrap();

        let rn_island = runtime
            .surfaces
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::ReactNativeFabric, "RnMapIsland")
                    .with_parent(nve_surface)
                    .with_island(IslandSpec::ReactNative(RnIslandSpec {
                        module_name: "MapsModule".into(),
                        component_name: "RNMapView".into(),
                        initial_props_json: r#"{"zoom":14}"#.into(),
                        fabric_enabled: true,
                        hermes_bytecode_path: None,
                        mount_policy: IslandMountPolicy::LazyOnVisible,
                    })),
            )
            .unwrap();

        // SwiftUI island overrides its own theme to stay dark even when parent switches to light.
        let swiftui_island = runtime
            .surfaces
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::SwiftUI, "ApplePaySheet")
                    .with_parent(rn_island)
                    .with_overrides(EnvironmentOverrides {
                        theme: Some(ThemeContext::dark()),
                        ..Default::default()
                    })
                    .with_island(IslandSpec::SwiftUI(SwiftUIIslandSpec {
                        view_name: "ApplePayButton".into(),
                        module_name: "PayKit".into(),
                        initial_props_json: r#"{"amountCents":4999}"#.into(),
                        use_intrinsic_sizing: true,
                        mount_policy: IslandMountPolicy::OnFirstInteraction,
                    })),
            )
            .unwrap();

        assert_eq!(runtime.surfaces.active_count(), 4);
        assert_eq!(
            runtime.surfaces.ancestor_chain(swiftui_island).unwrap(),
            vec![swiftui_island, rn_island, nve_surface, flutter_root]
        );

        // Verify inherited safe area & scale factor flowed all the way down.
        let swift_rec = runtime.surfaces.get(swiftui_island).unwrap();
        assert_eq!(swift_rec.environment.safe_area.top, 47.0);
        assert_eq!(swift_rec.environment.scale_factor, 3.0);
        assert!(swift_rec.environment.theme.is_dark());

        // Propagate dark theme & new safe area from Flutter root.
        runtime
            .surfaces
            .set_safe_area(flutter_root, SafeAreaInsets::new(59.0, 0.0, 34.0, 0.0))
            .unwrap();
        assert_eq!(
            runtime.surfaces.get(rn_island).unwrap().environment.safe_area.top,
            59.0
        );
        assert_eq!(
            runtime
                .surfaces
                .get(swiftui_island)
                .unwrap()
                .environment
                .safe_area
                .top,
            59.0
        );

        // Propagate lifecycle transitions across subtree.
        runtime
            .surfaces
            .propagate_lifecycle(flutter_root, LifecycleState::Resumed)
            .unwrap();
        assert_eq!(
            runtime.surfaces.get(rn_island).unwrap().state,
            LifecycleState::Resumed
        );
        runtime
            .surfaces
            .propagate_lifecycle(nve_surface, LifecycleState::Paused)
            .unwrap();
        assert_eq!(
            runtime.surfaces.get(flutter_root).unwrap().state,
            LifecycleState::Resumed
        );
        assert_eq!(
            runtime.surfaces.get(nve_surface).unwrap().state,
            LifecycleState::Paused
        );
        assert_eq!(
            runtime.surfaces.get(swiftui_island).unwrap().state,
            LifecycleState::Paused
        );

        // Subscribe each child to a channel & attach teardown hooks to assert bottom-up order.
        let hook_order = Arc::new(Mutex::new(Vec::new()));
        for id in [nve_surface, rn_island, swiftui_island] {
            let log = Arc::clone(&hook_order);
            runtime
                .surfaces
                .register_teardown_hook(id, move |rec| {
                    log.lock().unwrap().push(rec.surface_id);
                })
                .unwrap();
            runtime
                .channels
                .subscribe(TOPIC_THEME, Some(id), false, |_| {})
                .unwrap();
        }
        assert_eq!(runtime.channels.subscriber_count(TOPIC_THEME), 3);

        // Cascading teardown of NVE subtree must destroy SwiftUI -> RN -> NVE, leaving Flutter root intact.
        let torn = runtime.destroy_surface_cascade(nve_surface).unwrap();
        let torn_ids: Vec<SurfaceId> = torn.iter().map(|t| t.surface_id).collect();
        assert_eq!(torn_ids, vec![swiftui_island, rn_island, nve_surface]);
        assert_eq!(*hook_order.lock().unwrap(), torn_ids);

        // Verify zero leaks: parent children cleared, subscriptions removed, active count == 1.
        assert_eq!(runtime.surfaces.active_count(), 1);
        assert!(runtime.surfaces.children(flutter_root).unwrap().is_empty());
        assert_eq!(runtime.channels.subscriber_count(TOPIC_THEME), 0);
        assert!(runtime.surfaces.get(rn_island).is_none());
    }

    #[test]
    fn reparent_cycle_detection_and_lifecycle_guards() {
        let mut orch = SurfaceOrchestrator::new();
        let a = orch
            .register_root(FrameworkKind::SwiftUI, "A", SurfaceEnvironment::default())
            .unwrap();
        let b = orch
            .register_child(a, FrameworkKind::Nve, "B")
            .unwrap();
        let c = orch
            .register_child(b, FrameworkKind::JetpackCompose, "C")
            .unwrap();

        // Reparenting A under its own descendant C must fail with InvalidParentCycle.
        assert_eq!(
            orch.reparent(a, Some(c)),
            Err(InteropError::InvalidParentCycle {
                surface: a,
                proposed_parent: c
            })
        );

        // Promoting C to a root succeeds cleanly.
        orch.reparent(c, None).unwrap();
        assert_eq!(orch.roots(), &[a, c]);
        assert!(orch.children(b).unwrap().is_empty());

        // Cannot transition back to Mounted from Resumed.
        orch.transition_state(a, LifecycleState::Resumed).unwrap();
        assert!(matches!(
            orch.transition_state(a, LifecycleState::Mounted),
            Err(InteropError::InvalidLifecycleTransition { .. })
        ));
    }

    #[test]
    fn gesture_arbitration_nested_vertical_scroll_handoff_and_orthogonal_axis() {
        let mut runtime = InteropRuntime::new();
        // Outer Flutter vertical ListView -> Middle NVE horizontal carousel -> Inner RN vertical feed
        let outer_flutter = runtime
            .surfaces
            .register_root(
                FrameworkKind::Flutter,
                "OuterListView",
                SurfaceEnvironment::default(),
            )
            .unwrap();
        let middle_carousel = runtime
            .surfaces
            .register_child(outer_flutter, FrameworkKind::Nve, "MiddleCarousel")
            .unwrap();
        let inner_rn_feed = runtime
            .surfaces
            .register_child(
                middle_carousel,
                FrameworkKind::ReactNativeFabric,
                "InnerVerticalFeed",
            )
            .unwrap();

        runtime.gestures.set_capabilities(
            outer_flutter,
            GestureCapabilities::vertical_scroll(FrameworkKind::Flutter),
        );
        runtime.gestures.set_capabilities(
            middle_carousel,
            GestureCapabilities::horizontal_scroll(FrameworkKind::Nve),
        );
        runtime.gestures.set_capabilities(
            inner_rn_feed,
            GestureCapabilities::vertical_scroll(FrameworkKind::ReactNativeFabric),
        );

        // 1. Vertical drag when inner feed can scroll -> inner claims.
        let down = runtime
            .arbitrate_pointer(
                inner_rn_feed,
                PointerSample {
                    pointer_id: 1,
                    phase: PointerPhase::Down,
                    x: 100.0,
                    y: 200.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 1000,
                },
            )
            .unwrap();
        assert_eq!(down.winner, Some(inner_rn_feed));

        let move_inner = runtime
            .arbitrate_pointer(
                inner_rn_feed,
                PointerSample {
                    pointer_id: 1,
                    phase: PointerPhase::Move,
                    x: 100.0,
                    y: 170.0,
                    delta_x: 0.0,
                    delta_y: -30.0,
                    timestamp_us: 2000,
                },
            )
            .unwrap();
        assert_eq!(move_inner.innermost_disposition, GestureDisposition::Claim);
        assert_eq!(move_inner.winner, Some(inner_rn_feed));
        assert_eq!(move_inner.reason, ArbitrationReason::ScrollAxisClaimed);

        // 2. Inner feed hits bottom edge -> seamlessly defers vertical drag to outer Flutter ListView
        // and emits cancellation for inner RN feed!
        runtime.gestures.update_scroll_edges(
            inner_rn_feed,
            ScrollEdgeState {
                at_bottom: true,
                ..Default::default()
            },
        );
        let handoff = runtime
            .arbitrate_pointer(
                inner_rn_feed,
                PointerSample {
                    pointer_id: 1,
                    phase: PointerPhase::Move,
                    x: 100.0,
                    y: 140.0,
                    delta_x: 0.0,
                    delta_y: -30.0,
                    timestamp_us: 3000,
                },
            )
            .unwrap();
        assert_eq!(
            handoff.innermost_disposition,
            GestureDisposition::DeferToParent
        );
        assert_eq!(handoff.winner, Some(outer_flutter));
        assert_eq!(handoff.cancelled_surfaces, vec![inner_rn_feed]);
        assert_eq!(
            handoff.reason,
            ArbitrationReason::ScrollBoundaryDeferredToParent
        );

        let end = runtime
            .arbitrate_pointer(
                inner_rn_feed,
                PointerSample {
                    pointer_id: 1,
                    phase: PointerPhase::Up,
                    x: 100.0,
                    y: 140.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 4000,
                },
            )
            .unwrap();
        assert_eq!(end.reason, ArbitrationReason::PointerEnded);
        assert_eq!(runtime.gestures.active_pointer_count(), 0);

        // 3. Horizontal swipe inside inner vertical feed -> orthogonal pass-through to middle NVE carousel!
        runtime
            .arbitrate_pointer(
                inner_rn_feed,
                PointerSample {
                    pointer_id: 2,
                    phase: PointerPhase::Down,
                    x: 200.0,
                    y: 200.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 5000,
                },
            )
            .unwrap();
        let horiz = runtime
            .arbitrate_pointer(
                inner_rn_feed,
                PointerSample {
                    pointer_id: 2,
                    phase: PointerPhase::Move,
                    x: 160.0,
                    y: 202.0,
                    delta_x: -40.0,
                    delta_y: 2.0,
                    timestamp_us: 6000,
                },
            )
            .unwrap();
        assert_eq!(horiz.innermost_disposition, GestureDisposition::PassThrough);
        assert_eq!(horiz.winner, Some(middle_carousel));
        assert_eq!(horiz.cancelled_surfaces, vec![inner_rn_feed]);
        assert_eq!(horiz.reason, ArbitrationReason::OrthogonalAxisPassThrough);
    }

    #[test]
    fn channel_bus_zero_copy_pubsub_retained_state_and_unsub() {
        let mut bus = NveChannelBus::new();

        // Well-known channels have deterministic C-friendly IDs.
        assert_eq!(bus.channel_id(TOPIC_THEME), Some(CHANNEL_ID_THEME));
        assert_eq!(bus.channel_id(TOPIC_LOCALE), Some(CHANNEL_ID_LOCALE));
        assert_eq!(bus.channel_id(TOPIC_AUTH), Some(CHANNEL_ID_AUTH));
        assert_eq!(bus.channel_id(TOPIC_NAVIGATION), Some(CHANNEL_ID_NAVIGATION));

        // Publish retained auth state BEFORE a late subscriber joins.
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct AuthState {
            user_id: String,
            authenticated: bool,
        }
        let sent = AuthState {
            user_id: "usr_42".into(),
            authenticated: true,
        };
        let delivered = bus.publish_json(TOPIC_AUTH, None, &sent, true).unwrap();
        assert_eq!(delivered, 0);

        let received_a = Arc::new(Mutex::new(Vec::<ChannelMessage>::new()));
        let received_b = Arc::new(Mutex::new(Vec::<ChannelMessage>::new()));

        let ra = Arc::clone(&received_a);
        let sub_a = bus
            .subscribe(TOPIC_AUTH, Some(10), true, move |msg| {
                ra.lock().unwrap().push(msg.clone());
            })
            .unwrap();

        // Late subscriber with replay_retained=true receives the snapshot immediately.
        assert_eq!(received_a.lock().unwrap().len(), 1);
        let decoded: AuthState = received_a.lock().unwrap()[0].decode_json().unwrap();
        assert_eq!(decoded, sent);

        // Zero-copy binary broadcast across multiple subscribers on a custom topic.
        let custom_id = bus.ensure_channel("metrics.frame_timing").unwrap();
        assert!(custom_id >= 100);
        assert_eq!(bus.topic_for(custom_id), Some("metrics.frame_timing"));

        let ra2 = Arc::clone(&received_a);
        let rb2 = Arc::clone(&received_b);
        let sub_custom_a = bus
            .subscribe_by_id(custom_id, Some(10), false, move |m| {
                ra2.lock().unwrap().push(m.clone());
            })
            .unwrap();
        let _sub_custom_b = bus
            .subscribe_by_id(custom_id, Some(20), false, move |m| {
                rb2.lock().unwrap().push(m.clone());
            })
            .unwrap();

        let payload = ChannelPayload::from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let count = bus
            .publish_by_id(custom_id, Some(1), PayloadEncoding::Binary, payload.clone(), false)
            .unwrap();
        assert_eq!(count, 2);

        // Both subscribers point to the exact same Arc allocation (zero-copy!).
        let msg_a = received_a.lock().unwrap().last().unwrap().clone();
        let msg_b = received_b.lock().unwrap().last().unwrap().clone();
        assert!(msg_a.payload.ptr_eq(&payload));
        assert!(msg_b.payload.ptr_eq(&payload));
        assert!(msg_a.payload.ptr_eq(&msg_b.payload));

        // Unsubscribing stops further delivery.
        bus.unsubscribe(sub_custom_a).unwrap();
        bus.unsubscribe(sub_a).unwrap();
        let count_after = bus
            .publish_bytes("metrics.frame_timing", Some(1), payload, false)
            .unwrap();
        assert_eq!(count_after, 1);

        // Unsubscribe by surface cleans up remaining subscriber for surface 20.
        assert_eq!(bus.unsubscribe_surface(20), 1);
        assert_eq!(bus.subscriber_count("metrics.frame_timing"), 0);
    }

    #[test]
    fn intrinsic_layout_negotiation_clamping_caching_and_invalidation() {
        let mut runtime = InteropRuntime::new();
        let surf_id = runtime
            .surfaces
            .register_root(
                FrameworkKind::Nve,
                "TextCard",
                SurfaceEnvironment {
                    scale_factor: 2.0,
                    ..Default::default()
                },
            )
            .unwrap();

        let text = TextBlockSurface {
            chars: 25,
            char_width: 10.0,
            line_height: 20.0,
        };

        // Unbounded width: 25 * 10 = 250w x 20h
        let res_unbounded = runtime
            .negotiate_layout(surf_id, &text, SizeConstraints::UNBOUNDED)
            .unwrap();
        assert_eq!(res_unbounded.size, IntrinsicSize::new(250.0, 20.0));
        assert!(!res_unbounded.clipped);
        assert_eq!(res_unbounded.baseline.first_baseline, Some(16.0));

        // Bounded width = 100 -> 10 chars/line -> 3 lines -> 100w x 60h
        let constrained = SizeConstraints::loose(100.0, 200.0);
        let res_wrapped = runtime
            .negotiate_layout(surf_id, &text, constrained)
            .unwrap();
        assert_eq!(res_wrapped.size, IntrinsicSize::new(100.0, 60.0));
        assert_eq!(
            runtime.surfaces.get(surf_id).unwrap().viewport_size,
            IntrinsicSize::new(100.0, 60.0)
        );

        // Repeated identical negotiation hits cache.
        let misses_before = runtime.layout.cache_misses();
        let _ = runtime
            .negotiate_layout(surf_id, &text, constrained)
            .unwrap();
        assert_eq!(runtime.layout.cache_misses(), misses_before);
        assert_eq!(runtime.layout.cache_hits(), 1);

        // Tight height shorter than content marks clipped = true and clamps strictly.
        let clipped = runtime
            .negotiate_layout(surf_id, &text, SizeConstraints::tight(100.0, 30.0))
            .unwrap();
        assert_eq!(clipped.size, IntrinsicSize::new(100.0, 30.0));
        assert!(clipped.clipped);

        // Invalid constraints (min > max or NaN) fail cleanly.
        assert!(matches!(
            runtime.negotiate_layout(surf_id, &text, SizeConstraints::new(200.0, 0.0, 100.0, 100.0)),
            Err(InteropError::InvalidConstraints(_))
        ));
        assert!(matches!(
            SizeConstraints::new(f32::NAN, 0.0, 100.0, 100.0).validate(),
            Err(InteropError::InvalidConstraints(_))
        ));

        // Cache invalidation forces recompute.
        runtime.layout.invalidate_surface(surf_id);
        let _ = runtime
            .negotiate_layout(surf_id, &text, constrained)
            .unwrap();
        assert_eq!(runtime.layout.cache_misses(), misses_before + 2);
    }

    #[test]
    fn island_spec_validation_and_flutter_engine_group_serialization() {
        let flutter_island = IslandSpec::Flutter(FlutterIslandSpec {
            widget_name: "AnalyticsChartWidget".into(),
            engine_group: FlutterEngineGroupConfig {
                group_id: "main_flutter_pool".into(),
                entrypoint: "analyticsMain".into(),
                initial_route: Some("/charts/revenue".into()),
                dart_entrypoint_args: vec!["--high-refresh".into()],
                share_isolate_group: true,
                warm_spare_engine: true,
            },
            prefer_texture_compositing: true,
            initial_props_json: r#"{"range":"30d"}"#.into(),
            mount_policy: IslandMountPolicy::LazyOnVisible,
        });
        assert!(flutter_island.validate().is_ok());
        assert_eq!(flutter_island.framework(), FrameworkKind::Flutter);

        let json = serde_json::to_string(&flutter_island).unwrap();
        let roundtripped: IslandSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, flutter_island);

        let bad_island = IslandSpec::JetpackCompose(ComposeIslandSpec {
            composable_fqn: "dev.nve.Demo".into(),
            initial_props_json: "{not valid json".into(),
            use_hardware_layer: true,
            mount_policy: IslandMountPolicy::EagerOnCreate,
        });
        assert!(matches!(
            bad_island.validate(),
            Err(InteropError::InvalidIslandSpec(_))
        ));
    }

    #[test]
    fn gesture_edge_cases_slop_local_overscroll_multitouch_and_teardown() {
        let mut runtime = InteropRuntime::new();
        let outer = runtime
            .surfaces
            .register_root(FrameworkKind::SwiftUI, "Outer", SurfaceEnvironment::default())
            .unwrap();
        let inner = runtime
            .surfaces
            .register_child(outer, FrameworkKind::Nve, "Inner")
            .unwrap();

        let mut inner_caps = GestureCapabilities::vertical_scroll(FrameworkKind::Nve);
        inner_caps.nested_scroll_enabled = false;
        inner_caps.scroll_edges.at_top = true;
        runtime.gestures.set_capabilities(outer, GestureCapabilities::vertical_scroll(FrameworkKind::SwiftUI));
        runtime.gestures.set_capabilities(inner, inner_caps);

        // Down + tiny move below 8px touch slop stays within touch slop.
        runtime
            .arbitrate_pointer(
                inner,
                PointerSample {
                    pointer_id: 10,
                    phase: PointerPhase::Down,
                    x: 50.0,
                    y: 50.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 100,
                },
            )
            .unwrap();
        let tiny = runtime
            .arbitrate_pointer(
                inner,
                PointerSample {
                    pointer_id: 10,
                    phase: PointerPhase::Move,
                    x: 50.0,
                    y: 53.0,
                    delta_x: 0.0,
                    delta_y: 3.0,
                    timestamp_us: 200,
                },
            )
            .unwrap();
        assert_eq!(tiny.reason, ArbitrationReason::WithinTouchSlop);

        // Dragging down at top edge with nested_scroll_enabled=false consumes locally (overscroll bounce).
        let local_bounce = runtime
            .arbitrate_pointer(
                inner,
                PointerSample {
                    pointer_id: 10,
                    phase: PointerPhase::Move,
                    x: 50.0,
                    y: 80.0,
                    delta_x: 0.0,
                    delta_y: 27.0,
                    timestamp_us: 300,
                },
            )
            .unwrap();
        assert_eq!(local_bounce.innermost_disposition, GestureDisposition::Claim);
        assert_eq!(local_bounce.winner, Some(inner));
        assert_eq!(
            local_bounce.reason,
            ArbitrationReason::ScrollBoundaryConsumedLocally
        );

        // Second simultaneous pointer tracks independently.
        runtime
            .arbitrate_pointer(
                outer,
                PointerSample {
                    pointer_id: 11,
                    phase: PointerPhase::Down,
                    x: 10.0,
                    y: 10.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 350,
                },
            )
            .unwrap();
        assert_eq!(runtime.gestures.active_pointer_count(), 2);

        // Destroying inner surface mid-gesture removes its gesture state cleanly.
        runtime.destroy_surface_cascade(inner).unwrap();
        let end_outer = runtime
            .arbitrate_pointer(
                outer,
                PointerSample {
                    pointer_id: 11,
                    phase: PointerPhase::Up,
                    x: 10.0,
                    y: 10.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 400,
                },
            )
            .unwrap();
        assert_eq!(end_outer.reason, ArbitrationReason::PointerEnded);
    }

    #[test]
    fn constraints_deflate_swiftui_proposal_and_handoff_variants() {
        let c = SizeConstraints::new(40.0, 20.0, 200.0, f32::INFINITY);
        let deflated = c.deflate(16.0, 30.0);
        assert_eq!(deflated.min_w, 24.0);
        assert_eq!(deflated.min_h, 0.0);
        assert_eq!(deflated.max_w, 184.0);
        assert!(!deflated.has_bounded_height());

        let proposal = LayoutProposal::from_swiftui_proposal(Some(320.0), None, 3.0);
        assert_eq!(proposal.constraints.max_w, 320.0);
        assert!(!proposal.constraints.has_bounded_height());
        let snapped = IntrinsicSize::new(100.1, 44.2).snap_to_pixel_grid(2.0);
        assert_eq!(snapped, IntrinsicSize::new(100.5, 44.5));

        let handoffs = vec![
            RenderHandoff::SharedGpuTexture(SharedGpuTexture {
                width: 800,
                height: 600,
                scale_factor: 2.0,
                registry_texture_id: 1,
                handle: SharedGpuTextureKind::CvPixelBuffer {
                    ptr: 0x1000,
                    pixel_format: GpuTextureFormat::Bgra8Unorm,
                },
            }),
            RenderHandoff::SharedGpuTexture(SharedGpuTexture {
                width: 1080,
                height: 1920,
                scale_factor: 3.0,
                registry_texture_id: 2,
                handle: SharedGpuTextureKind::AHardwareBuffer {
                    ptr: 0x2000,
                    format: 1,
                },
            }),
            RenderHandoff::SharedGpuTexture(SharedGpuTexture {
                width: 1920,
                height: 1080,
                scale_factor: 1.5,
                registry_texture_id: 3,
                handle: SharedGpuTextureKind::DxgiHandle {
                    handle: 0x3000,
                    adapter_luid: 0xAABB_CCDD,
                },
            }),
            RenderHandoff::SharedGpuTexture(SharedGpuTexture {
                width: 1280,
                height: 720,
                scale_factor: 1.0,
                registry_texture_id: 4,
                handle: SharedGpuTextureKind::DmaBuf {
                    fd: 17,
                    stride: 5120,
                    offset: 0,
                    modifier: 0,
                },
            }),
            RenderHandoff::NativeView(NativeViewHandle {
                framework: FrameworkKind::WebDom,
                handle: NativeViewHandleKind::DomElement {
                    element_id: "nve-root".into(),
                },
            }),
        ];
        let json = serde_json::to_string(&handoffs).unwrap();
        let decoded: Vec<RenderHandoff> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, handoffs);
    }

    #[test]
    fn surface_registration_and_channel_error_guards() {
        let mut runtime = InteropRuntime::new();
        let id = runtime
            .surfaces
            .register_surface(
                SurfaceDescriptor::new(FrameworkKind::Nve, "Root").with_id(7),
            )
            .unwrap();
        assert_eq!(id, 7);
        assert_eq!(
            runtime
                .surfaces
                .register_surface(SurfaceDescriptor::new(FrameworkKind::Nve, "Dup").with_id(7)),
            Err(InteropError::DuplicateSurface(7))
        );
        assert_eq!(
            runtime
                .surfaces
                .register_child(999, FrameworkKind::Flutter, "Orphan"),
            Err(InteropError::ParentNotFound(999))
        );

        assert!(matches!(
            runtime.channels.ensure_channel("   "),
            Err(InteropError::InvalidTopic(_))
        ));
        assert_eq!(
            runtime.channels.unsubscribe(4242),
            Err(InteropError::SubscriptionNotFound(4242))
        );
        assert_eq!(
            runtime.channels.publish_by_id(
                999,
                None,
                PayloadEncoding::Binary,
                ChannelPayload::from_slice(&[]),
                false,
            ),
            Err(InteropError::ChannelNotFound(999))
        );
    }
}
