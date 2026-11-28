//! C-ABI entry points (`include/nve_a11y_win.h`) wrapping [`WinA11yBridge`] and (on Windows)
//! `accesskit_windows::Adapter`.

use core::cell::RefCell;
use core::ffi::{c_char, c_int, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use accesskit::ActionRequest;
use nve_abi::NveSceneView;
use nve_accessibility::{
    translate_action_request, StoreEpoch, DEFAULT_SCROLL_STEP_ITEM, DEFAULT_SCROLL_STEP_PAGE,
};

use crate::action::{with_ffi_action, NveA11yWinAction};
use crate::bridge::{ResourceReader, WinA11yBridge};

/// Matches `NveA11yWinActionCallback` in `include/nve_a11y_win.h`.
pub type NveA11yWinActionCallback =
    Option<unsafe extern "C" fn(ctx: *mut c_void, action: *const NveA11yWinAction)>;

/// Matches `NveA11yResourceLookupFn` in `include/nve_a11y_win.h`.
pub type NveA11yResourceLookupFn = Option<
    unsafe extern "C" fn(
        ctx: *mut c_void,
        resource_id: u32,
        expected_kind: u64,
        out_ptr: *mut *const u8,
        out_len: *mut usize,
    ) -> c_int,
>;

const OBJID_CLIENT: i32 = -4;
const UIA_ROOT_OBJECT_ID: i32 = -25;

/// Thread-safe action bridge shared with `accesskit_windows::Adapter` (whose `ActionHandler`
/// may fire on COM MTA worker threads). Cleared atomically before teardown in
/// [`nve_a11y_win_destroy`] so lingering COM references never invoke a destroyed host context.
struct ActionBridge {
    epoch: AtomicU32,
    sink: Mutex<Option<(unsafe extern "C" fn(*mut c_void, *const NveA11yWinAction), usize)>>,
}

impl ActionBridge {
    fn new(on_action: NveA11yWinActionCallback, action_ctx: *mut c_void) -> Self {
        let sink = on_action.map(|cb| (cb, action_ctx as usize));
        Self {
            epoch: AtomicU32::new(StoreEpoch::INITIAL.raw()),
            sink: Mutex::new(sink),
        }
    }

    #[inline]
    fn set_epoch(&self, epoch: StoreEpoch) {
        self.epoch.store(epoch.raw(), Ordering::Release);
    }

    fn detach(&self) {
        let mut guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    fn dispatch(&self, request: &ActionRequest) {
        let epoch = StoreEpoch::new(self.epoch.load(Ordering::Acquire));
        let Some(resolved) = translate_action_request(
            request,
            epoch,
            DEFAULT_SCROLL_STEP_ITEM,
            DEFAULT_SCROLL_STEP_PAGE,
        ) else {
            return;
        };
        let guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cb, ctx_addr)) = *guard {
            with_ffi_action(&resolved, |payload| {
                // SAFETY: `ctx_addr` is guaranteed valid until `nve_a11y_win_destroy` calls
                // `detach()` while holding `self.sink`; `payload` lives for this callback call.
                // Per `nve_a11y_win.h`, `cb` only copies/enqueues asynchronously and never
                // re-enters `nve_a11y_win_*`, so holding `self.sink` cannot self-deadlock.
                unsafe { cb(ctx_addr as *mut c_void, payload) };
            });
        }
    }
}

#[cfg(windows)]
struct WinActionHandler(Arc<ActionBridge>);

#[cfg(windows)]
impl accesskit::ActionHandler for WinActionHandler {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            self.0.dispatch(&request);
        }));
    }
}

#[cfg(windows)]
struct StaticActivation(Option<accesskit::TreeUpdate>);

#[cfg(windows)]
impl accesskit::ActivationHandler for StaticActivation {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        self.0.take()
    }
}

struct CResourceReader {
    f: NveA11yResourceLookupFn,
    ctx: *mut c_void,
}

