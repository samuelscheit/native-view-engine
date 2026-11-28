#![cfg(not(target_arch = "wasm32"))]
//! Zero-allocation heap-guard suite for `nve-docs` hot paths.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

use dioxus_core::Element;
use nve_dioxus::prelude::*;
use nve_docs::{
    callout_tone, default_component_registry, docs_theme_config, snapshot_appearance, syntax_color,
    DocsAppState, DocsRoute, DocsSiteConfig, DocsViewport, PlaygroundMeta, PlaygroundRegistry,
    ThemeModeCell,
};
use nve_host_services::MemoryHostServices;
use nve_interop::ContrastLevel;
use nve_mdx::{compile_site_with_validation, CalloutKind, CompileOptions, SiteSource, TokenKind};
use nve_system_env::{ColorScheme, ThemeSnapshot};
use nve_ui::{Appearance, ThemeMode};

struct CountingAlloc;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
}

#[inline]
fn thread_alloc_count() -> usize {
    ALLOC_COUNT.with(Cell::get)
}

#[inline]
fn measure_allocs<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = thread_alloc_count();
    let out = f();
    let after = thread_alloc_count();
    (out, after.wrapping_sub(before))
}

// SAFETY: Every allocation method forwards unchanged arguments to `System`, upholding `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn dummy_demo() -> Element {
    rsx! { view {} }
}

#[test]
fn counting_alloc_positive_probe() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (nonzero, allocs) = measure_allocs(|| black_box(vec![1u8; 64]));
    drop(nonzero);
    assert!(
        allocs >= 1,
        "positive probe failed: Vec allocation must increment CountingAlloc (got {allocs})"
    );
}

#[test]
fn theme_resolution_syntax_tokens_and_callout_tones_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let cell = ThemeModeCell::new(ThemeMode::System);
    let snapshots = [
        ThemeSnapshot {
            color_scheme: Some(ColorScheme::Dark),
            contrast: ContrastLevel::Standard,
            ..ThemeSnapshot::default()
        },
        ThemeSnapshot {
            color_scheme: Some(ColorScheme::Light),
            contrast: ContrastLevel::High,
            ..ThemeSnapshot::default()
        },
        ThemeSnapshot {
            color_scheme: None,
            contrast: ContrastLevel::Standard,
            ..ThemeSnapshot::default()
        },
    ];
    let token_kinds = [
        TokenKind::Plain,
        TokenKind::Keyword,
        TokenKind::TypeName,
        TokenKind::StringLit,
        TokenKind::Comment,
        TokenKind::Number,
        TokenKind::Function,
        TokenKind::Macro,
        TokenKind::Attribute,
        TokenKind::Punctuation,
        TokenKind::DiffAdd,
        TokenKind::DiffDel,
    ];
    let callout_kinds = [
        CalloutKind::Tip,
        CalloutKind::Info,
        CalloutKind::Warning,
        CalloutKind::Danger,
        CalloutKind::Note,
    ];

    let (checksum, allocs) = measure_allocs(|| {
        let mut acc = 0u64;
        for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            cell.set(mode);
            let loaded = cell.get();
            for snap in &snapshots {
                let appearance = snapshot_appearance(black_box(snap), true);
                let theme = docs_theme_config(loaded).resolve(appearance);
                for &tk in &token_kinds {
                    let c = syntax_color(&theme.syntax, tk);
                    acc = acc.wrapping_add(u64::from(c.r) + u64::from(c.g) + u64::from(c.b) + u64::from(c.a));
                }
                for &ck in &callout_kinds {
                    let tone = callout_tone(&theme, ck);
                    let c = tone.colors.text;
                    acc = acc.wrapping_add(
                        u64::from(c.r) + u64::from(c.g) + u64::from(c.b) + tone.label.len() as u64,
                    );
                }
            }
        }
        black_box(acc)
    });

    assert_ne!(checksum, 0);
    assert_eq!(
        allocs, 0,
        "docs theme / syntax_color / callout_tone hot paths must allocate 0 bytes (got {allocs})"
    );
}

#[test]
fn docs_state_viewport_search_cursor_and_registry_lookups_are_zero_alloc() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut src = SiteSource::new();
    src.insert(
        "index.mdx",
        "---\ntitle: \"Home\"\ndescription: \"Docs home\"\n---\n\n# Home\n\nWelcome.\n",
    );
    let (site, diags) = compile_site_with_validation(
        &src,
        &CompileOptions::default(),
        &default_component_registry(),
    );
    assert!(diags.is_empty(), "minimal site compiled with diagnostics: {diags:?}");
    let site = Arc::new(site);

    let mut playgrounds = PlaygroundRegistry::new();
    playgrounds.register(
        PlaygroundMeta::new("counter", "Counter", "fn main() {}"),
        dummy_demo,
    );
    let playgrounds = Arc::new(playgrounds);
    let host_services = Arc::new(MemoryHostServices::new());
    let mut state = DocsAppState::new(
        DocsSiteConfig::new("NVE Docs"),
        site,
        Arc::clone(&playgrounds),
        host_services,
        DocsRoute::root(),
        DocsViewport::desktop_default(),
    );

    let (ok, allocs) = measure_allocs(|| {
        state.set_viewport(390.0, 844.0);
        let mobile_label = state.viewport.mode_label();
        state.set_viewport(1280.0, 800.0);
        let desktop_label = state.viewport.mode_label();
        state.open_search();
        state.move_search_selection(1, 5);
        state.move_search_selection(-2, 5);
        let sel = state.search_selected;
        state.close_search();
        let theme = state.theme();
        let mode = state.theme_mode();
        let collapsed = state.is_folder_collapsed("guides");
        let copied = state.is_code_copied("block-0");
        let has_demo = playgrounds.contains("counter") && playgrounds.get("counter").is_some();
        let external_none = DocsRoute::parse_link_target("https://example.com").is_none()
            && DocsRoute::parse_link_target("mailto:dev@example.com").is_none()
            && DocsRoute::parse_link_target("").is_none();

        mobile_label == "mobile"
            && desktop_label == "desktop"
            && sel == 4
            && theme.is_dark()
            && mode == ThemeMode::System
            && !collapsed
            && !copied
            && has_demo
            && external_none
            && state.appearance == Appearance::DARK
    });

    assert!(ok);
    assert_eq!(
        allocs, 0,
        "DocsAppState viewport/search-cursor/theme & PlaygroundRegistry lookup must allocate 0 bytes (got {allocs})"
    );
}
