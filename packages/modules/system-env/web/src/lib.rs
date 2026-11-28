//! Browser `wasm32` leaf provider for `SERVICE_SYSTEM_ENV` (`0x0014`) and live [`WebThemeWatcher`].
//!
//! Queries `matchMedia`, `devicePixelRatio`, `navigator.language`, and `Intl` timezone using
//! compact `#[wasm_bindgen(inline_js = ...)]` bindings, watches media queries, re-arming DPR
//! `(resolution: <dpr>dppx)` queries, and `languagechange` events, and tracks live listeners in
//! a `thread_local!` slab detached cleanly on `Drop` (zero `unsafe impl Send`, zero re-entrant
//! closure UAF, zero leaked closures).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use std::any::{Any, TypeId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{ColorScheme, ContrastLevel, SERVICE_SYSTEM_ENV};
use nve_host_services::{
    pack_typed, CompletionSink, HandlerReply, HostServiceHandler, ResponseBuf, ServiceError,
};
pub use nve_system_env::{
    LocaleSnapshot, SystemEnvEvent, ThemeSnapshot, EVENT_LOCALE_CHANGED, EVENT_THEME_CHANGED,
};
use nve_system_env::{
    pack_locale_ffi, parse_locale_tag, SystemEnvBackend, METHOD_GET_LOCALE, METHOD_GET_THEME,
};

static NEXT_WATCHER_ID: AtomicU64 = AtomicU64::new(1);

/// Snapshot of live system-env watcher counts (for leak regression guards).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemEnvSlabCounts {
    pub rust_watchers: usize,
    pub js_watchers: usize,
}

/// Pure zero-allocation builder constructing a [`ThemeSnapshot`] from browser media-query flags
/// and `window.devicePixelRatio`.
#[inline]
pub fn theme_from_media_flags(
    dark: bool,
    high_contrast: bool,
    reduced_motion: bool,
    reduced_transparency: bool,
    device_pixel_ratio: f32,
) -> ThemeSnapshot {
    let scale_x100 = if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
        (device_pixel_ratio * 100.0)
            .round()
            .clamp(1.0, f32::from(u16::MAX)) as u16
    } else {
        100
    };
    ThemeSnapshot {
        color_scheme: Some(if dark {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        }),
        contrast: if high_contrast {
            ContrastLevel::High
        } else {
            ContrastLevel::Standard
        },
        reduced_motion,
        reduced_transparency,
        accent_rgba: 0x007A_FFFF,
        font_scale_x1000: 1000,
        scale_factor_x100: scale_x100,
    }
}

/// Pure builder constructing a [`LocaleSnapshot`] from browser `navigator.language` and `Intl`
/// timezone fields.
pub fn locale_from_browser_fields(
    lang_tag: &str,
    timezone: Option<&str>,
    uses_24h_clock: bool,
) -> LocaleSnapshot {
    let tz = timezone.filter(|s| !s.is_empty()).map(str::to_owned);
    parse_locale_tag(lang_tag, tz, uses_24h_clock)
}

/// Browser [`SystemEnvBackend`] & [`HostServiceHandler`] for [`SERVICE_SYSTEM_ENV`].
pub struct WebSystemEnv {
    theme: Mutex<ThemeSnapshot>,
    locale: Mutex<LocaleSnapshot>,
}

impl Default for WebSystemEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSystemEnv {
    /// Captures the live browser theme & locale snapshot (or defaults on non-wasm32 hosts).
    pub fn new() -> Self {
        Self::with_initial(query_live_theme(), query_live_locale())
    }

    /// Creates an instance seeded with explicit snapshots (deterministic tests).
    pub fn with_initial(theme: ThemeSnapshot, locale: LocaleSnapshot) -> Self {
        let theme = Mutex::new(theme);
        let locale = Mutex::new(locale);
        drop(theme.lock());
        drop(locale.lock());
        Self { theme, locale }
    }

    /// Re-queries live browser media flags and updates the cached [`ThemeSnapshot`].
    pub fn refresh_theme(&self) -> ThemeSnapshot {
        let next = query_live_theme();
        *self.theme.lock().unwrap_or_else(|e| e.into_inner()) = next;
        next
    }

