//! Thread-local `CountingAlloc` zero-allocation steady-state guards for `nve-interop`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use nve_abi::{SurfaceId, TimestampUs};
use nve_interop::{
    EnvironmentOverrides, FrameworkKind, GestureCapabilities, InteropError, InteropRuntime,
    PointerPhase, PointerSample, SurfaceDescriptor, SurfaceEnvironment, SurfaceOrchestrator,
    ThemeContext,
};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

// SAFETY: Delegates directly to `System` while incrementing a thread-local counter via `try_with`
// so concurrent test threads never race or panic during TLS teardown.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Forwards exact `layout` to the global `System` allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forwards exact `(ptr, layout)` previously allocated by `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Forwards exact realloc parameters to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    THREAD_ALLOCS.with(|c| c.set(0));
    let out = f();
    let count = THREAD_ALLOCS.with(Cell::get);
    (count, out)
}

#[test]
fn surface_orchestrator_lookups_reparent_and_pointer_arbitration_zero_alloc_steady_state() {
    let mut runtime = InteropRuntime::new();
    let mut ids = Vec::with_capacity(8);
    let root = runtime
        .register_root(FrameworkKind::Flutter, "Root", SurfaceEnvironment::default())
        .unwrap();
    ids.push(root);
    let mut parent = root;
    for depth in 1..8 {
        let child = runtime
            .register_child(parent, FrameworkKind::Nve, format!("Child{depth}"))
            .unwrap();
        ids.push(child);
        parent = child;
    }
    let leaf = *ids.last().unwrap();

    for (i, &sid) in ids.iter().enumerate() {
        let mut caps = GestureCapabilities::vertical_scroll(FrameworkKind::Nve);
        if i > 2 {
            caps.scroll_edges.at_bottom = true;
        }
        runtime.gestures.set_capabilities(sid, caps);
    }

    // Warm one full Down -> Move (delta_y > 0 with at_top == false so leaf claims) -> Up cycle.
    for (phase, dy, ts) in [
        (PointerPhase::Down, 0.0, 100),
        (PointerPhase::Move, 30.0, 200),
        (PointerPhase::Up, 0.0, 300),
    ] {
        let _ = runtime
            .arbitrate_pointer(
                leaf,
                PointerSample {
                    pointer_id: 1,
                    phase,
                    x: 20.0,
                    y: 100.0 + dy,
                    delta_x: 0.0,
                    delta_y: dy,
                    timestamp_us: TimestampUs(ts),
                },
            )
            .unwrap();
    }

    let mut chain_buf = [SurfaceId::NULL; SurfaceOrchestrator::INLINE_ANCESTOR_CAP];

    // 1. Hot SurfaceOrchestrator reads + stack ancestor walk must perform 0 allocations.
    let (read_allocs, checksum) = measure_allocs(|| {
        let mut acc = 0usize;
        for _ in 0..200 {
            for &sid in &ids {
                acc += runtime.surfaces().get(black_box(sid)).unwrap().children.len();
                acc += runtime.surfaces().children(black_box(sid)).unwrap().len();
            }
            acc += runtime.surfaces().roots().len();
            acc += runtime.surfaces().active_count();
            let len = runtime
                .surfaces()
                .ancestor_chain_into(black_box(leaf), &mut chain_buf)
                .unwrap()
                .unwrap();
            acc += len;
        }
        acc
    });
    black_box(checksum);
    assert_eq!(
        read_allocs, 0,
        "SurfaceOrchestrator hot reads must perform 0 heap allocations (saw {read_allocs})"
    );

    // 2. InteropRuntime::arbitrate_pointer on the 8-deep leaf must claim `leaf` with 0 allocations.
    let (arb_allocs, winner) = measure_allocs(|| {
        let mut last = None;
        for step in 0..100u64 {
            let down = runtime
                .arbitrate_pointer(
                    black_box(leaf),
                    black_box(PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Down,
                        x: 10.0,
                        y: 10.0,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        timestamp_us: TimestampUs(step * 1000 + 1),
                    }),
                )
                .unwrap();
            let mv = runtime
                .arbitrate_pointer(
                    black_box(leaf),
                    black_box(PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Move,
                        x: 10.0,
                        y: 40.0,
                        delta_x: 0.0,
                        delta_y: 30.0,
                        timestamp_us: TimestampUs(step * 1000 + 2),
                    }),
                )
                .unwrap();
            let up = runtime
                .arbitrate_pointer(
                    black_box(leaf),
                    black_box(PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Up,
                        x: 10.0,
                        y: 40.0,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        timestamp_us: TimestampUs(step * 1000 + 3),
                    }),
                )
                .unwrap();
            last = mv.winner.or(down.winner).or(up.winner);
        }
        last
    });
    assert_eq!(winner, Some(leaf));
    assert_eq!(
        arb_allocs, 0,
        "InteropRuntime::arbitrate_pointer on 8-deep leaf must perform 0 heap allocations (saw {arb_allocs})"
    );

    // 3. Cycle-rejection path in `reparent` must walk ancestors with 0 heap allocations.
    let (reparent_allocs, reparent_res) = measure_allocs(|| {
        runtime.reparent_surface(black_box(root), Some(black_box(leaf)))
    });
    assert_eq!(
        reparent_res,
        Err(InteropError::InvalidParentCycle {
            surface: root,
            proposed_parent: leaf,
        })
    );
    assert_eq!(
        reparent_allocs, 0,
        "reparent cycle detection must perform 0 heap allocations (saw {reparent_allocs})"
    );
}