impl ResourceReader for CResourceReader {
    fn read(&self, resource_id: u32, expected_kind: u64) -> Option<&[u8]> {
        let f = self.f?;
        let mut ptr: *const u8 = core::ptr::null();
        let mut len: usize = 0;
        // SAFETY: `f` and `self.ctx` are borrowed for the duration of `nve_a11y_win_sync_scene`;
        // the callback writes borrowed scene-guard pointers into `ptr` and `len`.
        let ok = unsafe { f(self.ctx, resource_id, expected_kind, &mut ptr, &mut len) };
        if ok == 0 || ptr.is_null() {
            return None;
        }
        // SAFETY: callback returned non-zero guaranteeing `ptr[0..len]` is valid during sync.
        Some(unsafe { core::slice::from_raw_parts(ptr, len) })
    }
}

struct HandleInner {
    mirror: WinA11yBridge,
    #[cfg(windows)]
    adapter: accesskit_windows::Adapter,
    #[cfg(windows)]
    adapter_activated: bool,
    #[cfg(not(windows))]
    window_focused: bool,
}

/// Opaque C-ABI accessibility bridge handle.
pub struct NveA11yWinHandle {
    action_bridge: Arc<ActionBridge>,
    inner: RefCell<HandleInner>,
}

impl NveA11yWinHandle {
    /// Test helper: dispatches a synthetic [`ActionRequest`] through the exact C-ABI action path.
    pub fn dispatch_test_action(&self, request: &ActionRequest) {
        self.action_bridge.dispatch(request);
    }

    /// Test helper: returns the active [`StoreEpoch`] mirrored into the action bridge.
    pub fn current_epoch(&self) -> StoreEpoch {
        StoreEpoch::new(self.action_bridge.epoch.load(Ordering::Acquire))
    }
}

/// Returns `true` when `lparam` from `WM_GETOBJECT` targets UIA or `OBJID_CLIENT`.
#[inline]
pub const fn wants_wm_getobject(lparam: isize) -> bool {
    let objid = lparam as i32;
    objid >= 0 || objid == UIA_ROOT_OBJECT_ID || objid == OBJID_CLIENT
}

/// Creates a new Windows accessibility handle for `hwnd`.
///
/// # Safety
///
/// On Windows, `hwnd` must be a valid window handle created on the calling thread and MUST NOT be
/// called from inside `WM_GETOBJECT`. `action_ctx` must remain valid until [`nve_a11y_win_destroy`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_a11y_win_create(
    hwnd: *mut c_void,
    is_window_focused: c_int,
    on_action: NveA11yWinActionCallback,
    action_ctx: *mut c_void,
) -> *mut NveA11yWinHandle {
    #[cfg(not(windows))]
    let _ = hwnd;

    let res = catch_unwind(AssertUnwindSafe(|| {
        let action_bridge = Arc::new(ActionBridge::new(on_action, action_ctx));
        let focused = is_window_focused != 0;
        let inner = HandleInner {
            mirror: WinA11yBridge::new(),
            #[cfg(windows)]
            adapter: accesskit_windows::Adapter::new(
                accesskit_windows::HWND(hwnd),
                focused,
                WinActionHandler(Arc::clone(&action_bridge)),
            ),
            #[cfg(windows)]
            adapter_activated: false,
            #[cfg(not(windows))]
            window_focused: focused,
        };
        Box::into_raw(Box::new(NveA11yWinHandle {
            action_bridge,
            inner: RefCell::new(inner),
        }))
    }));
    res.unwrap_or(core::ptr::null_mut())
}

/// Detaches the action callback and destroys `handle` (safe no-op on null).
///
/// # Safety
///
/// `handle` must be null or a pointer returned by [`nve_a11y_win_create`] not yet destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_a11y_win_destroy(handle: *mut NveA11yWinHandle) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: caller transfers unique ownership back from `nve_a11y_win_create`.
        let boxed = unsafe { Box::from_raw(handle) };
        boxed.action_bridge.detach();
        drop(boxed);
    }));
}

