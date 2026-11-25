//! Type-safe stack, tab, modal, and deep-link router (`nve-router`) for `nve-dioxus`, natively
//! bridged to `nve-interop::NveChannelBus` (`CHANNEL_ID_NAVIGATION = 4`) via the compact binary
//! `NVN1` wire format.

pub mod channel;
pub mod deeplink;
pub mod dioxus_ui;
pub mod guard;
pub mod nvn1;
pub mod query;
pub mod route;
mod route_macro;
pub mod stack;
pub mod tabs;

pub use channel::{
    decode_nav_packet, decode_nav_packet_ref, encode_nav_packet, encode_nav_packet_into,
    BridgeError, InboundApplyOutcome, InboundNavCommand, NavPacket, NavPacketRef, Nvn1DecodeError,
    Nvn1EncodeError, RouterChannelBridge, NVN1_FLAG_CAN_GO_BACK, NVN1_FLAG_CAN_GO_FORWARD,
    NVN1_FLAG_IS_MODAL, NVN1_MAGIC, NVN1_MIN_PACKET_BYTES, NVN1_VERSION,
};
pub use deeplink::{DeepLinkConfig, DeepLinkError, DeepLinkMatch, DeepLinkOrigin};
pub use dioxus_ui::{
    try_use_navigator, use_active_modal, use_active_tab, use_navigator, use_route,
    use_tab_navigator, Link, Navigator, Outlet, RouteRenderer, RouterProvider, TabNavigator,
    TabRouterProvider,
};
pub use guard::{
    guard_fn, FnGuard, GuardChain, GuardDecision, GuardOutcome, NavigationContext, NavigationGuard,
    DEFAULT_MAX_REDIRECT_HOPS,
};
pub use query::{
    is_unreserved, percent_decode, percent_encode, percent_encode_into, split_uri,
    PercentDecodeError, QueryParams, RawQueryIter, UriParts,
};
pub use route::{normalize_path, Routable};
pub use stack::{
    NavigationAction, NavigationError, NavigationTransition, RouteEntry, RouterState,
};
pub use tabs::{TabRouterState, TabSpec};

pub mod prelude {
    pub use crate::channel::{NavPacket, RouterChannelBridge};
    pub use crate::deeplink::{DeepLinkConfig, DeepLinkError, DeepLinkMatch, DeepLinkOrigin};
    pub use crate::define_routes;
    pub use crate::dioxus_ui::{
        try_use_navigator, use_active_modal, use_active_tab, use_navigator, use_route,
        use_tab_navigator, Link, Navigator, Outlet, RouteRenderer, RouterProvider, TabNavigator,
        TabRouterProvider,
    };
    pub use crate::guard::{guard_fn, GuardDecision, NavigationContext, NavigationGuard};
    pub use crate::query::QueryParams;
    pub use crate::route::Routable;
    pub use crate::stack::{NavigationAction, NavigationError, NavigationTransition, RouterState};
    pub use crate::tabs::{TabRouterState, TabSpec};
}
