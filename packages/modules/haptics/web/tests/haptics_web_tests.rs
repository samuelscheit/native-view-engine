//! Zero-allocation `CountingAlloc` and pattern/dispatch tests for `nve-haptics-web`.

use std::alloc::{GlobalAlloc, Layout, System};

use nve_abi::SERVICE_HAPTICS;
use nve_haptics::{
    HapticsBackend, ImpactStyle, NotificationHaptic, METHOD_IMPACT, METHOD_NOTIFICATION,
    METHOD_SELECTION,
};
use nve_haptics_web::{
    impact_pattern_ms, notification_pattern_ms, WebHaptics, SELECTION_PATTERN_MS,
};
use nve_host_services::{
    CompletionSink, FfiHostServices, HostCallbackBridge, HostServiceHandler, HostServiceProvider,
    HostServiceRegistry, ServiceError,
};

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static THREAD_ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// SAFETY: Delegates directly to `System` while counting heap allocations on the tracked thread.
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

fn count_thread_allocs<F: FnOnce()>(f: F) -> usize {
    THREAD_ALLOCS.with(|c| c.set(0));
    TRACK_THREAD.with(|flag| flag.set(true));
    f();
    TRACK_THREAD.with(|flag| flag.set(false));
    THREAD_ALLOCS.with(|c| c.get())
}

#[test]
fn web_haptics_handler_and_bridge_dispatch_are_zero_alloc_and_unsupported_on_native() {
    let h = WebHaptics::new();
    let sink = CompletionSink::new();
    let reg = HostServiceRegistry::new().with_handler(WebHaptics::new());
    let bridge = HostCallbackBridge::new(reg);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());

    let impact_req = [ImpactStyle::Medium as u8];
    let notif_req = [NotificationHaptic::Success as u8];

    // Prime mutexes once before measuring the steady-state zero-alloc dispatch path.
    assert_eq!(
        ffi.invoke_sync(SERVICE_HAPTICS, METHOD_SELECTION, &[]),
        Err(ServiceError::Unsupported)
    );

    let allocs = count_thread_allocs(|| {
        assert_eq!(impact_pattern_ms(ImpactStyle::Light), &[10]);
        assert_eq!(impact_pattern_ms(ImpactStyle::Heavy), &[35]);
        assert_eq!(
            notification_pattern_ms(NotificationHaptic::Success),
            &[12, 40, 18]
        );
        assert_eq!(SELECTION_PATTERN_MS, &[6]);

        assert_eq!(h.impact(ImpactStyle::Medium), Err(ServiceError::Unsupported));
        assert_eq!(
            h.notification(NotificationHaptic::Error),
            Err(ServiceError::Unsupported)
        );
        assert_eq!(h.selection(), Err(ServiceError::Unsupported));

        // Direct `HostServiceHandler::handle` dispatch across all 3 methods (0 heap allocs).
        assert_eq!(
            h.handle(METHOD_IMPACT, 1, &impact_req, &sink),
            Err(ServiceError::Unsupported)
        );
        assert_eq!(
            h.handle(METHOD_NOTIFICATION, 2, &notif_req, &sink),
            Err(ServiceError::Unsupported)
        );
        assert_eq!(
            h.handle(METHOD_SELECTION, 3, &[], &sink),
            Err(ServiceError::Unsupported)
        );

        // Full C-ABI `FfiHostServices` -> `HostCallbackBridge` -> `HostServiceRegistry` -> `WebHaptics`.
        assert_eq!(
            ffi.invoke_sync(SERVICE_HAPTICS, METHOD_IMPACT, &impact_req),
            Err(ServiceError::Unsupported)
        );
        assert_eq!(
            ffi.invoke_sync(SERVICE_HAPTICS, METHOD_NOTIFICATION, &notif_req),
            Err(ServiceError::Unsupported)
        );
        assert_eq!(
            ffi.invoke_sync(SERVICE_HAPTICS, METHOD_SELECTION, &[]),
            Err(ServiceError::Unsupported)
        );
    });
    assert_eq!(
        allocs, 0,
        "WebHaptics handler and C-ABI dispatch must perform zero heap allocations"
    );
}