#[test]
fn inline_ancestor_cap_boundary_zero_alloc_at_32_and_heap_fallback_above_32() {
    let cap = SurfaceOrchestrator::INLINE_ANCESTOR_CAP;
    let mut runtime = InteropRuntime::new();
    let root = runtime
        .register_root(FrameworkKind::Flutter, "D0", SurfaceEnvironment::default())
        .unwrap();
    runtime
        .gestures
        .set_capabilities(root, GestureCapabilities::vertical_scroll(FrameworkKind::Nve));

    let mut cur = root;
    let mut at_cap = root;
    for depth in 1..(cap + 3) {
        cur = runtime
            .register_child(cur, FrameworkKind::Nve, format!("D{depth}"))
            .unwrap();
        runtime
            .gestures
            .set_capabilities(cur, GestureCapabilities::vertical_scroll(FrameworkKind::Nve));
        if depth == cap - 1 {
            at_cap = cur;
        }
    }
    let beyond_cap = cur;

    // Warm pointer state at depth == INLINE_ANCESTOR_CAP (32).
    let _ = runtime
        .arbitrate_pointer(
            at_cap,
            PointerSample {
                pointer_id: 7,
                phase: PointerPhase::Down,
                x: 5.0,
                y: 5.0,
                delta_x: 0.0,
                delta_y: 0.0,
                timestamp_us: TimestampUs(1),
            },
        )
        .unwrap();
    let _ = runtime
        .arbitrate_pointer(
            at_cap,
            PointerSample {
                pointer_id: 7,
                phase: PointerPhase::Up,
                x: 5.0,
                y: 5.0,
                delta_x: 0.0,
                delta_y: 0.0,
                timestamp_us: TimestampUs(2),
            },
        )
        .unwrap();

    // At depth == cap (32): ancestor_chain_into fits inline AND arbitrate_pointer does 0 allocs.
    let mut buf = [SurfaceId::NULL; SurfaceOrchestrator::INLINE_ANCESTOR_CAP];
    let (cap_allocs, (len, move_winner)) = measure_allocs(|| {
        let len = runtime
            .surfaces()
            .ancestor_chain_into(black_box(at_cap), &mut buf)
            .unwrap()
            .unwrap();
        let _ = runtime
            .arbitrate_pointer(
                black_box(at_cap),
                black_box(PointerSample {
                    pointer_id: 7,
                    phase: PointerPhase::Down,
                    x: 5.0,
                    y: 5.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: TimestampUs(10),
                }),
            )
            .unwrap();
        let mv = runtime
            .arbitrate_pointer(
                black_box(at_cap),
                black_box(PointerSample {
                    pointer_id: 7,
                    phase: PointerPhase::Move,
                    x: 5.0,
                    y: 35.0,
                    delta_x: 0.0,
                    delta_y: 30.0,
                    timestamp_us: TimestampUs(20),
                }),
            )
            .unwrap();
        let _ = runtime
            .arbitrate_pointer(
                black_box(at_cap),
                black_box(PointerSample {
                    pointer_id: 7,
                    phase: PointerPhase::Up,
                    x: 5.0,
                    y: 35.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: TimestampUs(30),
                }),
            )
            .unwrap();
        (len, mv.winner)
    });
    assert_eq!(len, cap);
    assert_eq!(move_winner, Some(at_cap));
    assert_eq!(
        cap_allocs, 0,
        "depth == INLINE_ANCESTOR_CAP ({cap}) must stay zero-alloc (saw {cap_allocs})"
    );

    // At depth > cap (35): ancestor_chain_into signals overflow (`Ok(None)`) and arbitrate_pointer
    // falls back cleanly to heap `ancestor_chain`.
    assert_eq!(
        runtime
            .surfaces()
            .ancestor_chain_into(beyond_cap, &mut buf)
            .unwrap(),
        None
    );
    assert_eq!(
        runtime.surfaces().ancestor_chain(beyond_cap).unwrap().len(),
        cap + 3
    );
    let _ = runtime
        .arbitrate_pointer(
            beyond_cap,
            PointerSample {
                pointer_id: 8,
                phase: PointerPhase::Down,
                x: 5.0,
                y: 5.0,
                delta_x: 0.0,
                delta_y: 0.0,
                timestamp_us: TimestampUs(40),
            },
        )
        .unwrap();
    let mv_deep = runtime
        .arbitrate_pointer(
            beyond_cap,
            PointerSample {
                pointer_id: 8,
                phase: PointerPhase::Move,
                x: 5.0,
                y: 35.0,
                delta_x: 0.0,
                delta_y: 30.0,
                timestamp_us: TimestampUs(50),
            },
        )
        .unwrap();
    assert_eq!(mv_deep.winner, Some(beyond_cap));
}