    /// Re-queries live browser `navigator.language` / `Intl` timezone and updates the cached
    /// [`LocaleSnapshot`].
    pub fn refresh_locale(&self) -> LocaleSnapshot {
        let next = query_live_locale();
        *self.locale.lock().unwrap_or_else(|e| e.into_inner()) = next.clone();
        next
    }

    /// Overwrites the cached [`ThemeSnapshot`] (used by [`WebThemeWatcher`] and tests).
    pub fn set_theme(&self, theme: ThemeSnapshot) {
        *self.theme.lock().unwrap_or_else(|e| e.into_inner()) = theme;
    }

    /// Overwrites the cached [`LocaleSnapshot`] (used by [`WebThemeWatcher`] and tests).
    pub fn set_locale(&self, locale: LocaleSnapshot) {
        *self.locale.lock().unwrap_or_else(|e| e.into_inner()) = locale;
    }

    /// Returns the cached [`ThemeSnapshot`] without heap allocation.
    pub fn current_theme(&self) -> ThemeSnapshot {
        *self.theme.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Returns a cloned snapshot of the cached [`LocaleSnapshot`].
    pub fn current_locale(&self) -> LocaleSnapshot {
        self.locale
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl SystemEnvBackend for WebSystemEnv {
    fn get_theme(&self) -> Result<ThemeSnapshot, ServiceError> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(self.refresh_theme())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(self.current_theme())
        }
    }

    fn get_locale(&self) -> Result<LocaleSnapshot, ServiceError> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(self.refresh_locale())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(self.current_locale())
        }
    }
}

impl HostServiceHandler for WebSystemEnv {
    fn service_id(&self) -> u32 {
        SERVICE_SYSTEM_ENV
    }

    fn handle(
        &self,
        method_id: u32,
        _request_id: u64,
        _request: &[u8],
        _done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        match method_id {
            METHOD_GET_THEME => {
                let raw = self.get_theme()?.to_repr_c_bytes();
                Ok(HandlerReply::Ready(ResponseBuf::from_slice_or_vec(&raw)))
            }
            METHOD_GET_LOCALE => pack_locale_ffi(&self.get_locale()?).map(HandlerReply::ready),
            _ => Err(ServiceError::Unsupported),
        }
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn SystemEnvBackend>(self))
    }
}

/// Live browser `matchMedia` (color scheme, contrast, reduced motion, reduced transparency,
/// re-arming DPR `resolution: <dpr>dppx`) and `languagechange` watcher pushing
/// `SERVICE_SYSTEM_ENV` `Push` records into a [`CompletionSink`].
pub struct WebThemeWatcher {
    watcher_id: u64,
    env: Arc<WebSystemEnv>,
    sink: CompletionSink,
}

impl WebThemeWatcher {
    /// Installs live browser `matchMedia`, DPR, and `languagechange` listeners pushing updates
    /// onto `sink`.
    pub fn install(env: Arc<WebSystemEnv>, sink: CompletionSink) -> Self {
        let watcher_id = NEXT_WATCHER_ID.fetch_add(1, Ordering::Relaxed);
        #[cfg(target_arch = "wasm32")]
        wasm_impl::install_watcher(watcher_id, Arc::clone(&env), sink.clone());
        Self {
            watcher_id,
            env,
            sink,
        }
    }

    /// Pushes a `ThemeSnapshot` update through the watcher's [`CompletionSink`] as a canonical
    /// `SERVICE_SYSTEM_ENV` `EVENT_THEME_CHANGED` `Push` record.
    pub fn notify_theme_change(&self, theme: ThemeSnapshot) {
        self.env.set_theme(theme);
        self.sink.push_event(
            SERVICE_SYSTEM_ENV,
            EVENT_THEME_CHANGED,
            theme.to_repr_c_bytes().to_vec(),
        );
    }

    /// Pushes a `LocaleSnapshot` update through the watcher's [`CompletionSink`] as a canonical
    /// `SERVICE_SYSTEM_ENV` `EVENT_LOCALE_CHANGED` `Push` record.
    pub fn notify_locale_change(&self, locale: LocaleSnapshot) {
        self.env.set_locale(locale.clone());
        if let Ok(payload) = pack_locale_ffi(&locale) {
            self.sink
                .push_event(SERVICE_SYSTEM_ENV, EVENT_LOCALE_CHANGED, payload);
        }
    }

