//! Zero-allocation guard for steady-state theme resolution and lookup ([INV:bench-and-alloc-guards]):
//! preset lookup, OS-following resolution, runtime accent re-derivation, `use_theme()` context
//! reads during re-renders, and the `Hex` `style:` bridge writing into a reused buffer.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write as _;
use std::hint::black_box;

use dioxus_core::{Element, VirtualDom};
use dioxus_core_macro::{component, rsx};
use dioxus_signals::{Signal, WritableExt};
use nve_abi::{ColorScheme, ContrastLevel};
use nve_ui::{
    use_theme, Accent, AppBrand, Appearance, ColorHex, Polarity, Theme,
    ThemeConfig, ThemeProvider,
};

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
fn theme_resolution_accent_override_and_hex_bridge_allocate_nothing() {
    let appearances = [
        Appearance::LIGHT,
        Appearance::DARK,
        Appearance { scheme: ColorScheme::Light, contrast: ContrastLevel::High },
        Appearance { scheme: ColorScheme::Dark, contrast: ContrastLevel::High },
    ];
    let mut buf = String::with_capacity(64);
    let (allocs, checksum) = allocs_during(|| {
        let mut sum = 0u32;
        for _ in 0..200 {
            for brand in AppBrand::ALL {
                for os in appearances {
                    let theme = black_box(ThemeConfig::new(brand)).resolve(black_box(os));
                    let custom = theme.with_accent(black_box(Accent::VIOLET));
                    sum = sum.wrapping_add(custom.status.accent.solid.to_u32());
                    buf.clear();
                    let _ = write!(buf, "color: {};", theme.text.primary.hex());
                    sum = sum.wrapping_add(buf.len() as u32);
                }
            }
            for p in Polarity::ALL {
                sum = sum.wrapping_add(Theme::preset(p, AppBrand::Docs).surface.canvas.to_u32());
            }
        }
        sum
    });
    assert!(checksum > 0);
    assert_eq!(allocs, 0, "steady-state theme resolution must not allocate");
}

thread_local! {
    static READ_ALLOCS: Cell<usize> = const { Cell::new(usize::MAX) };
    static RENDERS: Cell<usize> = const { Cell::new(0) };
}

#[component]
fn Reader() -> Element {
    // The first read registers this scope as a subscriber (one-time setup per render).
    let first = use_theme();
    let (allocs, theme) = allocs_during(|| {
        let mut last = first;
        for _ in 0..1_000 {
            last = black_box(use_theme());
        }
        last
    });
    READ_ALLOCS.with(|c| c.set(allocs));
    RENDERS.with(|c| c.set(c.get() + 1));
    let _ = black_box(theme.surface.canvas);
    rsx! {}
}

thread_local! {
    static DARK: Cell<Option<Signal<bool>>> = const { Cell::new(None) };
}

#[component]
fn ThemedApp() -> Element {
    let dark = nve_dioxus::prelude::use_signal(|| false);
    DARK.with(|d| d.set(Some(dark)));
    let theme = if dark() { Theme::DARK } else { Theme::LIGHT };
    rsx! {
        ThemeProvider { theme, Reader {} }
    }
}

#[test]
fn use_theme_context_reads_allocate_nothing_across_rerenders() {
    let mut dom = VirtualDom::new(ThemedApp);
    dom.rebuild_in_place();
    assert_eq!(RENDERS.with(Cell::get), 1);
    assert_eq!(READ_ALLOCS.with(Cell::get), 0, "1,000 use_theme() reads must not allocate");

    // A theme switch re-renders the subscribed reader; its reads stay allocation-free.
    let dark = DARK.with(Cell::get).expect("signal registered");
    dom.in_runtime(|| {
        let mut dark = dark;
        dark.set(true);
    });
    dom.render_immediate_to_vec();
    // The provider's context write during its render is delivered on the next event pass.
    dom.process_events();
    dom.render_immediate_to_vec();
    assert_eq!(RENDERS.with(Cell::get), 2, "theme switch re-renders the reader once");
    assert_eq!(READ_ALLOCS.with(Cell::get), 0);
}
