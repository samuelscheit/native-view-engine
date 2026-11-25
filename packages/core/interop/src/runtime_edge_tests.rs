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

#[cfg(feature = "channel")]
#[test]
fn surface_registration_and_channel_error_guards() {
    let mut runtime = InteropRuntime::new();
    let id = runtime
        .surfaces
        .register_surface(SurfaceDescriptor::new(FrameworkKind::Nve, "Root").with_id(7))
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
