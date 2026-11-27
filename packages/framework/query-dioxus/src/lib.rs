//! Dioxus 0.7 reactive query/action hooks, dense slot-signal bridge, portable [`QuerySuspense`]
//! streaming boundary, and progressive [`ActionForm`] built on [`nve_query`] and [`nve_ui`].
#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod action_form;
pub mod action_hooks;
pub mod bridge;
pub mod hooks;
pub mod sleep;
pub mod suspense;

pub use action_form::{ActionForm, ActionFormProps};
pub use action_hooks::{use_action, UseAction};
pub use bridge::{
    erased_query_client_with_clock, DioxusQueryBridge, DynTransport, ErasedQueryClient,
    QueryClientProvider, QueryClientProviderProps,
};
#[cfg(not(target_arch = "wasm32"))]
pub use bridge::{erased_query_client, erased_ssr_query_client};
pub use hooks::{use_query, use_query_client, use_query_with, QueryOutcome, UseQuery};
pub use nve_dioxus::{ErrorBoundaryReset, ErrorFallback, UiErrorReport};
pub use nve_query::{DehydrateCursor, DehydrateError};
pub use sleep::{default_sleep_callback, SleepCallback, SleepHandle};
pub use suspense::{
    BoundaryKey, BoundaryQuerySet, InDeferredSuspense, QuerySuspense, QuerySuspenseProps,
    SsrBoundaryEntry, SsrBoundaryRegistry, NVE_SUSPENSE_ATTR,
};

pub mod prelude {
    pub use crate::action_form::ActionForm;
    pub use crate::action_hooks::{use_action, UseAction};
    pub use crate::bridge::{
        erased_query_client_with_clock, DynTransport, ErasedQueryClient, QueryClientProvider,
    };
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::bridge::{erased_query_client, erased_ssr_query_client};
    pub use crate::hooks::{use_query, use_query_client, use_query_with, QueryOutcome, UseQuery};
    pub use crate::sleep::{default_sleep_callback, SleepCallback, SleepHandle};
    pub use crate::suspense::{
        BoundaryKey, BoundaryQuerySet, InDeferredSuspense, QuerySuspense, SsrBoundaryEntry,
        SsrBoundaryRegistry, NVE_SUSPENSE_ATTR,
    };
    pub use nve_dioxus::{ErrorBoundaryReset, ErrorFallback, UiErrorReport};
    pub use nve_query::{DehydrateCursor, DehydrateError};
}
