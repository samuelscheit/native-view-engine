//! Portable `<QuerySuspense>` streaming boundary, per-boundary [`BoundaryQuerySet`] handle
//! attribution, nested-SSR [`InDeferredSuspense`] guard, and server-side [`SsrBoundaryRegistry`].

use core::cell::RefCell;
use std::rc::Rc;

use dioxus_core::{
    provide_context, try_consume_context, use_hook, Element, EventHandler, SuspenseBoundary,
    SuspenseContext,
};
use dioxus_core_macro::{component, rsx, Props};
use nve_dioxus::prelude::dioxus_elements;
pub use nve_dioxus::NVE_SUSPENSE_ATTR;
use nve_dioxus::{ErrorBoundary, ErrorFallback, NveStreamSlot, UiErrorReport};
use nve_query::{QueryHandle, QueryMode};

use crate::bridge::DioxusQueryBridge;

/// Sanitizes an explicit boundary `id` to safe ASCII `[A-Za-z0-9_-]` characters matching
/// `nve-backend-web`'s Declarative Shadow DOM `<slot>` naming contract.
pub(crate) fn sanitize_slot_id(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let clean: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    if clean.is_empty() {
        None
    } else {
        Some(clean)
    }
}

/// Context provided inside `<QuerySuspense>` so [`crate::UseQuery::suspend`] /
/// [`crate::UseQuery::suspend_result`] record every [`QueryHandle`] belonging strictly to THAT
/// boundary (zero sibling leakage during per-boundary SSR delta dehydration).
#[derive(Clone, Default)]
pub struct BoundaryQuerySet(pub Rc<RefCell<Vec<QueryHandle>>>);

impl BoundaryQuerySet {
    #[inline]
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    /// Records `handle` once in this boundary's query list.
    pub fn record(&self, handle: QueryHandle) {
        let mut list = self.0.borrow_mut();
        if !list.contains(&handle) {
            list.push(handle);
        }
    }

    /// Returns a snapshot of all [`QueryHandle`]s recorded in this boundary.
    #[inline]
    pub fn handles(&self) -> Vec<QueryHandle> {
        self.0.borrow().clone()
    }
}

/// Context set to `true` inside an enclosing SSR [`QuerySuspense`] boundary so nested
/// [`QuerySuspense`] boundaries never emit unprojectable nested [`fn@NveStreamSlot`] markers inside
/// Declarative Shadow DOM; the outer boundary resolves atomically once its full subtree settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InDeferredSuspense(pub bool);

/// Metadata tracked per top-level streaming [`QuerySuspense`] boundary inside
/// [`SsrBoundaryRegistry`].
#[derive(Clone)]
pub struct SsrBoundaryEntry {
    pub slot_id: String,
    pub handles: Rc<RefCell<Vec<QueryHandle>>>,
    pub suspense_ctx: Option<SuspenseContext>,
    pub flushed: bool,
}

impl SsrBoundaryEntry {
    /// Returns `true` when this boundary suspended and still has unresolved tasks.
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.suspense_ctx
            .as_ref()
            .is_some_and(SuspenseContext::has_suspended_tasks)
    }

    /// Returns `true` when this boundary suspended, all its tasks have resolved, and it has not
    /// yet been flushed to the streaming response.
    #[inline]
    pub fn is_ready_to_flush(&self) -> bool {
        !self.flushed
            && self
                .suspense_ctx
                .as_ref()
                .is_some_and(|ctx| !ctx.has_suspended_tasks())
    }

    /// Returns a snapshot of the [`QueryHandle`]s attributed to this boundary.
    #[inline]
    pub fn query_handles(&self) -> Vec<QueryHandle> {
        self.handles.borrow().clone()
    }
}

#[derive(Default)]
struct SsrBoundaryRegistryInner {
    next_auto_id: u32,
    boundaries: Vec<SsrBoundaryEntry>,
}

/// Optional server-only context injected on `ScopeId::ROOT` by `nve-ssr` to coordinate
/// out-of-order streaming `<QuerySuspense>` boundaries (`None` on client/desktop).
#[derive(Clone, Default)]
pub struct SsrBoundaryRegistry {
    inner: Rc<RefCell<SsrBoundaryRegistryInner>>,
}

impl SsrBoundaryRegistry {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates a slot ID using `explicit_id` (sanitized) when valid or the monotonic `s0`, `s1`,
    /// ... sequence otherwise.
    pub fn allocate_slot_id(&self, explicit_id: Option<&str>) -> String {
        if let Some(clean) = sanitize_slot_id(explicit_id) {
            return clean;
        }
        let mut inner = self.inner.borrow_mut();
        let n = inner.next_auto_id;
        inner.next_auto_id = inner.next_auto_id.wrapping_add(1);
        format!("s{n}")
    }

