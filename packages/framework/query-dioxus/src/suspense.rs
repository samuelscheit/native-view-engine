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
use nve_dioxus::{ErrorBoundary, ErrorFallback, NveStreamSlot, UiErrorReport};
use nve_query::{QueryHandle, QueryMode};

use crate::bridge::DioxusQueryBridge;

/// Raw DOM attribute identifying a `<QuerySuspense>` container on both SSR and Client renders.
pub const NVE_SUSPENSE_ATTR: &str = "data-nve-suspense";

/// Sanitizes an explicit boundary `id` to safe ASCII `[A-Za-z0-9_-]` characters matching
/// `nve-backend-web`'s Declarative Shadow DOM `<slot>` naming contract, stripping leading `_`
/// (reserved for auto IDs, see [`SuspenseIdAllocator`]).
pub(crate) fn sanitize_slot_id(raw: Option<&str>) -> Option<String> {
    let clean: String = raw?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .skip_while(|&c| c == '_')
        .collect();
    (!clean.is_empty()).then_some(clean)
}

/// The one `<QuerySuspense>` ID allocator shared by the server ([`SsrBoundaryRegistry`]) and the
/// client ([`DioxusQueryBridge`]), so both assign identical `data-nve-suspense` IDs in render
/// order. Auto IDs (`_s0`, `_s1`, ...) start with `_`, which sanitized explicit IDs never do, so
/// the two namespaces are disjoint by construction whatever the render order.
#[derive(Debug, Default)]
pub(crate) struct SuspenseIdAllocator {
    next_auto: u32,
}

impl SuspenseIdAllocator {
    pub(crate) fn allocate(&mut self, explicit_id: Option<&str>) -> String {
        if let Some(clean) = sanitize_slot_id(explicit_id) {
            return clean;
        }
        let id = format!("_s{}", self.next_auto);
        self.next_auto = self.next_auto.wrapping_add(1);
        id
    }
}

#[derive(Default)]
struct BoundaryQuerySetInner {
    live: Vec<(QueryHandle, u32)>,
    errored: Vec<QueryHandle>,
}

/// Context provided inside `<QuerySuspense>` so [`crate::UseQuery::suspend`] /
/// [`crate::UseQuery::suspend_result`] record every [`QueryHandle`] belonging strictly to THAT
/// boundary (zero sibling leakage during per-boundary SSR delta dehydration). Live hook
/// references are strictly reference-counted (`record`/`forget`), while handles caught by the
/// boundary's `<ErrorBoundary>` are tracked separately in `errored` and consumed on reset.
#[derive(Clone, Default)]
pub struct BoundaryQuerySet(Rc<RefCell<BoundaryQuerySetInner>>);

impl BoundaryQuerySet {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one live hook's reference to `handle`.
    pub fn record(&self, handle: QueryHandle) {
        let mut inner = self.0.borrow_mut();
        match inner.live.iter_mut().find(|(h, _)| *h == handle) {
            Some((_, refs)) => *refs = refs.saturating_add(1),
            None => inner.live.push((handle, 1)),
        }
    }

    /// Drops one live hook's reference to `handle` (key switch or unmount), removing it once no
    /// live hook in this boundary reads it.
    pub fn forget(&self, handle: QueryHandle) {
        let mut inner = self.0.borrow_mut();
        if let Some(pos) = inner.live.iter().position(|(h, _)| *h == handle) {
            inner.live[pos].1 -= 1;
            if inner.live[pos].1 == 0 {
                inner.live.swap_remove(pos);
            }
        }
    }

    /// Retains `handle` in the boundary's caught-error set when [`crate::UseQuery::suspend`]
    /// throws into `<ErrorBoundary>`, so SSR error fallback dehydration and reset invalidation
    /// still see `handle` after `<ErrorBoundary>` unmounts the throwing hook.
    pub fn retain_errored(&self, handle: QueryHandle) {
        let mut inner = self.0.borrow_mut();
        if !inner.errored.contains(&handle) {
            inner.errored.push(handle);
        }
    }

    /// Removes `handle` from the caught-error set once a hook in this boundary resolves `Ok(_)`.
    pub fn clear_errored_handle(&self, handle: QueryHandle) {
        let mut inner = self.0.borrow_mut();
        if let Some(pos) = inner.errored.iter().position(|&h| h == handle) {
            inner.errored.swap_remove(pos);
        }
    }

