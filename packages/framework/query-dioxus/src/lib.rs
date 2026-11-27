//! Dioxus 0.7 reactive query/action hooks, dense slot-signal bridge, and progressive
//! [`ActionForm`] built on [`nve_query`] and [`nve_ui`].

pub mod action_form;
pub mod bridge;
pub mod hooks;

pub use action_form::{ActionForm, ActionFormProps};
pub use bridge::{
    erased_query_client_with_clock, DioxusQueryBridge, DynTransport, ErasedQueryClient,
    QueryClientProvider, QueryClientProviderProps,
};
#[cfg(not(target_arch = "wasm32"))]
pub use bridge::{erased_query_client, erased_ssr_query_client};
pub use hooks::{use_action, use_query, use_query_client, use_query_with, UseAction, UseQuery};

pub mod prelude {
    pub use crate::action_form::ActionForm;
    pub use crate::bridge::{
        erased_query_client_with_clock, DynTransport, ErasedQueryClient, QueryClientProvider,
    };
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::bridge::{erased_query_client, erased_ssr_query_client};
    pub use crate::hooks::{
        use_action, use_query, use_query_client, use_query_with, UseAction, UseQuery,
    };
}
