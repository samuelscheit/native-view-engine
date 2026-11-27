//! Rebaseable layered optimistic patch stack so concurrent overlapping mutations and mid-flight
//! background refetches never clobber sibling mutations or flicker.

use core::any::Any;

use nve_rpc::{
    encode_args, CacheTag, CodecError, MaybeSend, MaybeSync, ProcId, Procedure, Query, RpcCodec,
    Tag64,
};

use crate::slab::{QueryMeta, QueryPayload, QuerySlotId};
use crate::types::{into_shared_any, MutationId, QueryHandle, Shared, SharedAny};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type PatchFn = Shared<dyn Fn(&dyn Any) -> Option<SharedAny> + Send + Sync>;

#[cfg(target_arch = "wasm32")]
pub(crate) type PatchFn = Shared<dyn Fn(&dyn Any) -> Option<SharedAny>>;

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
            apply: Shared::new(move |cur: &dyn Any| {
                let typed = cur.downcast_ref::<T>()?;
                Some(into_shared_any(f(typed)))
            }),
        }
    }
}

/// Snapshot of a slot's committed value and optimistic layer closures prepared under the cache
/// lock so all user transforms execute strictly OUTSIDE the lock (preventing native deadlocks and
/// `wasm32` `RefCell` re-entrancy panics).
pub(crate) struct RecomputeJob {
    pub slot: QuerySlotId,
    pub generation: u32,
    pub rev: u16,
    pub committed: SharedAny,
    pub layers: Vec<PatchFn>,
}

/// Prepares a lock-free [`RecomputeJob`] for `slot`, or clears `effective_optimistic` immediately
/// when no layers or no committed value exist.
pub(crate) fn prepare_recompute_job(
    slot: QuerySlotId,
    meta: &mut QueryMeta,
    payload: &mut QueryPayload,
) -> Option<RecomputeJob> {
    meta.optimistic_rev = meta.optimistic_rev.wrapping_add(1);
    if payload.optimistic_layers.is_empty() {
        payload.effective_optimistic = None;
        return None;
    }
    let Some(committed) = payload.committed.as_ref() else {
        payload.effective_optimistic = None;
        return None;
    };
    let layers = payload
        .optimistic_layers
        .iter()
        .map(|l| Shared::clone(&l.apply))
        .collect();
    Some(RecomputeJob {
        slot,
        generation: meta.slot_gen,
        rev: meta.optimistic_rev,
        committed: Shared::clone(committed),
        layers,
    })
}

/// Executes every optimistic layer closure in submission order outside the cache lock.
pub(crate) fn fold_recompute_job(job: &RecomputeJob) -> SharedAny {
    let mut cur = Shared::clone(&job.committed);
    for layer in &job.layers {
        if let Some(next) = (layer)(cur.as_ref()) {
            cur = next;
        }
    }
    cur
}

/// Retires optimistic layers settled by a fetch at `completed_fetch_gen` (caller recomputes
/// effective value outside the lock via [`prepare_recompute_job`]).
pub(crate) fn settle_layers_up_to(payload: &mut QueryPayload, completed_fetch_gen: u32) {
    if !payload.optimistic_layers.is_empty() {
        payload
            .optimistic_layers
            .retain(|layer| match layer.settle_on_fetch_gen {
                Some(target_gen) => (completed_fetch_gen.wrapping_sub(target_gen) as i32) < 0,
                None => true,
            });
    }
}

/// Removes every optimistic layer owned by `mutation_id` (caller recomputes effective value
/// outside the lock via [`prepare_recompute_job`]).
pub(crate) fn remove_mutation_layers(payload: &mut QueryPayload, mutation_id: MutationId) -> bool {
    if payload.optimistic_layers.is_empty() {
        return false;
    }
    let before = payload.optimistic_layers.len();
    payload
        .optimistic_layers
        .retain(|l| l.mutation_id != mutation_id);
    payload.optimistic_layers.len() != before
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
    /// Returns `Err(CodecError)` if encoding `args` fails rather than silently dropping the patch.
    pub fn update_query<P>(
        &mut self,
        args: &P::Args,
        transform: impl Fn(&P::Output) -> P::Output + MaybeSend + MaybeSync + Clone + 'static,
    ) -> Result<&mut Self, CodecError>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Args>,
    {
        let encoded_args = encode_args::<P, C>(self.codec, args)?;
        self.ops.push(PendingPatchOp::ExactKey {
            proc_id: P::ID,
            encoded_args,
            make_layer: Box::new(move |id| {
                OptimisticLayer::typed::<P::Output, _>(id, transform.clone())
            }),
        });
        Ok(self)
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
}