    /// Clears all boundary-caught errored handles after `<ErrorBoundary>` reset invalidates them.
    pub fn clear_errored(&self) {
        self.0.borrow_mut().errored.clear();
    }

    /// Returns a deduplicated snapshot of all live and boundary-caught errored [`QueryHandle`]s.
    pub fn handles(&self) -> Vec<QueryHandle> {
        let inner = self.0.borrow();
        let mut out = Vec::with_capacity(inner.live.len() + inner.errored.len());
        for &(h, _) in &inner.live {
            out.push(h);
        }
        for &h in &inner.errored {
            if !out.contains(&h) {
                out.push(h);
            }
        }
        out
    }

    /// Whether `self` and `other` are the same boundary.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Typed key of one registered boundary in [`SsrBoundaryRegistry`] (distinct per boundary, even
/// when two boundaries share an explicit `id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct BoundaryKey(u32);

/// Context set to `true` inside an enclosing SSR [`QuerySuspense`] boundary so nested
/// [`QuerySuspense`] boundaries never emit unprojectable nested [`NveStreamSlot`] markers inside
/// Declarative Shadow DOM; the outer boundary resolves atomically once its full subtree settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InDeferredSuspense(pub bool);

/// Metadata tracked per top-level streaming [`QuerySuspense`] boundary inside
/// [`SsrBoundaryRegistry`].
#[derive(Clone)]
pub struct SsrBoundaryEntry {
    pub slot_id: String,
    pub handles: BoundaryQuerySet,
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
        self.handles.handles()
    }
}

#[derive(Default)]
struct SsrBoundaryRegistryInner {
    ids: SuspenseIdAllocator,
    boundaries: Vec<SsrBoundaryEntry>,
}

impl SsrBoundaryRegistryInner {
    /// Index of the boundary streaming `slot_id`: prefer an unflushed suspended boundary (so a
    /// duplicate explicit ID that settled pre-headers never masks a sibling still streaming), then
    /// any suspended boundary with `slot_id`, else the first registered with `slot_id`.
    fn streaming_entry_pos(&self, slot_id: &str) -> Option<usize> {
        self.boundaries
            .iter()
            .position(|e| e.slot_id == slot_id && !e.flushed && e.suspense_ctx.is_some())
            .or_else(|| {
                self.boundaries
                    .iter()
                    .position(|e| e.slot_id == slot_id && e.suspense_ctx.is_some())
            })
            .or_else(|| self.boundaries.iter().position(|e| e.slot_id == slot_id))
    }
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

    /// Allocates a slot ID: `explicit_id` (sanitized) when valid, else the next auto `_sN` (the
    /// same sequence the client bridge assigns).
    pub fn allocate_slot_id(&self, explicit_id: Option<&str>) -> String {
        self.inner.borrow_mut().ids.allocate(explicit_id)
    }

    /// Registers a top-level streaming boundary with its own handle collector. Boundaries sharing
    /// a duplicate explicit `id` stay distinct entries (handles never merge); if both suspend, the
    /// streaming shell rejects the duplicate slot before headers.
    pub fn register(&self, slot_id: String) -> (BoundaryKey, BoundaryQuerySet) {
        let mut inner = self.inner.borrow_mut();
        let key = BoundaryKey(inner.boundaries.len() as u32);
        let handles = BoundaryQuerySet::new();
        inner.boundaries.push(SsrBoundaryEntry {
            slot_id,
            handles: handles.clone(),
            suspense_ctx: None,
            flushed: false,
        });
        (key, handles)
    }

    /// Attaches the Dioxus [`SuspenseContext`] observed when boundary `key` suspends.
    pub fn attach_suspense_context(&self, key: BoundaryKey, ctx: SuspenseContext) {
        if let Some(entry) = self.inner.borrow_mut().boundaries.get_mut(key.0 as usize) {
            entry.suspense_ctx = Some(ctx);
        }
    }

