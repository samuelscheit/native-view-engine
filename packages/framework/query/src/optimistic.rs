//! Rebaseable layered optimistic patch stack so concurrent overlapping mutations and mid-flight
//! background refetches never clobber sibling mutations or flicker.

use core::any::Any;

use nve_rpc::{
    encode_args, CacheTag, MaybeSend, MaybeSync, ProcId, Procedure, Query, RpcCodec, Tag64,
};

use crate::slab::QueryPayload;
use crate::types::{into_shared_any, MutationId, QueryHandle, SharedAny};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type PatchFn = Box<dyn Fn(&dyn Any) -> Option<SharedAny> + Send + Sync>;

#[cfg(target_arch = "wasm32")]
pub(crate) type PatchFn = Box<dyn Fn(&dyn Any) -> Option<SharedAny>>;

pub(crate) struct OptimisticLayer {
    pub mutation_id: MutationId,
    /// When `Some(gen)`, this layer was created by a mutation that succeeded and invalidated this
    /// slot at `fetch_gen == gen`; it retires automatically once a fetch `>= gen` commits.
    pub settle_on_fetch_gen: Option<u32>,
    pub apply: PatchFn,
}

impl OptimisticLayer {
    pub(crate) fn typed<T, F>(mutation_id: MutationId, f: F) -> Self
    where
        T: MaybeSend + MaybeSync + 'static,
        F: Fn(&T) -> T + MaybeSend + MaybeSync + 'static,
    {
        Self {
            mutation_id,
            settle_on_fetch_gen: None,
            apply: Box::new(move |cur: &dyn Any| {
                let typed = cur.downcast_ref::<T>()?;
                Some(into_shared_any(f(typed)))
            }),
        }
    }
}

/// Recomputes `payload.effective_optimistic` by folding `payload.committed` through all active
/// `optimistic_layers` in submission order.
pub(crate) fn recompute_effective(payload: &mut QueryPayload) {
    if payload.optimistic_layers.is_empty() {
        payload.effective_optimistic = None;
        return;
    }
    let Some(committed) = payload.committed.as_ref() else {
        payload.effective_optimistic = None;
        return;
    };

    let mut cur: SharedAny = crate::types::Shared::clone(committed);
    for layer in &payload.optimistic_layers {
        if let Some(next) = (layer.apply)(cur.as_ref()) {
            cur = next;
        }
    }
    payload.effective_optimistic = Some(cur);
}

/// Retires optimistic layers settled by a fetch at `completed_fetch_gen`, then rebases any
/// still-pending sibling layers on top of the freshly committed value.
pub(crate) fn settle_layers_up_to(payload: &mut QueryPayload, completed_fetch_gen: u32) {
    if !payload.optimistic_layers.is_empty() {
        payload.optimistic_layers.retain(|layer| match layer.settle_on_fetch_gen {
            Some(target_gen) => completed_fetch_gen < target_gen,
            None => true,
        });
    }
    recompute_effective(payload);
}

/// Removes every optimistic layer owned by `mutation_id` (rollback on error) and rebases any
/// remaining sibling layers on top of `committed`.
pub(crate) fn remove_mutation_layers(payload: &mut QueryPayload, mutation_id: MutationId) -> bool {
    if payload.optimistic_layers.is_empty() {
        return false;
    }
    let before = payload.optimistic_layers.len();
    payload
        .optimistic_layers
        .retain(|l| l.mutation_id != mutation_id);
    let changed = payload.optimistic_layers.len() != before;
    if changed {
        recompute_effective(payload);
    }
    changed
}

/// Marks every layer owned by `mutation_id` on `payload` to settle once `settle_gen` commits.
pub(crate) fn mark_mutation_settling(
    payload: &mut QueryPayload,
    mutation_id: MutationId,
    settle_gen: u32,
) -> bool {
    if payload.optimistic_layers.is_empty() {
        return false;
    }
    let mut touched = false;
    for layer in &mut payload.optimistic_layers {
        if layer.mutation_id == mutation_id {
            layer.settle_on_fetch_gen = Some(settle_gen);
            touched = true;
        }
    }
    touched
}

