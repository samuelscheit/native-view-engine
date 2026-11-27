#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod actions;
pub mod engine;
pub mod hydrate;
pub mod optimistic;
pub mod slab;
pub(crate) mod state;
pub mod types;

pub use engine::{DehydrateError, QueryClient, WakeCallback};
pub use optimistic::{OptimisticHandle, OptimisticTx};
pub use slab::{QueryMeta, QuerySlotId};
pub use types::{
    Clock, FetchTicket, ManualClock, MutationId, QueryClientConfig, QueryHandle, QueryMode,
    QueryOptions, QuerySnapshot, Shared, SystemClock,
};
