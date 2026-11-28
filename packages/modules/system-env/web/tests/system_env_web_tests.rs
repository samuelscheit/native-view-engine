//! Zero-allocation `CountingAlloc` and `WebThemeWatcher` batch tests for `nve-system-env-web`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;

use nve_abi::{
    ColorScheme, ContrastLevel, NveHostEventBatchRef, NveHostEventKind, SERVICE_SYSTEM_ENV,
};
use nve_host_services::{CompletionSink, HandlerReply, HostServiceHandler};
use nve_system_env::{
    unpack_locale_ffi, SystemEnvBackend, ThemeSnapshot, EVENT_LOCALE_CHANGED, EVENT_THEME_CHANGED,
    METHOD_GET_THEME,
};
use nve_system_env_web::{
    locale_from_browser_fields, theme_from_media_flags, WebSystemEnv, WebThemeWatcher,
};

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static THREAD_ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// SAFETY: Delegates directly to `System` while counting allocations on the tracked test thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK_THREAD.try_with(|flag| {
            if flag.get() {
                let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get() + 1));
            }
        });
        // SAFETY: Forwarded unchanged to the global `System` allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` came from `System::alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_thread_allocs<F: FnOnce() -> R, R>(f: F) -> (R, usize) {
    THREAD_ALLOCS.with(|c| c.set(0));
    TRACK_THREAD.with(|flag| flag.set(true));
    let out = f();
    TRACK_THREAD.with(|flag| flag.set(false));
    (out, THREAD_ALLOCS.with(|c| c.get()))
}

#[test]
fn web_system_env_get_theme_is_zero_alloc_and_watcher_emits_batch_push() {
    let initial = theme_from_media_flags(true, true, true, false, 2.0);
    assert_eq!(initial.color_scheme, Some(ColorScheme::Dark));
    assert_eq!(initial.contrast, ContrastLevel::High);
    assert!(initial.reduced_motion);
    assert_eq!(initial.scale_factor_x100, 200);

    let locale = locale_from_browser_fields("ar-EG", Some("Africa/Cairo"), true);
    assert_eq!(locale.tag, "ar-EG");
    assert_eq!(locale.timezone, "Africa/Cairo");

    let env = Arc::new(WebSystemEnv::with_initial(initial, locale));
    let sink = CompletionSink::new();

    let (reply, allocs) = count_thread_allocs(|| env.handle(METHOD_GET_THEME, 1, &[], &sink).unwrap());
    assert_eq!(allocs, 0, "METHOD_GET_THEME must do zero heap allocs");

    let HandlerReply::Ready(buf) = reply else {
        panic!("expected Ready");
    };
    assert!(buf.is_inline());
    assert_eq!(ThemeSnapshot::from_repr_c_bytes(&buf).unwrap(), initial);

    let watcher = WebThemeWatcher::install(Arc::clone(&env), sink.clone());
    let next = theme_from_media_flags(false, false, false, true, 1.5);
    watcher.notify_theme_change(next);
    assert_eq!(env.current_theme(), next);

    let batch_bytes = sink.drain_batch().expect("theme push batch");
    let parsed = NveHostEventBatchRef::parse(&batch_bytes).unwrap();
    assert_eq!(parsed.count(), 1);
    let rec = parsed.iter().next().unwrap();
    assert_eq!(rec.kind, NveHostEventKind::Push);
    assert_eq!(rec.service_id.raw(), SERVICE_SYSTEM_ENV);
    assert_eq!(rec.method_or_event_or_channel, EVENT_THEME_CHANGED);
    assert_eq!(ThemeSnapshot::from_repr_c_bytes(rec.payload).unwrap(), next);

    // DPR scale factor update + live locale change push canonical EVENT_THEME_CHANGED & EVENT_LOCALE_CHANGED.
    let dpr3 = theme_from_media_flags(false, false, false, true, 3.0);
    watcher.notify_theme_change(dpr3);
    let de_locale = locale_from_browser_fields("de-DE", Some("Europe/Berlin"), true);
    watcher.notify_locale_change(de_locale.clone());
    assert_eq!(env.get_locale().unwrap(), de_locale);
    assert_eq!(env.get_theme().unwrap().scale_factor_x100, 300);

    let batch2 = sink.drain_batch().expect("dpr + locale push batch");
    let parsed2 = NveHostEventBatchRef::parse(&batch2).unwrap();
    let recs2: Vec<_> = parsed2.iter().collect();
    assert_eq!(recs2.len(), 2);
    assert_eq!(recs2[0].method_or_event_or_channel, EVENT_THEME_CHANGED);
    assert_eq!(
        ThemeSnapshot::from_repr_c_bytes(recs2[0].payload)
            .unwrap()
            .scale_factor_x100,
        300
    );
    assert_eq!(recs2[1].method_or_event_or_channel, EVENT_LOCALE_CHANGED);
    assert_eq!(unpack_locale_ffi(recs2[1].payload).unwrap(), de_locale);
}