/// Returns `1` when `lparam` from `WM_GETOBJECT` should be handled by the accessibility adapter.
#[unsafe(no_mangle)]
pub extern "C" fn nve_a11y_win_wants_wm_getobject(lparam: isize) -> c_int {
    c_int::from(wants_wm_getobject(lparam))
}

/// Returns `1` when accessibility mirroring is active on `handle`.
///
/// # Safety
///
/// `handle` must be null or a live pointer returned by [`nve_a11y_win_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_a11y_win_is_active(handle: *const NveA11yWinHandle) -> c_int {
    if handle.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: caller guarantees `handle` is live for the duration of this call.
        let h = unsafe { &*handle };
        h.inner
            .try_borrow()
            .map_or(0, |inner| c_int::from(inner.mirror.is_active()))
    }))
    .unwrap_or(0)
}

/// Activates accessibility mirroring on `handle`.
///
/// # Safety
///
/// `handle` must be null or a live pointer returned by [`nve_a11y_win_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_a11y_win_activate(handle: *mut NveA11yWinHandle) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: caller guarantees `handle` is live on the UI thread for this call.
        let h = unsafe { &*handle };
        if let Ok(mut inner) = h.inner.try_borrow_mut() {
            inner.mirror.activate();
        }
    }));
}

/// Mirrors `scene` when active and raises any resulting UIA events outside internal borrows.
/// Returns `1` if a tree update was emitted, `0` otherwise.
///
/// # Safety
///
/// `handle` must be null or live; when non-null, `scene` must satisfy the `NveSceneView` ABI
/// contract and `window_title_utf8` (if non-null) must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_a11y_win_sync_scene(
    handle: *mut NveA11yWinHandle,
    scene: *const NveSceneView,
    res_lookup: NveA11yResourceLookupFn,
    res_ctx: *mut c_void,
    focused_node_id: u32,
    scale_factor: f32,
    window_title_utf8: *const c_char,
) -> c_int {
    if handle.is_null() || scene.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: non-null `handle` and `scene` are valid for the duration of this call.
        let h = unsafe { &*handle };
        // SAFETY: non-null `scene` points to an initialized `NveSceneView`.
        let scene_ref = unsafe { &*scene };
        let title = if window_title_utf8.is_null() {
            ""
        } else {
            // SAFETY: caller guarantees NUL-terminated C string when non-null.
            unsafe { CStr::from_ptr(window_title_utf8) }
                .to_str()
                .unwrap_or("")
        };
        let reader = CResourceReader {
            f: res_lookup,
            ctx: res_ctx,
        };

        #[cfg(windows)]
        {
            let mut queued: Option<accesskit_windows::QueuedEvents> = None;
            let mut emitted = false;
            {
                let Ok(mut inner) = h.inner.try_borrow_mut() else {
                    return 0;
                };
                if !inner.mirror.is_active() {
                    return 0;
                }
                if !inner.adapter_activated {
                    // Pre-WM_GETOBJECT mirror pass: populate replica store without double-building.
                    // SAFETY: `scene_ref` is valid for this call.
                    let _ = unsafe {
                        inner.mirror.mirror_only(
                            scene_ref,
                            &reader,
                            focused_node_id,
                            scale_factor,
                            title,
                        )
                    };
                    h.action_bridge.set_epoch(inner.mirror.epoch());
                    return 0;
                }
                // SAFETY: `scene_ref` is valid for this call.
                let update = unsafe {
                    inner.mirror.sync_scene(
                        scene_ref,
                        &reader,
                        focused_node_id,
                        scale_factor,
                        title,
                    )
                };
                h.action_bridge.set_epoch(inner.mirror.epoch());
                if let Some(upd) = update {
                    emitted = true;
                    queued = inner.adapter.update_if_active(|| upd);
                }
            }
            if let Some(events) = queued {
                events.raise();
            }
            c_int::from(emitted)
        }

        #[cfg(not(windows))]
        {
            let Ok(mut inner) = h.inner.try_borrow_mut() else {
                return 0;
            };
            if !inner.mirror.is_active() {
                return 0;
            }
            // SAFETY: `scene_ref` is valid for this call.
            let update = unsafe {
                inner.mirror.sync_scene(
                    scene_ref,
                    &reader,
                    focused_node_id,
                    scale_factor,
                    title,
                )
            };
            h.action_bridge.set_epoch(inner.mirror.epoch());
            c_int::from(update.is_some())
        }
    }))
    .unwrap_or(0)
}

