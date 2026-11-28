//! Heap-allocation guards for the Studio hot paths ([INV:bench-and-alloc-guards]): typing into a
//! composer costs the same whatever the size of the collections beside it (copy-on-write
//! collections compared by pointer, drafts in their own domains), and resolving a view's style
//! (theme table lookup, X-Ray overlay, metrics, author and status tones) never allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

use nve_abi::{FrameId, SurfaceId};
use nve_studio_workspace::theme::{
    LayoutMetrics, XRayPalette, author_tone, studio_theme, task_status_tone,
};
use nve_studio_workspace::ui::store::{DomainMask, StudioStore};
use nve_studio_workspace::{
    AccentTheme, ChannelKind, ChatMessage, StudioConfig, TaskStatus, WorkspaceState,
    create_surface_with_store,
};
use nve_system_env::{ColorScheme, ContrastLevel};
use nve_ui::Appearance;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn record() {
    // `try_with` keeps allocation during thread teardown safe.
    let _ = TRACKING.try_with(|t| {
        if t.get() {
            let _ = ALLOCS.try_with(|a| a.set(a.get() + 1));
        }
    });
}

// SAFETY: Delegates directly to `std::alloc::System`; only a thread-local counter is touched.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record();
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn allocs_during<R>(f: impl FnOnce() -> R) -> (usize, R) {
    ALLOCS.with(|a| a.set(0));
    TRACKING.with(|t| t.set(true));
    let out = f();
    TRACKING.with(|t| t.set(false));
    (ALLOCS.with(Cell::get), out)
}

#[test]
fn counting_allocator_detects_allocations() {
    let (allocs, b) = allocs_during(|| black_box(Box::new(1u8)));
    drop(b);
    assert!(
        allocs > 0,
        "CountingAlloc self-check: a Box must be counted"
    );
}

/// Allocations of one keystroke into the design-channel composer of a mounted workspace whose
/// channel holds `messages` messages, and the domains that keystroke republished.
fn channel_keystroke(messages: usize) -> (usize, DomainMask) {
    let mut state = WorkspaceState::default();
    let seed = state.current.design_messages[0].clone();
    state.current.design_messages = (0..messages)
        .map(|i| ChatMessage {
            id: 1_000 + i as u32,
            body: format!("message {i}"),
            ..seed.clone()
        })
        .collect();
    let mut surface = create_surface_with_store(SurfaceId::ROOT, Arc::new(Mutex::new(state)));
    surface.reconcile(FrameId(1));
    let store = surface
        .in_root_scope(StudioStore::from_root)
        .expect("mounted Studio store");
    // Warm-up: the first keystroke sizes the draft buffers.
    surface.in_root_scope(|| store.update(|st| st.set_channel_draft(ChannelKind::Design, "a")));
    let (allocs, ((), mask)) = surface.in_root_scope(|| {
        allocs_during(|| store.update_masked(|st| st.set_channel_draft(ChannelKind::Design, "ab")))
    });
    (allocs, mask)
}

#[test]
fn a_composer_keystroke_costs_the_same_for_5_and_500_messages() {
    let (small, small_mask) = channel_keystroke(5);
    let (large, large_mask) = channel_keystroke(500);
    let expected = DomainMask::CHANNEL_DRAFTS.union(DomainMask::INSPECTOR);
    assert_eq!(
        small_mask, expected,
        "a keystroke republishes only the draft and the frame counter"
    );
    assert_eq!(large_mask, expected);
    assert_eq!(
        small, large,
        "keystroke allocations must not grow with the message count"
    );
    // The new draft and its projection, the inspector's action label, and the dirty marks.
    assert!(small <= 8, "a keystroke allocated {small} times");
}

#[test]
fn resolving_a_view_style_never_allocates() {
    let polarities = [
        (ColorScheme::Light, ContrastLevel::Standard),
        (ColorScheme::Dark, ContrastLevel::Standard),
        (ColorScheme::Light, ContrastLevel::High),
        (ColorScheme::Dark, ContrastLevel::High),
    ];
    let config = StudioConfig {
        xray: true,
        ..StudioConfig::default()
    };
    let (allocs, ()) = allocs_during(|| {
        for (scheme, contrast) in polarities {
            for accent in AccentTheme::ALL {
                let theme = black_box(studio_theme(
                    black_box(&config),
                    accent,
                    Appearance { scheme, contrast },
                ));
                black_box(XRayPalette::for_theme(&theme));
                black_box(LayoutMetrics::from_studio(&config));
                black_box(author_tone(&theme, black_box("Ada Lovelace")));
                black_box(task_status_tone(&theme, TaskStatus::Todo));
            }
        }
    });
    assert_eq!(
        allocs, 0,
        "Studio style resolution allocated {allocs} times"
    );
}
