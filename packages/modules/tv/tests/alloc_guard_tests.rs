//! Zero-heap guards for the TV input hot paths, counted per thread so parallel tests never skew
//! the result: every `map_web_tv_key` call and every warm `move_focus` / `handle_input` on a
//! 200-node grid performs exactly 0 allocations.

use core::alloc::{GlobalAlloc, Layout};
use std::alloc::System;
use std::cell::Cell;

use nve_tv::{
    FocusDirection, FocusOutcome, GamepadFrame, GamepadRepeater, NodeId, RemoteAction, RemoteInput,
    SpatialFocusEngine, SpatialFocusGroupId, SpatialFocusNode, SpatialRect, TextInputContext,
    WebKeyInput, WebTvPlatform, map_web_tv_key,
};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

fn record_alloc() {
    let _ = THREAD_ALLOCS.try_with(|n| n.set(n.get() + 1));
}

// SAFETY: every method forwards unchanged arguments to `System`, which upholds the `GlobalAlloc`
// contract; the thread-local counter is const-initialized and never allocates.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_alloc();
        // SAFETY: `layout` is forwarded unchanged from our caller to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` come from a prior `System` allocation per the caller contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_alloc();
        // SAFETY: `ptr`/`layout` come from a prior `System` allocation per the caller contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn allocations<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    (THREAD_ALLOCS.with(Cell::get) - before, out)
}

/// 20 x 10 grid in 2 groups with one explicit override per row.
fn grid_200() -> SpatialFocusEngine {
    let mut engine = SpatialFocusEngine::new().with_wrap_axis(true);
    for row in 0..10u32 {
        for col in 0..20u32 {
            let rect = SpatialRect::new(col as f32 * 120.0, row as f32 * 90.0, 110.0, 80.0);
            let mut node = SpatialFocusNode::new(NodeId::new(row * 20 + col + 1, 3), rect)
                .with_group(SpatialFocusGroupId(row / 5));
            if col == 19 {
                node = node.with_next(FocusDirection::Right, NodeId::new(row * 20 + 1, 3));
            }
            engine.register(node);
        }
    }
    engine
}

#[test]
fn counter_observes_allocations_on_this_thread() {
    let (allocs, v) = allocations(|| Vec::<u64>::with_capacity(4));
    assert_eq!((allocs, v.capacity()), (1, 4), "guard must not pass vacuously");
}

#[test]
fn warm_move_focus_and_handle_input_allocate_nothing() {
    let mut engine = grid_200();
    // Warm-up: visit both groups so group memory has a slot for each.
    for _ in 0..12 {
        let _ = engine.move_focus(FocusDirection::Down);
    }
    let path = [
        FocusDirection::Right,
        FocusDirection::Down,
        FocusDirection::Left,
        FocusDirection::Up,
    ];
    let (allocs, moved) = allocations(|| {
        let mut moved = 0usize;
        for step in 0..2_000usize {
            moved += usize::from(engine.move_focus(path[step % 4]).is_some());
            // Explicit override + wrap path (row-end Right) and the throttled dispatcher.
            let outcome = engine.handle_input(RemoteInput {
                action: RemoteAction::Move(FocusDirection::Right),
                repeat: step % 2 == 1,
                timestamp_us: step as u64 * 30_000,
            });
            moved += usize::from(matches!(outcome, FocusOutcome::Moved(_)));
        }
        moved
    });
    assert!(
        moved > 2_000,
        "moves must actually navigate (moved {moved})"
    );
    assert_eq!(
        allocs, 0,
        "warm move_focus / handle_input must not allocate"
    );
}

#[test]
fn map_web_tv_key_allocates_nothing_across_all_tiers_and_platforms() {
    let platforms = [
        WebTvPlatform::Standard,
        WebTvPlatform::Tizen,
        WebTvPlatform::WebOs,
        WebTvPlatform::AndroidTv,
        WebTvPlatform::Xbox,
        WebTvPlatform::PlayStation,
    ];
    let inputs = [
        ("ArrowLeft", "ArrowLeft", 37, TextInputContext::None),
        ("Unidentified", "", 10009, TextInputContext::None),
        ("Unidentified", "Digit7", 0, TextInputContext::None),
        ("", "", 19, TextInputContext::None),
        ("ColorF2Yellow", "", 405, TextInputContext::None),
        ("ArrowUp", "ArrowUp", 38, TextInputContext::MultiLine),
        ("é", "KeyE", 69, TextInputContext::SingleLine),
    ];
    let (allocs, mapped) = allocations(|| {
        let mut mapped = 0usize;
        for platform in platforms {
            for &(key, code, key_code, text_input) in &inputs {
                let input = WebKeyInput {
                    key,
                    code,
                    key_code,
                    text_input,
                    ..WebKeyInput::default()
                };
                mapped += usize::from(map_web_tv_key(platform, &input).is_some());
            }
        }
        mapped
    });
    assert!(
        mapped >= 4 * platforms.len(),
        "most probes must map (mapped {mapped})"
    );
    assert_eq!(allocs, 0, "map_web_tv_key must not allocate");
}

#[test]
fn gamepad_repeater_and_user_agent_detection_allocate_nothing() {
    let mut pad = GamepadRepeater::new();
    let (allocs, emitted) = allocations(|| {
        let mut emitted = 0usize;
        for frame in 0..600u64 {
            let pressed = if frame % 100 < 60 { 1 << 15 } else { 0 };
            pad.update(
                GamepadFrame {
                    pressed,
                    axis_x: 0.0,
                    axis_y: 0.9,
                },
                frame * 16_667,
                |_| {
                    emitted += 1;
                },
            );
        }
        emitted += usize::from(
            WebTvPlatform::from_user_agent("Mozilla/5.0 (SMART-TV; Linux; Tizen 6.5)")
                == WebTvPlatform::Tizen,
        );
        emitted
    });
    assert!(emitted > 12);
    assert_eq!(
        allocs, 0,
        "gamepad polling + UA detection must not allocate"
    );
}