    /// Marks every boundary that suspended and already resolved before HTTP headers as flushed so
    /// subsequent streaming lookups for a duplicated explicit slot ID target only the sibling
    /// still streaming.
    pub fn mark_settled_pre_headers(&self) {
        let mut inner = self.inner.borrow_mut();
        for entry in &mut inner.boundaries {
            if !entry.flushed && entry.suspense_ctx.is_some() && !entry.is_pending() {
                entry.flushed = true;
            }
        }
    }

    /// Marks streaming slot `slot_id` flushed, returning `true` iff it transitioned from
    /// unflushed to flushed.
    pub fn mark_flushed(&self, slot_id: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(pos) = inner.streaming_entry_pos(slot_id) else {
            return false;
        };
        let entry = &mut inner.boundaries[pos];
        if !entry.flushed {
            entry.flushed = true;
            true
        } else {
            false
        }
    }

    /// Returns a snapshot of all registered top-level boundaries in mount order.
    pub fn boundaries(&self) -> Vec<SsrBoundaryEntry> {
        self.inner.borrow().boundaries.clone()
    }

    /// Looks up the boundary streaming `slot_id` (preferring an unflushed suspended boundary).
    pub fn entry(&self, slot_id: &str) -> Option<SsrBoundaryEntry> {
        let inner = self.inner.borrow();
        let pos = inner.streaming_entry_pos(slot_id)?;
        inner.boundaries.get(pos).cloned()
    }

    /// Collects all unique [`QueryHandle`]s across unflushed suspended boundaries (the
    /// `StreamPending` set of [`nve_query::QueryClient::dehydrate_initial_stream_chunk`] for the
    /// `seq=0` mid-stream boot payload).
    pub fn pending_handles(&self) -> Vec<QueryHandle> {
        let inner = self.inner.borrow();
        let mut out = Vec::new();
        for entry in &inner.boundaries {
            if !entry.flushed && entry.suspense_ctx.is_some() {
                for h in entry.handles.handles() {
                    if !out.contains(&h) {
                        out.push(h);
                    }
                }
            }
        }
        out
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

    // `InDeferredSuspense(true)` is only ever provided by an outer SSR `<QuerySuspense>`, which
    // always provides its `BoundaryQuerySet` alongside, so a nested SSR boundary records into the
    // outer boundary's set.
    let nested_ssr = is_ssr && in_deferred;
    let (slot_id, boundary_handles, boundary_key) = use_hook(|| {
        let resolved_id = if let Some(ref reg) = registry {
            reg.allocate_slot_id(id.as_deref())
        } else if let Some(ref b) = bridge {
            b.allocate_suspense_id(id.as_deref())
        } else {
            SuspenseIdAllocator::default().allocate(id.as_deref())
        };
        if nested_ssr {
            let enclosing = try_consume_context::<BoundaryQuerySet>().unwrap_or_default();
            return (resolved_id, enclosing, None);
        }
        let (key, handles) = match registry {
            Some(ref reg) => {
                let (key, handles) = reg.register(resolved_id.clone());
                (Some(key), handles)
            }
            None => (None, BoundaryQuerySet::new()),
        };
        provide_context(handles.clone());
        if is_ssr {
            provide_context(InDeferredSuspense(true));
        }
        (resolved_id, handles, key)
    });

    let bridge_for_reset = bridge.clone();
    let handles_for_reset = boundary_handles.clone();

    // Nested `<QuerySuspense>` during SSR: NEVER emit a nested `NveStreamSlot` (unprojectable in
    // Declarative Shadow DOM). Suspension bubbles directly to the outer boundary's
    // `SuspenseContext` so the outer boundary resolves atomically once its entire subtree settles
    // while preserving local error isolation.
    if nested_ssr {
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

    let slot_id_for_fb = slot_id.clone();
    let registry_for_fb = registry.clone();

    rsx! {
        view {
            "data-nve-suspense": "{slot_id}",
            SuspenseBoundary {
                fallback: move |ctx: SuspenseContext| {
                    if let (Some(reg), Some(key)) = (&registry_for_fb, boundary_key) {
                        reg.attach_suspense_context(key, ctx);
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

fn invalidate_boundary_handles(bridge: &Option<DioxusQueryBridge>, handles: &BoundaryQuerySet) {
    let list = handles.handles();
    if let Some(b) = bridge
        && !list.is_empty()
    {
        b.client().invalidate_handles(&list);
    }
    handles.clear_errored();
}
