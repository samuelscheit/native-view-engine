use super::*;

#[cfg(feature = "layout")]
struct TextBlockSurface {
    chars: usize,
    char_width: f32,
    line_height: f32,
}

#[cfg(feature = "layout")]
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

#[cfg(feature = "layout")]
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

#[cfg(feature = "gesture")]
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
    runtime
        .gestures
        .set_capabilities(outer, GestureCapabilities::vertical_scroll(FrameworkKind::SwiftUI));
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

#[cfg(feature = "layout")]
#[test]
fn constraints_deflate_and_swiftui_proposal() {
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
}

#[cfg(feature = "channel")]
#[test]
fn surface_registration_and_channel_error_guards() {
    let mut runtime = InteropRuntime::new();
    let id = runtime
        .surfaces
        .register_surface(SurfaceDescriptor::new(FrameworkKind::Nve, "Root").with_id(SurfaceId(7)))
        .unwrap();
    assert_eq!(id, SurfaceId(7));
    assert_eq!(
        runtime
            .surfaces
            .register_surface(SurfaceDescriptor::new(FrameworkKind::Nve, "Dup").with_id(SurfaceId(7))),
        Err(InteropError::DuplicateSurface(SurfaceId(7)))
    );
    assert_eq!(
        runtime
            .surfaces
            .register_child(SurfaceId(999), FrameworkKind::Flutter, "Orphan"),
        Err(InteropError::ParentNotFound(SurfaceId(999)))
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

#[test]
fn surface_orchestrator_rejects_null_and_aba_stale_generational_handles() {
    let mut orch = SurfaceOrchestrator::new();

    // Index 0 (both NULL and non-zero generation) is rejected cleanly.
    assert_eq!(
        orch.register_surface(
            SurfaceDescriptor::new(FrameworkKind::Nve, "Null").with_id(SurfaceId::NULL)
        ),
        Err(InteropError::InvalidSurfaceId(SurfaceId::NULL))
    );
    let zero_idx_gen1 = SurfaceId::new(0, 1);
    assert_eq!(
        orch.register_surface(
            SurfaceDescriptor::new(FrameworkKind::Nve, "ZeroIdx").with_id(zero_idx_gen1)
        ),
        Err(InteropError::InvalidSurfaceId(zero_idx_gen1))
    );
    assert_eq!(orch.active_count(), 0);

    // Allocate, destroy, and re-allocate the same slot -> generation bumps, stale handle rejected.
    let s1 = orch
        .register_root(FrameworkKind::Nve, "First", SurfaceEnvironment::default())
        .unwrap();
    assert!(!s1.is_null());
    assert_eq!(orch.active_count(), 1);
    let destroyed = orch.destroy_surface(s1).unwrap();
    assert_eq!(destroyed.len(), 1);
    assert_eq!(destroyed[0].surface_id, s1);
    assert_eq!(orch.active_count(), 0);

    let s2 = orch
        .register_root(FrameworkKind::Nve, "Reused", SurfaceEnvironment::default())
        .unwrap();
    assert_eq!(s2.index(), s1.index());
    assert_eq!(s2.generation(), s1.generation().wrapping_add(1));
    assert_ne!(s2, s1);

    // Stale handle `s1` fails every lookup/mutation while live `s2` stays intact.
    assert!(orch.get(s1).is_none());
    assert_eq!(
        orch.transition_state(s1, LifecycleState::Resumed),
        Err(InteropError::SurfaceNotFound(s1))
    );
    assert_eq!(
        orch.destroy_surface(s1),
        Err(InteropError::SurfaceNotFound(s1))
    );
    assert_eq!(orch.get(s2).unwrap().component_name, "Reused");
    assert_eq!(orch.active_count(), 1);
}