#[test]
fn environment_update_pruning_and_override_child_zero_alloc_steady_state() {
    let mut rich_theme = ThemeContext::dark();
    for i in 0..16 {
        rich_theme
            .tokens
            .insert(format!("token.{i}"), format!("#{:06x}", i * 0x1111));
    }

    let mut orch = SurfaceOrchestrator::new();
    let root_env = SurfaceEnvironment {
        locale: String::new(),
        scale_factor: 1.0,
        ..Default::default()
    };
    let root = orch
        .register_root(FrameworkKind::Nve, "root", root_env.clone())
        .unwrap();
    let ov_child = orch
        .register_surface(
            SurfaceDescriptor::new(FrameworkKind::Nve, "ov_child")
                .with_parent(root)
                .with_overrides(EnvironmentOverrides {
                    theme: Some(rich_theme.clone()),
                    scale_factor: Some(2.0),
                    locale: Some("ja-JP".into()),
                    ..Default::default()
                }),
        )
        .unwrap();
    let _grandchild = orch
        .register_child(ov_child, FrameworkKind::Nve, "gc")
        .unwrap();
    let ov_child_env = orch.get(ov_child).unwrap().environment.clone();

    // Warm event/stack capacity once before measuring steady-state allocations.
    let _ = orch.set_scale_factor(root, 1.5).unwrap();
    let _ = orch.set_scale_factor(root, 1.0).unwrap();
    let _ = orch.drain_events();

    // Positive probe ensures CountingAlloc actively detects heap allocations.
    let (probe_allocs, _) = measure_allocs(|| black_box(vec![1u8; 64]));
    assert!(probe_allocs >= 1, "CountingAlloc positive probe must detect allocation");

    // 1. Unchanged-root update_environment performs 0 heap allocations.
    let unchanged_root_env = root_env.clone();
    let (root_noop_allocs, root_upd) = measure_allocs(|| {
        orch.update_environment(black_box(root), unchanged_root_env)
            .unwrap()
    });
    assert!(root_upd.is_empty());
    assert_eq!(
        root_noop_allocs, 0,
        "unchanged-root update_environment must perform 0 allocations (saw {root_noop_allocs})"
    );

    // 2. Unchanged override-child update_environment performs 0 heap allocations (no parent or BTreeMap clone!).
    let (child_noop_allocs, child_upd) =
        measure_allocs(|| orch.update_environment(black_box(ov_child), ov_child_env).unwrap());
    assert!(child_upd.is_empty());
    assert_eq!(
        child_noop_allocs, 0,
        "unchanged override-child update_environment must perform 0 allocations (saw {child_noop_allocs})"
    );

    // 3. Root scale change blocked by override child: compare against a standalone root with no
    //    children so pruned override-child + rich BTreeMap theme + grandchild do 0 extra allocations.
    let mut solo = SurfaceOrchestrator::new();
    let solo_root = solo
        .register_root(FrameworkKind::Nve, "solo", root_env)
        .unwrap();
    let _ = solo.set_scale_factor(solo_root, 1.5).unwrap();
    let _ = solo.set_scale_factor(solo_root, 1.0).unwrap();
    let _ = solo.drain_events();

    let (solo_allocs, solo_upd) =
        measure_allocs(|| solo.set_scale_factor(black_box(solo_root), 2.5).unwrap());
    let (pruned_allocs, pruned_upd) =
        measure_allocs(|| orch.set_scale_factor(black_box(root), 2.5).unwrap());
    assert_eq!(solo_upd, vec![solo_root]);
    assert_eq!(pruned_upd, vec![root]);
    assert_eq!(
        pruned_allocs, solo_allocs,
        "pruned override child + rich BTreeMap theme + grandchild must perform 0 extra allocations (pruned={pruned_allocs}, solo={solo_allocs})"
    );
}
