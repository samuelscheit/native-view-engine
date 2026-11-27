//! Sorted-slice `RpcRegistry` (`Box<[(ProcId, Entry)]>`), by-value `Cx<S, U>`, and dynamic
//! `TagCollector` for zero-allocation lookup and startup collision detection.

use core::future::Future;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::HeaderMap;
use nve_rpc::{
    assert_valid_procedure_name, Action, BoxFuture, CacheTag, CborCodec, CodecError, DomainStatus,
    Effect, EffectKind, FieldErrors, FormDecode, FormPairs, MaybeSend, MaybeSync, ProcId,
    Procedure, RpcCodec, RpcStatus, TagSink,
};

/// Dynamic invalidation collector carried inside [`Cx`] for [`nve_rpc::Action`] procedures.
///
/// Queries receive [`TagCollector::noop`] (`None`, 0 heap allocations); actions receive a single
/// `Arc<Mutex<TagSink>>` (1 heap allocation for up to 8 tags).
#[derive(Debug, Clone, Default)]
pub(crate) struct TagCollector {
    inner: Option<Arc<Mutex<TagSink>>>,
}

impl TagCollector {
    #[inline]
    pub(crate) const fn noop() -> Self {
        Self { inner: None }
    }

    #[inline]
    pub(crate) fn active() -> Self {
        Self {
            inner: Some(Arc::new(Mutex::new(TagSink::new()))),
        }
    }

    /// Records a cache tag to be emitted in the response `Nve-Invalidate` header.
    #[inline]
    pub(crate) fn invalidate(&self, tag: impl CacheTag) {
        if let Some(lock) = &self.inner {
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            guard.push(tag);
        }
    }

    /// Records multiple cache tags for invalidation.
    #[inline]
    pub(crate) fn invalidate_many<T: CacheTag>(&self, tags: impl IntoIterator<Item = T>) {
        if let Some(lock) = &self.inner {
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            guard.extend_tags(tags);
        }
    }

    /// Latches wildcard (`*`) invalidation so clients invalidate all cached queries.
    #[inline]
    pub(crate) fn mark_all(&self) {
        if let Some(lock) = &self.inner {
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            guard.mark_all();
        }
    }

    /// Drains recorded dynamic tags into `sink`.
    pub(crate) fn drain_into(&self, sink: &mut TagSink) {
        if let Some(lock) = &self.inner {
            let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_overflow_all() {
                sink.mark_all();
                return;
            }
            for &tag in guard.as_slice() {
                sink.push(tag);
            }
        }
    }
}

/// By-value handler context passed into every registered procedure handler.
///
/// Avoids HRTB `for<'r> AsyncFn` lifetime traps so handler futures cleanly satisfy `Send + 'static`
/// on native multi-threaded servers (`axum` / `hyper`) and `?Send` on `wasm32-wasip2`.
#[derive(Debug, Clone)]
pub struct Cx<S, U = ()> {
    pub state: S,
    pub principal: U,
    pub headers: HeaderMap,
    tags: TagCollector,
}

impl<S, U> Cx<S, U> {
    #[inline]
    pub(crate) fn new(state: S, principal: U, headers: HeaderMap, tags: TagCollector) -> Self {
        Self {
            state,
            principal,
            headers,
            tags,
        }
    }

    /// Marks `tag` as invalidated in this action's `Nve-Invalidate` response header.
    #[inline]
    pub fn invalidate(&self, tag: impl CacheTag) {
        self.tags.invalidate(tag);
    }

    /// Marks every tag in `tags` as invalidated in this action's `Nve-Invalidate` response header.
    #[inline]
    pub fn invalidate_many<T: CacheTag>(&self, tags: impl IntoIterator<Item = T>) {
        self.tags.invalidate_many(tags);
    }

    /// Emits wildcard `Nve-Invalidate: *`, instructing clients to invalidate all cached queries.
    #[inline]
    pub fn invalidate_all(&self) {
        self.tags.mark_all();
    }
}

/// Read-only metadata describing a registered procedure returned by [`RpcRegistry::lookup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcMeta {
    pub id: ProcId,
    pub name: &'static str,
    pub effect: Effect,
    pub supports_form: bool,
}

