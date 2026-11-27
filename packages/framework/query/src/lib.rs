#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod actions;
pub mod engine;
pub mod fetch;
pub mod hydrate;
pub mod invalidation;
pub mod optimistic;
pub mod slab;
pub(crate) mod state;
pub mod types;

pub use engine::{DehydrateError, QueryClient, WakeCallback};
pub use hydrate::{nveq_entry_count, DehydrateCursor};
pub use optimistic::{OptimisticHandle, OptimisticTx};
pub use slab::{QueryMeta, QuerySlotId};
#[cfg(not(target_arch = "wasm32"))]
pub use types::SystemClock;
pub use types::{
    Clock, FetchTicket, ManualClock, MutationId, QueryClientConfig, QueryHandle, QueryMode,
    QueryOptions, QuerySnapshot, Shared,
};