/// Opaque handle returned by an optimistic update transaction, tracking all touched query slots
/// so the caller (or `execute_action`) can settle or roll back in O(touched slots).
#[derive(Debug, Clone)]
pub struct OptimisticHandle {
    pub(crate) mutation_id: MutationId,
    pub(crate) touched: Vec<QueryHandle>,
}

impl OptimisticHandle {
    #[inline]
    pub fn mutation_id(&self) -> MutationId {
        self.mutation_id
    }

    #[inline]
    pub fn touched_count(&self) -> usize {
        self.touched.len()
    }
}

pub(crate) enum PendingPatchOp {
    ExactKey {
        proc_id: ProcId,
        encoded_args: bytes::Bytes,
        make_layer: Box<dyn Fn(MutationId) -> OptimisticLayer>,
    },
    ByTag {
        proc_id: ProcId,
        tag: Tag64,
        make_layer: Box<dyn Fn(MutationId) -> OptimisticLayer>,
    },
    AllOfProc {
        proc_id: ProcId,
        make_layer: Box<dyn Fn(MutationId) -> OptimisticLayer>,
    },
}

/// Declarative builder for recording typed optimistic patch layers against cached queries.
///
/// Built outside the cache lock so user closures never run while the cache is borrowed/locked.
pub struct OptimisticTx<'a, C> {
    codec: &'a C,
    pub(crate) ops: Vec<PendingPatchOp>,
}

impl<'a, C> OptimisticTx<'a, C> {
    #[inline]
    pub(crate) fn new(codec: &'a C) -> Self {
        Self {
            codec,
            ops: Vec::new(),
        }
    }

    /// Schedules an optimistic patch on the exact cached query `(P, args)`.
    pub fn update_query<P>(
        &mut self,
        args: &P::Args,
        transform: impl Fn(&P::Output) -> P::Output + MaybeSend + MaybeSync + Clone + 'static,
    ) -> &mut Self
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Args>,
    {
        if let Ok(encoded_args) = encode_args::<P, C>(self.codec, args) {
            self.ops.push(PendingPatchOp::ExactKey {
                proc_id: P::ID,
                encoded_args,
                make_layer: Box::new(move |id| {
                    OptimisticLayer::typed::<P::Output, _>(id, transform.clone())
                }),
            });
        }
        self
    }

    /// Schedules an optimistic patch on every cached query of procedure `P` carrying `tag`.
    pub fn update_queries_by_tag<P>(
        &mut self,
        tag: impl CacheTag,
        transform: impl Fn(&P::Output) -> P::Output + MaybeSend + MaybeSync + Clone + 'static,
    ) -> &mut Self
    where
        P: Procedure<Effect = Query>,
    {
        self.ops.push(PendingPatchOp::ByTag {
            proc_id: P::ID,
            tag: tag.tag64(),
            make_layer: Box::new(move |id| {
                OptimisticLayer::typed::<P::Output, _>(id, transform.clone())
            }),
        });
        self
    }

    /// Schedules an optimistic patch on every currently cached query of procedure `P` (regardless of
    /// filter arguments, e.g. updating an item across all open/closed `ListIssues` views).
    pub fn update_each<P>(
        &mut self,
        transform: impl Fn(&P::Output) -> P::Output + MaybeSend + MaybeSync + Clone + 'static,
    ) -> &mut Self
    where
        P: Procedure<Effect = Query>,
    {
        self.ops.push(PendingPatchOp::AllOfProc {
            proc_id: P::ID,
            make_layer: Box::new(move |id| {
                OptimisticLayer::typed::<P::Output, _>(id, transform.clone())
            }),
        });
        self
    }
}

pub(crate) fn apply_layer_to_slot(payload: &mut QueryPayload, layer: OptimisticLayer) {
    payload.optimistic_layers.push(layer);
    recompute_effective(payload);
}