/// Outcome of invoking a type-erased binary RPC handler.
#[derive(Debug)]
pub(crate) enum RpcHandlerOutcome {
    Ok {
        body: Bytes,
        invalidated: TagSink,
    },
    DomainErr {
        status: RpcStatus,
        body: Bytes,
    },
    BadInput(CodecError),
    EncodeFailure(CodecError),
}

/// Outcome of invoking a type-erased HTML form action handler.
#[derive(Debug)]
pub(crate) enum FormHandlerOutcome {
    Ok {
        invalidated: TagSink,
    },
    FormDecodeErr {
        errors: FieldErrors,
        form: FormPairs,
    },
    DomainErr {
        status: RpcStatus,
        body: Bytes,
    },
    EncodeFailure(CodecError),
}

#[cfg(not(target_arch = "wasm32"))]
type ErasedRpcFn<S, U> =
    Box<dyn Fn(Cx<S, U>, Bytes) -> BoxFuture<'static, RpcHandlerOutcome> + Send + Sync + 'static>;

#[cfg(target_arch = "wasm32")]
type ErasedRpcFn<S, U> =
    Box<dyn Fn(Cx<S, U>, Bytes) -> BoxFuture<'static, RpcHandlerOutcome> + 'static>;

#[cfg(not(target_arch = "wasm32"))]
type ErasedFormFn<S, U> = Box<
    dyn Fn(Cx<S, U>, FormPairs) -> BoxFuture<'static, FormHandlerOutcome> + Send + Sync + 'static,
>;

#[cfg(target_arch = "wasm32")]
type ErasedFormFn<S, U> =
    Box<dyn Fn(Cx<S, U>, FormPairs) -> BoxFuture<'static, FormHandlerOutcome> + 'static>;

pub(crate) struct RegistryEntry<S, U> {
    pub(crate) id: ProcId,
    pub(crate) name: &'static str,
    pub(crate) effect: Effect,
    rpc: ErasedRpcFn<S, U>,
    form: Option<ErasedFormFn<S, U>>,
}

impl<S, U> RegistryEntry<S, U> {
    #[inline]
    pub(crate) fn meta(&self) -> ProcMeta {
        ProcMeta {
            id: self.id,
            name: self.name,
            effect: self.effect,
            supports_form: self.form.is_some(),
        }
    }

    #[inline]
    pub(crate) fn call_rpc(&self, cx: Cx<S, U>, body: Bytes) -> BoxFuture<'static, RpcHandlerOutcome> {
        (self.rpc)(cx, body)
    }

    #[inline]
    pub(crate) fn supports_form(&self) -> bool {
        self.form.is_some()
    }

    #[inline]
    pub(crate) fn call_form(
        &self,
        cx: Cx<S, U>,
        form: FormPairs,
    ) -> Option<BoxFuture<'static, FormHandlerOutcome>> {
        self.form.as_ref().map(|f| (f)(cx, form))
    }
}

/// Immutable sorted-slice procedure table supporting zero-allocation lookup by path.
pub struct RpcRegistry<S, U = (), C = CborCodec> {
    entries: Box<[(ProcId, RegistryEntry<S, U>)]>,
    codec: C,
}

impl<S, U> RpcRegistry<S, U, CborCodec>
where
    S: Clone + MaybeSend + MaybeSync + 'static,
    U: Clone + MaybeSend + MaybeSync + 'static,
{
    pub fn builder() -> RpcRegistryBuilder<S, U, CborCodec> {
        RpcRegistryBuilder::with_codec(CborCodec::default())
    }
}