    /// Returns raw live Rust and JS watcher slab counts WITHOUT triggering a hidden reap so leak
    /// tests verify automatic deferred reaping directly.
    pub fn debug_slab_counts() -> SystemEnvSlabCounts {
        #[cfg(target_arch = "wasm32")]
        {
            wasm_impl::debug_slab_counts()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            SystemEnvSlabCounts::default()
        }
    }

    /// Returns how many times the live re-arming `matchMedia("(resolution: <dpr>dppx)")` change
    /// listener (excluding `resize` fallbacks) has fired in this browser instance.
    pub fn debug_dpr_mql_fire_count() -> u32 {
        #[cfg(target_arch = "wasm32")]
        {
            wasm_impl::debug_dpr_mql_fire_count()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            0
        }
    }
}

impl Drop for WebThemeWatcher {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        wasm_impl::uninstall_watcher(self.watcher_id);
        let _ = self.watcher_id;
    }
}

fn query_live_theme() -> ThemeSnapshot {
    #[cfg(target_arch = "wasm32")]
    {
        let flags = wasm_impl::query_media_flags();
        let dpr = wasm_impl::query_dpr();
        theme_from_media_flags(
            (flags & 1) != 0,
            (flags & 2) != 0,
            (flags & 4) != 0,
            (flags & 8) != 0,
            dpr,
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ThemeSnapshot::default()
    }
}

fn query_live_locale() -> LocaleSnapshot {
    #[cfg(target_arch = "wasm32")]
    {
        let lang = wasm_impl::query_language();
        let tz = wasm_impl::query_timezone();
        let hour24 = wasm_impl::query_uses_24h();
        locale_from_browser_fields(
            if lang.is_empty() { "en-US" } else { &lang },
            (!tz.is_empty()).then_some(tz.as_str()),
            hour24,
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        LocaleSnapshot::default()
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    use nve_abi::SERVICE_SYSTEM_ENV;
    use nve_host_services::{CompletionSink, ReentrantSlotSlab};
    use nve_system_env::{pack_locale_ffi, EVENT_LOCALE_CHANGED, EVENT_THEME_CHANGED};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::*;

    use super::{query_live_locale, query_live_theme, SystemEnvSlabCounts, WebSystemEnv};

    #[wasm_bindgen(inline_js = "
const QUERIES = [
  '(prefers-color-scheme: dark)',
  '(prefers-contrast: more)',
  '(prefers-reduced-motion: reduce)',
  '(prefers-reduced-transparency: reduce)',
];
const watchers = new Map();
let dprMqlFires = 0;

export function nve_web_sysenv_media_flags() {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return 0;
  let bits = 0;
  for (let i = 0; i < QUERIES.length; i++) {
    try { if (window.matchMedia(QUERIES[i]).matches) bits |= (1 << i); } catch (_) {}
  }
  return bits;
}

export function nve_web_sysenv_dpr() {
  if (typeof window === 'undefined') return 1.0;
  const d = Number(window.devicePixelRatio);
  return Number.isFinite(d) && d > 0 ? d : 1.0;
}

export function nve_web_sysenv_lang() {
  try {
    if (typeof navigator !== 'undefined' && typeof navigator.language === 'string' && navigator.language) {
      return navigator.language;
    }
  } catch (_) {}
  return 'en-US';
}

export function nve_web_sysenv_tz() {
  try {
    if (typeof Intl !== 'undefined' && Intl.DateTimeFormat) {
      const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
      if (typeof tz === 'string') return tz;
    }
  } catch (_) {}
  return 'UTC';
}

export function nve_web_sysenv_hour24() {
  try {
    if (typeof Intl !== 'undefined' && Intl.DateTimeFormat) {
      const opts = Intl.DateTimeFormat(undefined, { hour: 'numeric' }).resolvedOptions();
      return opts.hourCycle === 'h23' || opts.hourCycle === 'h24' || opts.hour12 === false;
    }
  } catch (_) {}
  return false;
}

export function nve_web_sysenv_watch(id, onTheme, onLocale) {
  if (typeof window === 'undefined') return;
  const entry = {
    disposed: false,
    mqls: [],
    dprMql: null,
    lastDpr: nve_web_sysenv_dpr(),
    onThemeHandler: null,
    onDprMqlChange: null,
    onResize: null,
    onLang: null,
  };
  const onThemeHandler = () => {
    if (entry.disposed) return;
    try { onTheme(); } catch (_) {}
  };
  entry.onThemeHandler = onThemeHandler;
  const armDpr = () => {
    if (entry.dprMql && entry.onDprMqlChange) {
      try { entry.dprMql.removeEventListener('change', entry.onDprMqlChange); } catch (_) {}
      entry.dprMql = null;
    }
    if (entry.disposed || typeof window.matchMedia !== 'function') return;
    const dpr = nve_web_sysenv_dpr();
    entry.lastDpr = dpr;
    try {
      const mql = window.matchMedia(`(resolution: ${dpr}dppx)`);
      if (typeof mql.addEventListener === 'function') {
        mql.addEventListener('change', onDprMqlChange);
        entry.dprMql = mql;
      }
    } catch (_) {}
  };
  const handleDprTransition = () => {
    if (entry.disposed) return;
    armDpr();
    try { onTheme(); } catch (_) {}
  };
  const onDprMqlChange = () => {
    if (entry.disposed) return;
    dprMqlFires += 1;
    handleDprTransition();
  };
  entry.onDprMqlChange = onDprMqlChange;
  if (typeof window.matchMedia === 'function') {
    for (let i = 0; i < QUERIES.length; i++) {
      try {
        const mql = window.matchMedia(QUERIES[i]);
        if (typeof mql.addEventListener === 'function') {
          mql.addEventListener('change', onThemeHandler);
          entry.mqls.push(mql);
        }
      } catch (_) {}
    }
    armDpr();
  }
  const onResize = () => {
    if (entry.disposed) return;
    const dpr = nve_web_sysenv_dpr();
    if (dpr !== entry.lastDpr) {
      handleDprTransition();
    }
  };
  entry.onResize = onResize;
  const onLang = () => {
    if (entry.disposed) return;
    try { onLocale(); } catch (_) {}
  };
  entry.onLang = onLang;
  if (typeof window.addEventListener === 'function') {
    window.addEventListener('resize', onResize);
    window.addEventListener('languagechange', onLang);
  }
  watchers.set(id, entry);
}

export function nve_web_sysenv_unwatch(id) {
  const entry = watchers.get(id);
  if (!entry) return;
  entry.disposed = true;
  watchers.delete(id);
  for (let i = 0; i < entry.mqls.length; i++) {
    try { entry.mqls[i].removeEventListener('change', entry.onThemeHandler); } catch (_) {}
  }
  if (entry.dprMql && entry.onDprMqlChange) {
    try { entry.dprMql.removeEventListener('change', entry.onDprMqlChange); } catch (_) {}
    entry.dprMql = null;
  }
  if (typeof window !== 'undefined' && typeof window.removeEventListener === 'function') {
    if (entry.onResize) {
      try { window.removeEventListener('resize', entry.onResize); } catch (_) {}
    }
    if (entry.onLang) {
      try { window.removeEventListener('languagechange', entry.onLang); } catch (_) {}
    }
  }
}

export function nve_web_sysenv_watcher_count() {
  return watchers.size;
}

export function nve_web_sysenv_dpr_mql_fires() {
  return dprMqlFires;
}

export function nve_web_sysenv_schedule_reap(cb) {
  const run = () => { try { cb(); } catch (_) {} };
  if (typeof queueMicrotask === 'function') {
    queueMicrotask(run);
  } else {
    Promise.resolve().then(run);
  }
}
")]
    extern "C" {
        fn nve_web_sysenv_media_flags() -> u32;
        fn nve_web_sysenv_dpr() -> f32;
        fn nve_web_sysenv_lang() -> String;
        fn nve_web_sysenv_tz() -> String;
        fn nve_web_sysenv_hour24() -> bool;
        fn nve_web_sysenv_watch(id: f64, on_theme: &JsValue, on_locale: &JsValue);
        fn nve_web_sysenv_unwatch(id: f64);
        fn nve_web_sysenv_watcher_count() -> u32;
        fn nve_web_sysenv_dpr_mql_fires() -> u32;
        fn nve_web_sysenv_schedule_reap(cb: &JsValue);
    }

    type WatchClosure = Closure<dyn FnMut()>;

    struct WatcherSlot {
        _on_theme: WatchClosure,
        _on_locale: WatchClosure,
    }

    thread_local! {
        static WATCHER_SLAB: RefCell<ReentrantSlotSlab<u64, WatcherSlot>> =
            const { RefCell::new(ReentrantSlotSlab::new()) };
        static REAP_SCHEDULED: Cell<bool> = const { Cell::new(false) };
        static REAP_CLOSURE: Closure<dyn FnMut()> = Closure::new(|| {
            REAP_SCHEDULED.with(|f| f.set(false));
            reap_deferred();
        });
    }

    fn schedule_deferred_reap() {
        if REAP_SCHEDULED.with(|f| f.replace(true)) {
            return;
        }
        REAP_CLOSURE.with(|cb| nve_web_sysenv_schedule_reap(cb.as_ref()));
    }

    struct ActiveWatcherGuard(u64);

    impl Drop for ActiveWatcherGuard {
        fn drop(&mut self) {
            let needs_reap = WATCHER_SLAB.with(|slab| slab.borrow_mut().leave(self.0, false));
            if needs_reap {
                schedule_deferred_reap();
            }
        }
    }

    pub(super) fn query_media_flags() -> u32 {
        nve_web_sysenv_media_flags()
    }

    pub(super) fn query_dpr() -> f32 {
        nve_web_sysenv_dpr()
    }

    pub(super) fn query_language() -> String {
        nve_web_sysenv_lang()
    }

    pub(super) fn query_timezone() -> String {
        nve_web_sysenv_tz()
    }

    pub(super) fn query_uses_24h() -> bool {
        nve_web_sysenv_hour24()
    }

    pub(super) fn debug_dpr_mql_fire_count() -> u32 {
        nve_web_sysenv_dpr_mql_fires()
    }

    pub(super) fn reap_deferred() {
        let evicted = WATCHER_SLAB.with(|slab| slab.borrow_mut().reap_deferred());
        drop(evicted);
    }

    pub(super) fn install_watcher(id: u64, env: Arc<WebSystemEnv>, sink: CompletionSink) {
        reap_deferred();
        let env_theme = Arc::clone(&env);
        let sink_theme = sink.clone();
        let on_theme = Closure::<dyn FnMut()>::new(move || {
            WATCHER_SLAB.with(|slab| slab.borrow_mut().enter(id));
            let _guard = ActiveWatcherGuard(id);
            let next = query_live_theme();
            env_theme.set_theme(next);
            sink_theme.push_event(
                SERVICE_SYSTEM_ENV,
                EVENT_THEME_CHANGED,
                next.to_repr_c_bytes().to_vec(),
            );
        });
        let on_locale = Closure::<dyn FnMut()>::new(move || {
            WATCHER_SLAB.with(|slab| slab.borrow_mut().enter(id));
            let _guard = ActiveWatcherGuard(id);
            let next = query_live_locale();
            env.set_locale(next.clone());
            if let Ok(payload) = pack_locale_ffi(&next) {
                sink.push_event(SERVICE_SYSTEM_ENV, EVENT_LOCALE_CHANGED, payload);
            }
        });
        nve_web_sysenv_watch(id as f64, on_theme.as_ref(), on_locale.as_ref());
        let evicted = WATCHER_SLAB.with(|slab| {
            slab.borrow_mut().insert(
                id,
                WatcherSlot {
                    _on_theme: on_theme,
                    _on_locale: on_locale,
                },
            )
        });
        drop(evicted);
    }

    pub(super) fn uninstall_watcher(id: u64) {
        nve_web_sysenv_unwatch(id as f64);
        let evicted = WATCHER_SLAB.with(|slab| slab.borrow_mut().remove_or_defer(id));
        drop(evicted);
    }

    pub(super) fn debug_slab_counts() -> SystemEnvSlabCounts {
        SystemEnvSlabCounts {
            rust_watchers: WATCHER_SLAB.with(|slab| slab.borrow().len()),
            js_watchers: nve_web_sysenv_watcher_count() as usize,
        }
    }
}