    /// Registers a top-level streaming boundary entry with its handle collector.
    pub fn register(&self, slot_id: String, handles: Rc<RefCell<Vec<QueryHandle>>>) {
        let mut inner = self.inner.borrow_mut();
        if let Some(existing) = inner.boundaries.iter_mut().find(|e| e.slot_id == slot_id) {
            existing.handles = handles;
            return;
        }
        inner.boundaries.push(SsrBoundaryEntry {
            slot_id,
            handles,
            suspense_ctx: None,
            flushed: false,
        });
    }

    /// Attaches the Dioxus [`SuspenseContext`] observed when `slot_id` suspends.
    pub fn attach_suspense_context(&self, slot_id: &str, ctx: SuspenseContext) {
        let mut inner = self.inner.borrow_mut();
        if let Some(entry) = inner.boundaries.iter_mut().find(|e| e.slot_id == slot_id) {
            entry.suspense_ctx = Some(ctx);
        }
    }

    /// Marks `slot_id` flushed, returning `true` iff it transitioned from unflushed to flushed.
    pub fn mark_flushed(&self, slot_id: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        if let Some(entry) = inner.boundaries.iter_mut().find(|e| e.slot_id == slot_id)
            && !entry.flushed
        {
            entry.flushed = true;
            return true;
        }
        false
    }

    /// Returns a snapshot of all registered top-level boundaries in mount order.
    pub fn boundaries(&self) -> Vec<SsrBoundaryEntry> {
        self.inner.borrow().boundaries.clone()
    }

    /// Looks up one registered boundary by `slot_id`.
    pub fn entry(&self, slot_id: &str) -> Option<SsrBoundaryEntry> {
        self.inner
            .borrow()
            .boundaries
            .iter()
            .find(|e| e.slot_id == slot_id)
            .cloned()
    }

    /// Returns all suspended boundaries whose tasks have resolved and are ready to flush.
    pub fn ready_to_flush(&self) -> Vec<SsrBoundaryEntry> {
        self.inner
            .borrow()
            .boundaries
            .iter()
            .filter(|e| e.is_ready_to_flush())
            .cloned()
            .collect()
    }

    /// Returns `true` while any suspended boundary has not yet been flushed.
    pub fn has_pending(&self) -> bool {
        self.inner
            .borrow()
            .boundaries
            .iter()
            .any(|e| !e.flushed && e.suspense_ctx.is_some())
    }

    /// Collects all unique [`QueryHandle`]s across unflushed suspended boundaries (used with
    /// [`nve_query::QueryClient::dehydrate_pending_handles`] for `seq=0` mid-stream boot).
    pub fn pending_handles(&self) -> Vec<QueryHandle> {
        let inner = self.inner.borrow();
        let mut out = Vec::new();
        for entry in &inner.boundaries {
            if !entry.flushed && entry.suspense_ctx.is_some() {
                for &h in entry.handles.borrow().iter() {
                    if !out.contains(&h) {
                        out.push(h);
                    }
                }
            }
        }
        out
    }

    /// Collects all unique [`QueryHandle`]s recorded across all registered boundaries.
    pub fn all_handles(&self) -> Vec<QueryHandle> {
        let inner = self.inner.borrow();
        let mut out = Vec::new();
        for entry in &inner.boundaries {
            for &h in entry.handles.borrow().iter() {
                if !out.contains(&h) {
                    out.push(h);
                }
            }
        }
        out
    }

    /// Returns `true` if any registered SSR boundary has recorded at least one query handle.
    pub fn has_active_ssr_handles(&self) -> bool {
        self.inner
            .borrow()
            .boundaries
            .iter()
            .any(|e| !e.handles.borrow().is_empty())
    }
}