/// Updates top-level window focus state (for `WM_SETFOCUS` / `WM_KILLFOCUS`).
///
/// # Safety
///
/// `handle` must be null or a live pointer returned by [`nve_a11y_win_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_a11y_win_set_window_focus(
    handle: *mut NveA11yWinHandle,
    is_focused: c_int,
) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: caller guarantees `handle` is live for this call.
        let h = unsafe { &*handle };
        let focused = is_focused != 0;

        #[cfg(windows)]
        {
            let queued = {
                let Ok(mut inner) = h.inner.try_borrow_mut() else {
                    return;
                };
                inner.adapter.update_window_focus_state(focused)
            };
            if let Some(events) = queued {
                events.raise();
            }
        }

        #[cfg(not(windows))]
        {
            if let Ok(mut inner) = h.inner.try_borrow_mut() {
                inner.window_focused = focused;
            }
        }
    }));
}

/// Handles `WM_GETOBJECT`, writing the resulting `LRESULT` into `out_lresult` and returning `1`
/// when handled (or `0` to fall through to `DefWindowProc`).
///
/// # Safety
///
/// `handle` must be null or live; `out_lresult` (if non-null) must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_a11y_win_handle_wm_getobject(
    handle: *mut NveA11yWinHandle,
    wparam: usize,
    lparam: isize,
    out_lresult: *mut isize,
) -> c_int {
    if handle.is_null() || out_lresult.is_null() {
        return 0;
    }
    #[cfg(not(windows))]
    let _ = wparam;

    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: non-null `handle` is live on the UI thread for this call.
        let h = unsafe { &*handle };

        #[cfg(windows)]
        {
            let result = {
                let Ok(mut inner) = h.inner.try_borrow_mut() else {
                    return 0;
                };
                let initial = if !inner.adapter_activated
                    && wants_wm_getobject(lparam)
                    && inner.mirror.has_synced_scene()
                {
                    inner.mirror.activate();
                    Some(inner.mirror.build_full_tree_update())
                } else {
                    None
                };
                let mut activation = StaticActivation(initial);
                let res = inner.adapter.handle_wm_getobject(
                    accesskit_windows::WPARAM(wparam),
                    accesskit_windows::LPARAM(lparam),
                    &mut activation,
                );
                if res.is_some() {
                    inner.adapter_activated = true;
                    inner.mirror.activate();
                    h.action_bridge.set_epoch(inner.mirror.epoch());
                }
                res
            };
            if let Some(pending) = result {
                // Convert outside `inner` borrow: `UiaReturnRawElementProvider` can re-enter.
                let lres: accesskit_windows::LRESULT = pending.into();
                // SAFETY: caller checked `out_lresult` is non-null and writable.
                unsafe { *out_lresult = lres.0 };
                return 1;
            }
            0
        }

        #[cfg(not(windows))]
        {
            if !wants_wm_getobject(lparam) {
                return 0;
            }
            if let Ok(mut inner) = h.inner.try_borrow_mut() {
                inner.mirror.activate();
                if inner.mirror.has_synced_scene() {
                    let _ = inner.mirror.build_full_tree_update();
                }
                h.action_bridge.set_epoch(inner.mirror.epoch());
                // SAFETY: caller guarantees `out_lresult` is writable.
                unsafe { *out_lresult = 1 };
                return 1;
            }
            0
        }
    }))
    .unwrap_or(0)
}