impl<S, U, C> RpcRegistry<S, U, C> {
    #[inline]
    pub fn codec(&self) -> &C {
        &self.codec
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Zero-allocation sorted-slice binary search by procedure wire name (`"issues.v1.Issues/List"`).
    ///
    /// Verifies exact string equality after matching [`ProcId`] so hash collisions can never
    /// misroute a request.
    #[inline]
    pub fn lookup(&self, proc_name: &str) -> Option<ProcMeta> {
        let idx = self.lookup_index(proc_name)?;
        Some(self.entries[idx].1.meta())
    }

    #[inline]
    pub(crate) fn lookup_index(&self, proc_name: &str) -> Option<usize> {
        let id = ProcId::of(proc_name);
        let idx = self.entries.binary_search_by_key(&id, |(k, _)| *k).ok()?;
        if self.entries[idx].1.name == proc_name {
            Some(idx)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn entry_at(&self, idx: usize) -> &RegistryEntry<S, U> {
        &self.entries[idx].1
    }

    #[inline]
    pub(crate) fn entry_get(&self, idx: usize) -> Option<&RegistryEntry<S, U>> {
        self.entries.get(idx).map(|(_, entry)| entry)
    }
}

/// Builder for [`RpcRegistry`]. Panics in [`Self::build`] if any two registered procedures share
/// a [`ProcId`] or wire name.
pub struct RpcRegistryBuilder<S, U = (), C = CborCodec> {
    entries: Vec<(ProcId, RegistryEntry<S, U>)>,
    codec: C,
}

impl<S, U, C> RpcRegistryBuilder<S, U, C>
where
    S: Clone + MaybeSend + MaybeSync + 'static,
    U: Clone + MaybeSend + MaybeSync + 'static,
    C: Clone + MaybeSend + MaybeSync + 'static,
{
    pub fn with_codec(codec: C) -> Self {
        Self {
            entries: Vec::new(),
            codec,
        }
    }

    /// Registers a procedure handler accepting codec-encoded binary RPC bodies.
    pub fn register<P, F, Fut>(&mut self, handler: F) -> &mut Self
    where
        P: Procedure,
        C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
        F: Fn(Cx<S, U>, P::Args) -> Fut + MaybeSend + MaybeSync + 'static,
        Fut: Future<Output = Result<P::Output, P::Error>> + MaybeSend + 'static,
    {
        const {
            assert_valid_procedure_name(P::NAME);
            assert!(
                P::ID.raw() == ProcId::of(P::NAME).raw(),
                "Procedure::ID must match ProcId::of(Procedure::NAME)"
            );
        }
        let handler = Arc::new(handler);
        let rpc = erase_rpc::<P, S, U, C, F, Fut>(Arc::clone(&handler), self.codec.clone());
        self.entries.push((
            P::ID,
            RegistryEntry {
                id: P::ID,
                name: P::NAME,
                effect: P::Effect::EFFECT,
                rpc,
                form: None,
            },
        ));
        self
    }

    /// Registers an [`Action`] procedure handler accepting BOTH binary RPC bodies AND progressive
    /// 0-JS `application/x-www-form-urlencoded` submissions via [`FormDecode`].
    pub fn register_with_form<P, F, Fut>(&mut self, handler: F) -> &mut Self
    where
        P: Procedure<Effect = Action>,
        P::Args: FormDecode,
        C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
        F: Fn(Cx<S, U>, P::Args) -> Fut + MaybeSend + MaybeSync + 'static,
        Fut: Future<Output = Result<P::Output, P::Error>> + MaybeSend + 'static,
    {
        const {
            assert_valid_procedure_name(P::NAME);
            assert!(
                P::ID.raw() == ProcId::of(P::NAME).raw(),
                "Procedure::ID must match ProcId::of(Procedure::NAME)"
            );
        }
        let handler = Arc::new(handler);
        let rpc = erase_rpc::<P, S, U, C, F, Fut>(Arc::clone(&handler), self.codec.clone());
        let form = erase_form::<P, S, U, C, F, Fut>(handler, self.codec.clone());
        self.entries.push((
            P::ID,
            RegistryEntry {
                id: P::ID,
                name: P::NAME,
                effect: P::Effect::EFFECT,
                rpc,
                form: Some(form),
            },
        ));
        self
    }

    /// Sorts the procedure table by [`ProcId`] and panics on any duplicate ID or wire name.
    pub fn build(mut self) -> RpcRegistry<S, U, C> {
        self.entries.sort_by_key(|(id, _)| *id);
        for pair in self.entries.windows(2) {
            if pair[0].0 == pair[1].0 {
                panic!(
                    "duplicate ProcId {:016x} for procedures `{}` and `{}`",
                    pair[0].0.raw(),
                    pair[0].1.name,
                    pair[1].1.name
                );
            }
        }
        RpcRegistry {
            entries: self.entries.into_boxed_slice(),
            codec: self.codec,
        }
    }
}

fn erase_rpc<P, S, U, C, F, Fut>(handler: Arc<F>, codec: C) -> ErasedRpcFn<S, U>
where
    P: Procedure,
    S: Clone + MaybeSend + MaybeSync + 'static,
    U: Clone + MaybeSend + MaybeSync + 'static,
    C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
    F: Fn(Cx<S, U>, P::Args) -> Fut + MaybeSend + MaybeSync + 'static,
    Fut: Future<Output = Result<P::Output, P::Error>> + MaybeSend + 'static,
{
    Box::new(move |cx: Cx<S, U>, raw_body: Bytes| {
        let handler = Arc::clone(&handler);
        let codec = codec.clone();
        Box::pin(async move {
            let args: P::Args = match codec.decode_bounded(&raw_body) {
                Ok(a) => a,
                Err(err) => return RpcHandlerOutcome::BadInput(err),
            };
            let mut static_invalidates = TagSink::new();
            if !P::Effect::IS_QUERY {
                P::invalidates(&args, &mut static_invalidates);
            }
            let dynamic_tags = cx.tags.clone();

            match (handler)(cx, args).await {
                Ok(output) => {
                    dynamic_tags.drain_into(&mut static_invalidates);
                    let mut buf = Vec::with_capacity(64);
                    match codec.encode_into(&output, &mut buf) {
                        Ok(()) => RpcHandlerOutcome::Ok {
                            body: Bytes::from(buf),
                            invalidated: static_invalidates,
                        },
                        Err(err) => RpcHandlerOutcome::EncodeFailure(err),
                    }
                }
                Err(domain_err) => {
                    let status = domain_err.rpc_status();
                    let mut buf = Vec::with_capacity(64);
                    match codec.encode_into(&domain_err, &mut buf) {
                        Ok(()) => RpcHandlerOutcome::DomainErr {
                            status,
                            body: Bytes::from(buf),
                        },
                        Err(err) => RpcHandlerOutcome::EncodeFailure(err),
                    }
                }
            }
        })
    })
}

fn erase_form<P, S, U, C, F, Fut>(handler: Arc<F>, codec: C) -> ErasedFormFn<S, U>
where
    P: Procedure<Effect = Action>,
    P::Args: FormDecode,
    S: Clone + MaybeSend + MaybeSync + 'static,
    U: Clone + MaybeSend + MaybeSync + 'static,
    C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
    F: Fn(Cx<S, U>, P::Args) -> Fut + MaybeSend + MaybeSync + 'static,
    Fut: Future<Output = Result<P::Output, P::Error>> + MaybeSend + 'static,
{
    Box::new(move |cx: Cx<S, U>, form: FormPairs| {
        let handler = Arc::clone(&handler);
        let codec = codec.clone();
        Box::pin(async move {
            let args = match P::Args::from_form(&form) {
                Ok(a) => a,
                Err(errors) => return FormHandlerOutcome::FormDecodeErr { errors, form },
            };
            let mut static_invalidates = TagSink::new();
            P::invalidates(&args, &mut static_invalidates);
            let dynamic_tags = cx.tags.clone();

            match (handler)(cx, args).await {
                Ok(_output) => {
                    dynamic_tags.drain_into(&mut static_invalidates);
                    FormHandlerOutcome::Ok {
                        invalidated: static_invalidates,
                    }
                }
                Err(domain_err) => {
                    let status = domain_err.rpc_status();
                    let mut buf = Vec::with_capacity(64);
                    match codec.encode_into(&domain_err, &mut buf) {
                        Ok(()) => FormHandlerOutcome::DomainErr {
                            status,
                            body: Bytes::from(buf),
                        },
                        Err(err) => FormHandlerOutcome::EncodeFailure(err),
                    }
                }
            }
        })
    })
}