/// Portable query suspense + error boundary uniting Declarative Shadow DOM SSR streaming
/// ([`NveStreamSlot`]), per-boundary `NVEQ` v2 handle attribution ([`BoundaryQuerySet`]), and
/// client/native interactive suspense + [`ErrorBoundary`] recovery.
#[component]
pub fn QuerySuspense(
    #[props(default)] id: Option<String>,
    fallback: Element,
    #[props(into, default)] error_fallback: ErrorFallback,
    #[props(default)] on_error: Option<EventHandler<UiErrorReport>>,
    #[props(default)] on_reset: Option<EventHandler<u32>>,
    children: Element,
) -> Element {
    let bridge = try_consume_context::<DioxusQueryBridge>();
    let registry = try_consume_context::<SsrBoundaryRegistry>();
    let in_deferred = try_consume_context::<InDeferredSuspense>().is_some_and(|d| d.0);
    let is_ssr = registry.is_some()
        || bridge
            .as_ref()
            .is_some_and(|b| b.client().mode() == QueryMode::Ssr);

    let (slot_id, boundary_handles, has_enclosing_query_set) = use_hook(|| {
        let resolved_id = if let Some(ref reg) = registry {
            reg.allocate_slot_id(id.as_deref())
        } else if let Some(ref b) = bridge {
            b.allocate_suspense_id(id.as_deref())
        } else {
            sanitize_slot_id(id.as_deref()).unwrap_or_else(|| "s0".to_owned())
        };

        if is_ssr && in_deferred {
            if let Some(enclosing) = try_consume_context::<BoundaryQuerySet>() {
                (resolved_id, enclosing.0, true)
            } else {
                let handles = Rc::new(RefCell::new(Vec::new()));
                provide_context(BoundaryQuerySet(Rc::clone(&handles)));
                (resolved_id, handles, false)
            }
        } else {
            let handles = Rc::new(RefCell::new(Vec::new()));
            provide_context(BoundaryQuerySet(Rc::clone(&handles)));
            if is_ssr {
                provide_context(InDeferredSuspense(true));
            }
            if let Some(ref reg) = registry {
                reg.register(resolved_id.clone(), Rc::clone(&handles));
            }
            (resolved_id, handles, false)
        }
    });

    let bridge_for_reset = bridge.clone();
    let handles_for_reset = Rc::clone(&boundary_handles);

    // Nested `<QuerySuspense>` during SSR: NEVER emit a nested `NveStreamSlot` (unprojectable in
    // Declarative Shadow DOM). When enclosed by an outer `<QuerySuspense>`, let suspension bubble
    // directly to the outer boundary's `SuspenseContext` so the outer boundary resolves atomically
    // once its entire subtree settles while preserving local error isolation.
    if is_ssr && in_deferred {
        if has_enclosing_query_set {
            return rsx! {
                view {
                    "data-nve-suspense": "{slot_id}",
                    ErrorBoundary {
                        fallback: error_fallback,
                        on_error: on_error,
                        on_reset: move |reset_count: u32| {
                            invalidate_boundary_handles(&bridge_for_reset, &handles_for_reset);
                            if let Some(cb) = on_reset {
                                cb.call(reset_count);
                            }
                        },
                        {children}
                    }
                }
            };
        }
        return rsx! {
            view {
                "data-nve-suspense": "{slot_id}",
                SuspenseBoundary {
                    fallback: move |_| fallback.clone(),
                    ErrorBoundary {
                        fallback: error_fallback,
                        on_error: on_error,
                        on_reset: move |reset_count: u32| {
                            invalidate_boundary_handles(&bridge_for_reset, &handles_for_reset);
                            if let Some(cb) = on_reset {
                                cb.call(reset_count);
                            }
                        },
                        {children}
                    }
                }
            }
        };
    }

    let slot_id_for_fb = slot_id.clone();
    let slot_id_for_reg = slot_id.clone();
    let registry_for_fb = registry.clone();

    rsx! {
        view {
            "data-nve-suspense": "{slot_id}",
            SuspenseBoundary {
                fallback: move |ctx: SuspenseContext| {
                    if let Some(ref reg) = registry_for_fb {
                        reg.attach_suspense_context(&slot_id_for_reg, ctx);
                    }
                    rsx! {
                        NveStreamSlot {
                            slot_id: slot_id_for_fb.clone(),
                            {fallback.clone()}
                        }
                    }
                },
                ErrorBoundary {
                    fallback: error_fallback,
                    on_error: on_error,
                    on_reset: move |reset_count: u32| {
                        invalidate_boundary_handles(&bridge_for_reset, &handles_for_reset);
                        if let Some(cb) = on_reset {
                            cb.call(reset_count);
                        }
                    },
                    {children}
                }
            }
        }
    }
}

fn invalidate_boundary_handles(
    bridge: &Option<DioxusQueryBridge>,
    handles: &Rc<RefCell<Vec<QueryHandle>>>,
) {
    if let Some(b) = bridge {
        let list = handles.borrow().clone();
        if !list.is_empty() {
            b.client().invalidate_handles(&list);
        }
    }
}
