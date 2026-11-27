#[cfg(feature = "channel")]
use std::sync::{Arc, Mutex};

use nve_abi::TimestampUs;

use super::*;

#[cfg(feature = "channel")]
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
                .with_parent(flutter_root),
        )
        .unwrap();

    let rn_island = runtime
        .surfaces
        .register_surface(
            SurfaceDescriptor::new(FrameworkKind::ReactNativeFabric, "RnMapSurface")
                .with_parent(nve_surface),
        )
        .unwrap();

    // SwiftUI surface overrides its own theme to stay dark even when parent switches to light.
    let swiftui_island = runtime
        .surfaces
        .register_surface(
            SurfaceDescriptor::new(FrameworkKind::SwiftUI, "ApplePaySheet")
                .with_parent(rn_island)
                .with_overrides(EnvironmentOverrides {
                    theme: Some(ThemeContext::dark()),
                    ..Default::default()
                }),
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

#[cfg(feature = "gesture")]
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
                timestamp_us: TimestampUs(1000),
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
                timestamp_us: TimestampUs(2000),
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
                timestamp_us: TimestampUs(3000),
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
                timestamp_us: TimestampUs(4000),
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
                timestamp_us: TimestampUs(5000),
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
                timestamp_us: TimestampUs(6000),
            },
        )
        .unwrap();
    assert_eq!(horiz.innermost_disposition, GestureDisposition::PassThrough);
    assert_eq!(horiz.winner, Some(middle_carousel));
    assert_eq!(horiz.cancelled_surfaces, vec![inner_rn_feed]);
    assert_eq!(horiz.reason, ArbitrationReason::OrthogonalAxisPassThrough);
}

#[cfg(feature = "channel")]
#[test]
fn channel_bus_zero_copy_pubsub_retained_state_and_unsub() {
    let mut bus = NveChannelBus::new();

    // Well-known channels have deterministic C-friendly IDs.
    assert_eq!(bus.channel_id(TOPIC_THEME), Some(CHANNEL_ID_THEME));
    assert_eq!(bus.channel_id(TOPIC_LOCALE), Some(CHANNEL_ID_LOCALE));
    assert_eq!(bus.channel_id(TOPIC_AUTH), Some(CHANNEL_ID_AUTH));
    assert_eq!(bus.channel_id(TOPIC_NAVIGATION), Some(CHANNEL_ID_NAVIGATION));

    // Publish retained binary auth state BEFORE a late subscriber joins.
    let sent_bytes = b"AUTH1:usr_42:1";
    let delivered = bus
        .publish_bytes(TOPIC_AUTH, None, ChannelPayload::from_slice(sent_bytes), true)
        .unwrap();
    assert_eq!(delivered, 0);

    let received_a = Arc::new(Mutex::new(Vec::<ChannelMessage>::new()));
    let received_b = Arc::new(Mutex::new(Vec::<ChannelMessage>::new()));

    let ra = Arc::clone(&received_a);
    let s10 = SurfaceId::new(10, 0);
    let s20 = SurfaceId::new(20, 0);
    let sub_a = bus
        .subscribe(TOPIC_AUTH, Some(s10), true, move |msg| {
            ra.lock().unwrap().push(msg.clone());
        })
        .unwrap();

    // Late subscriber with replay_retained=true receives the retained binary snapshot immediately.
    assert_eq!(received_a.lock().unwrap().len(), 1);
    assert_eq!(received_a.lock().unwrap()[0].as_bytes(), sent_bytes);

    // Zero-copy binary broadcast across multiple subscribers on a custom topic.
    let custom_id = bus.ensure_channel("metrics.frame_timing").unwrap();
    assert!(custom_id >= 100);
    assert_eq!(bus.topic_for(custom_id), Some("metrics.frame_timing"));

    let ra2 = Arc::clone(&received_a);
    let rb2 = Arc::clone(&received_b);
    let sub_custom_a = bus
        .subscribe_by_id(custom_id, Some(s10), false, move |m| {
            ra2.lock().unwrap().push(m.clone());
        })
        .unwrap();
    let _sub_custom_b = bus
        .subscribe_by_id(custom_id, Some(s20), false, move |m| {
            rb2.lock().unwrap().push(m.clone());
        })
        .unwrap();

    let payload = ChannelPayload::from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let count = bus
        .publish_by_id(
            custom_id,
            Some(SurfaceId::ROOT),
            PayloadEncoding::Binary,
            payload.clone(),
            false,
        )
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
        .publish_bytes("metrics.frame_timing", Some(SurfaceId::ROOT), payload, false)
        .unwrap();
    assert_eq!(count_after, 1);

    // Unsubscribe by surface cleans up remaining subscriber for surface 20.
    assert_eq!(bus.unsubscribe_surface(s20), 1);
    assert_eq!(bus.subscriber_count("metrics.frame_timing"), 0);
}
