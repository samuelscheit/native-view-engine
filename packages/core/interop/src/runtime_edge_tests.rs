use std::collections::BTreeMap;

#[cfg(any(feature = "gesture", feature = "channel"))]
use nve_abi::TimestampUs;

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
                timestamp_us: TimestampUs(100),
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
                timestamp_us: TimestampUs(200),
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
                timestamp_us: TimestampUs(300),
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
                timestamp_us: TimestampUs(350),
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
                timestamp_us: TimestampUs(400),
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
    let root = runtime
        .surfaces
        .register_surface(SurfaceDescriptor::new(FrameworkKind::Nve, "Root"))
        .unwrap();
    assert_eq!(root, SurfaceId::ROOT);
    let missing_parent = SurfaceId::new(999, 0);
    assert_eq!(
        runtime
            .surfaces
            .register_child(missing_parent, FrameworkKind::Flutter, "Orphan"),
        Err(InteropError::ParentNotFound(missing_parent))
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
fn surface_orchestrator_and_runtime_reject_stale_and_null_handles_on_every_method() {
    let mut runtime = InteropRuntime::new();
    let first = runtime
        .surfaces
        .register_root(FrameworkKind::Nve, "First", SurfaceEnvironment::default())
        .unwrap();
    let live_other = runtime
        .surfaces
        .register_root(FrameworkKind::Flutter, "LiveOther", SurfaceEnvironment::default())
        .unwrap();
    assert_eq!(runtime.surfaces.destroy_surface(first).unwrap().len(), 1);

    // Every registration path allocates a fresh generational handle; `first` stays permanently dead.
    let reused = runtime
        .surfaces
        .register_surface(SurfaceDescriptor::new(FrameworkKind::Nve, "Reused"))
        .unwrap();
    assert_eq!(reused.index(), first.index());
    assert_eq!(reused.generation(), first.generation() + 1);
    assert_ne!(reused, first);
    let _child = runtime
        .surfaces
        .register_child(live_other, FrameworkKind::SwiftUI, "LiveChild")
        .unwrap();

    // Drain setup events before verifying that rejected stale/null calls emit zero events.
    let _ = runtime.surfaces.drain_events();

    let future_gen = SurfaceId::new(live_other.index_u32(), live_other.generation().wrapping_add(1));
    let bad_handles = [
        first,
        SurfaceId::NULL,
        SurfaceId::new(0, 1),
        future_gen,
        SurfaceId::new(99_999, 0),
    ];
    let mut chain_buf = [SurfaceId::NULL; SurfaceOrchestrator::INLINE_ANCESTOR_CAP];
    #[cfg(feature = "layout")]
    let dummy_measurable = TextBlockSurface {
        chars: 10,
        char_width: 8.0,
        line_height: 16.0,
    };

    for stale in bad_handles {
        assert!(runtime.surfaces.get(stale).is_none());
        assert_eq!(
            runtime.surfaces.register_child(stale, FrameworkKind::Nve, "Bad"),
            Err(InteropError::ParentNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.reparent(stale, None),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.reparent(live_other, Some(stale)),
            Err(InteropError::ParentNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.register_teardown_hook(stale, |_| {}),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime
                .surfaces
                .update_environment(stale, SurfaceEnvironment::default()),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.set_theme(stale, ThemeContext::dark()),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime
                .surfaces
                .set_safe_area(stale, SafeAreaInsets::new(1.0, 2.0, 3.0, 4.0)),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.set_scale_factor(stale, 2.0),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime
                .surfaces
                .transition_state(stale, LifecycleState::Resumed),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime
                .surfaces
                .transition_state(stale, LifecycleState::Destroyed),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime
                .surfaces
                .propagate_lifecycle(stale, LifecycleState::Resumed),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime
                .surfaces
                .propagate_lifecycle(stale, LifecycleState::Destroyed),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.children(stale),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.descendants(stale),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.ancestor_chain(stale),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.ancestor_chain_into(stale, &mut chain_buf),
            Err(InteropError::SurfaceNotFound(stale))
        );
        #[cfg(feature = "layout")]
        assert_eq!(
            runtime
                .surfaces
                .set_viewport_size(stale, IntrinsicSize::new(10.0, 10.0)),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.surfaces.destroy_surface(stale),
            Err(InteropError::SurfaceNotFound(stale))
        );
        assert_eq!(
            runtime.destroy_surface_cascade(stale),
            Err(InteropError::SurfaceNotFound(stale))
        );
        #[cfg(feature = "gesture")]
        assert_eq!(
            runtime.arbitrate_pointer(
                stale,
                PointerSample {
                    pointer_id: 1,
                    phase: PointerPhase::Down,
                    x: 1.0,
                    y: 1.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: TimestampUs(10),
                }
            ),
            Err(InteropError::SurfaceNotFound(stale))
        );
        #[cfg(feature = "layout")]
        assert_eq!(
            runtime.negotiate_layout(stale, &dummy_measurable, SizeConstraints::UNBOUNDED),
            Err(InteropError::SurfaceNotFound(stale))
        );
    }

    // Rejected calls must emit zero events and leave live surfaces completely untouched.
    assert!(runtime.surfaces.drain_events().is_empty());
    assert_eq!(runtime.surfaces.get(reused).unwrap().component_name, "Reused");
    assert_eq!(runtime.surfaces.active_count(), 3);
}

#[test]
fn surface_orchestrator_fifo_reuse_retires_slot_at_generation_255_and_reports_exhaustion() {
    let mut orch = SurfaceOrchestrator::new();

    // Verify FIFO free-queue ordering: destroying a then b reuses a's slot before b's.
    let a = orch
        .register_root(FrameworkKind::Nve, "A", SurfaceEnvironment::default())
        .unwrap();
    let b = orch
        .register_root(FrameworkKind::Nve, "B", SurfaceEnvironment::default())
        .unwrap();
    orch.destroy_surface(a).unwrap();
    orch.destroy_surface(b).unwrap();
    let a2 = orch
        .register_root(FrameworkKind::Nve, "A2", SurfaceEnvironment::default())
        .unwrap();
    let b2 = orch
        .register_root(FrameworkKind::Nve, "B2", SurfaceEnvironment::default())
        .unwrap();
    assert_eq!(a2.index(), a.index());
    assert_eq!(a2.generation(), 1);
    assert_eq!(b2.index(), b.index());
    assert_eq!(b2.generation(), 1);

    // Churn a fresh orchestrator through 260 create/destroy cycles on slot 1:
    // generations 0..=255 are issued once each; after gen 255 is destroyed, slot 1 is
    // permanently retired and cycle 256 allocates slot 2 (gen 0), so `first` NEVER aliases!
    let mut churn = SurfaceOrchestrator::new();
    let first = churn
        .register_root(FrameworkKind::Nve, "c0", SurfaceEnvironment::default())
        .unwrap();
    assert_eq!(first.index_u32(), 1);
    assert_eq!(first.generation(), 0);
    churn.destroy_surface(first).unwrap();

    for expected_gen in 1u8..=255 {
        let next = churn
            .register_root(FrameworkKind::Nve, "churn", SurfaceEnvironment::default())
            .unwrap();
        assert_eq!(next.index_u32(), 1);
        assert_eq!(next.generation(), expected_gen);
        assert!(churn.get(first).is_none());
        churn.destroy_surface(next).unwrap();
    }

    // Slot 1 hit generation 255 and is now retired — next allocation MUST move to slot 2!
    let after_retire = churn
        .register_root(FrameworkKind::Nve, "post_retire", SurfaceEnvironment::default())
        .unwrap();
    assert_eq!(after_retire.index_u32(), 2);
    assert_eq!(after_retire.generation(), 0);
    assert!(churn.get(first).is_none());
    assert!(churn.get(SurfaceId::new(1, 255)).is_none());
    assert_eq!(churn.active_count(), 1);

    // Slot exhaustion returns dedicated `InteropError::SurfaceSlotsExhausted`.
    let mut tiny = SurfaceOrchestrator::new();
    assert_eq!(tiny.allocate_slot_with_limit(2).unwrap(), SurfaceId::new(1, 0));
    assert_eq!(tiny.allocate_slot_with_limit(2).unwrap(), SurfaceId::new(2, 0));
    assert_eq!(
        tiny.allocate_slot_with_limit(2),
        Err(InteropError::SurfaceSlotsExhausted)
    );
}

#[test]
fn surface_orchestrator_seeded_model_property_test() {
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct ModelNode {
        parent: Option<SurfaceId>,
        children: Vec<SurfaceId>,
        has_hook: bool,
    }

    fn model_post_order(model: &BTreeMap<SurfaceId, ModelNode>, root: SurfaceId) -> Vec<SurfaceId> {
        let mut out = Vec::new();
        let mut stack = vec![(root, false)];
        while let Some((id, visited)) = stack.pop() {
            if visited {
                out.push(id);
            } else if let Some(node) = model.get(&id) {
                stack.push((id, true));
                for &child in node.children.iter().rev() {
                    stack.push((child, false));
                }
            }
        }
        out
    }

    let mut orch = SurfaceOrchestrator::new();
    let mut model: BTreeMap<SurfaceId, ModelNode> = BTreeMap::new();
    let mut model_roots: Vec<SurfaceId> = Vec::new();
    let mut dead_handles: Vec<SurfaceId> = Vec::new();
    let fired_hooks: Arc<Mutex<Vec<SurfaceId>>> = Arc::new(Mutex::new(Vec::new()));
    let mut chain_buf = [SurfaceId::NULL; SurfaceOrchestrator::INLINE_ANCESTOR_CAP];
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let next_rand = |s: &mut u64| -> usize {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s as usize
    };

    for step in 0..400usize {
        let live: Vec<SurfaceId> = model.keys().copied().collect();
        let op = next_rand(&mut rng) % 10;
        match op {
            // Register root
            0..=2 => {
                let id = orch
                    .register_root(
                        FrameworkKind::Nve,
                        format!("r{step}"),
                        SurfaceEnvironment::default(),
                    )
                    .unwrap();
                assert!(!model.contains_key(&id));
                assert!(!dead_handles.contains(&id));
                let attach_hook = step % 2 == 0;
                if attach_hook {
                    let log = Arc::clone(&fired_hooks);
                    orch.register_teardown_hook(id, move |rec| {
                        log.lock().unwrap().push(rec.surface_id);
                    })
                    .unwrap();
                }
                model.insert(
                    id,
                    ModelNode {
                        parent: None,
                        children: Vec::new(),
                        has_hook: attach_hook,
                    },
                );
                model_roots.push(id);
            }
            // Register child under a live node
            3..=5 if !live.is_empty() => {
                let parent = live[next_rand(&mut rng) % live.len()];
                let id = orch
                    .register_child(parent, FrameworkKind::Flutter, format!("c{step}"))
                    .unwrap();
                assert!(!model.contains_key(&id));
                assert!(!dead_handles.contains(&id));
                let attach_hook = step % 3 == 0;
                if attach_hook {
                    let log = Arc::clone(&fired_hooks);
                    orch.register_teardown_hook(id, move |rec| {
                        log.lock().unwrap().push(rec.surface_id);
                    })
                    .unwrap();
                }
                model.get_mut(&parent).unwrap().children.push(id);
                model.insert(
                    id,
                    ModelNode {
                        parent: Some(parent),
                        children: Vec::new(),
                        has_hook: attach_hook,
                    },
                );
            }
            // Reparent
            6..=7 if live.len() >= 2 => {
                let target = live[next_rand(&mut rng) % live.len()];
                let new_parent = if next_rand(&mut rng) % 4 == 0 {
                    None
                } else {
                    Some(live[next_rand(&mut rng) % live.len()])
                };
                let old_parent = model[&target].parent;
                // Check cycle in model.
                let mut creates_cycle = false;
                let mut cur = new_parent;
                while let Some(a) = cur {
                    if a == target {
                        creates_cycle = true;
                        break;
                    }
                    cur = model.get(&a).and_then(|n| n.parent);
                }
                let res = orch.reparent(target, new_parent);
                if old_parent == new_parent {
                    assert!(res.is_ok());
                } else if creates_cycle {
                    assert!(matches!(res, Err(InteropError::InvalidParentCycle { .. })));
                } else {
                    assert!(res.is_ok());
                    match old_parent {
                        Some(p) => model.get_mut(&p).unwrap().children.retain(|&c| c != target),
                        None => model_roots.retain(|&r| r != target),
                    }
                    match new_parent {
                        Some(p) => model.get_mut(&p).unwrap().children.push(target),
                        None => model_roots.push(target),
                    }
                    model.get_mut(&target).unwrap().parent = new_parent;
                }
            }
            // Destroy subtree — compute expected post-order independently from model!
            _ if !live.is_empty() => {
                let target = live[next_rand(&mut rng) % live.len()];
                let expected_order = model_post_order(&model, target);
                let expected_hooks: Vec<SurfaceId> = expected_order
                    .iter()
                    .copied()
                    .filter(|id| model[id].has_hook)
                    .collect();
                fired_hooks.lock().unwrap().clear();

                let torn = orch.destroy_surface(target).unwrap();
                let torn_ids: Vec<SurfaceId> = torn.iter().map(|r| r.surface_id).collect();
                assert_eq!(torn_ids, expected_order);
                assert_eq!(*fired_hooks.lock().unwrap(), expected_hooks);

                let parent = model[&target].parent;
                match parent {
                    Some(p) => model.get_mut(&p).unwrap().children.retain(|&c| c != target),
                    None => model_roots.retain(|&r| r != target),
                }
                for id in expected_order {
                    assert!(model.remove(&id).is_some());
                    dead_handles.push(id);
                }
            }
            _ => {}
        }

        // Verify structural invariants and ancestor chains against reference model after every step.
        assert_eq!(orch.active_count(), model.len());
        assert_eq!(orch.roots(), model_roots.as_slice());
        for (&id, node) in &model {
            let rec = orch.get(id).expect("live model node must exist");
            assert_eq!(rec.parent, node.parent);
            assert_eq!(rec.children, node.children);
            let chain = orch.ancestor_chain(id).unwrap();
            if chain.len() <= chain_buf.len() {
                let len = orch.ancestor_chain_into(id, &mut chain_buf).unwrap().unwrap();
                assert_eq!(&chain_buf[..len], chain.as_slice());
            } else {
                assert_eq!(orch.ancestor_chain_into(id, &mut chain_buf).unwrap(), None);
            }
        }
        // Spot-check up to 8 dead handles for permanent ABA rejection.
        for &dead in dead_handles.iter().rev().take(8) {
            assert!(orch.get(dead).is_none());
        }
    }
}

#[test]
fn destroy_surface_is_panic_safe_when_teardown_hook_panics() {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let mut orch = SurfaceOrchestrator::new();
    let root = orch
        .register_root(FrameworkKind::Nve, "Root", SurfaceEnvironment::default())
        .unwrap();
    let mid = orch
        .register_child(root, FrameworkKind::Flutter, "Mid")
        .unwrap();
    let leaf = orch
        .register_child(mid, FrameworkKind::SwiftUI, "Leaf")
        .unwrap();

    orch.register_teardown_hook(leaf, |_| {
        panic!("simulated teardown hook failure on leaf");
    })
    .unwrap();
    orch.register_teardown_hook(root, |_| {})
        .unwrap();

    let res = catch_unwind(AssertUnwindSafe(|| {
        let _ = orch.destroy_surface(root);
    }));
    assert!(res.is_err(), "panicking hook should unwind");

    // All state mutations committed BEFORE hooks ran: zero leaked slots, roots, or active_count!
    assert_eq!(orch.active_count(), 0);
    assert!(orch.roots().is_empty());
    assert!(orch.get(root).is_none());
    assert!(orch.get(mid).is_none());
    assert!(orch.get(leaf).is_none());

    // Subsequent registration cleanly reuses the freed slots with bumped generations.
    let fresh = orch
        .register_root(FrameworkKind::Nve, "Fresh", SurfaceEnvironment::default())
        .unwrap();
    assert_eq!(fresh.generation(), 1);
    assert_eq!(orch.active_count(), 1);
}
