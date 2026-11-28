//! Web WASM shell mounting NVE apps onto browser DOM with live interactive events and SSR
//! hydration. Cargo features select what a bundle contains (see `Cargo.toml`): one app per
//! production bundle (`app-studio` / `app-kitchen-sink`), plus `dev` (binary `NVP1` hot reload,
//! `NveWireFrame` pairing) and `multi-window` (popups + `WindowManager` window sync).

#[cfg(not(any(feature = "app-studio", feature = "app-kitchen-sink", feature = "app-issues")))]
compile_error!(
    "nve-web-shell bundles at least one app: enable `app-studio`, `app-kitchen-sink`, and/or `app-issues`"
);

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();

#[cfg(feature = "web-host-services")]
pub mod host_services;
#[cfg(all(test, feature = "web-host-services"))]
mod host_services_tests;
pub mod hydrate;
#[cfg(feature = "multi-window")]
pub mod multi_window;
#[cfg(feature = "dev")]
pub mod pairing;
pub mod session;
/// The issues family's web plugin set, exactly as the browser shell boots it (its SSR server's
/// `issues_web_backend` must register the same tables).
#[cfg(feature = "app-issues")]
pub use nve_example_issues_client::register_web_systems as register_issues_web_systems;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(any(
    all(test, not(target_arch = "wasm32")),
    all(target_arch = "wasm32", feature = "app-issues")
))]
pub(crate) mod query_stream_state;
#[cfg(any(
    all(test, not(target_arch = "wasm32")),
    all(target_arch = "wasm32", feature = "app-issues")
))]
pub(crate) mod wake_sched;
#[cfg(target_arch = "wasm32")]
pub mod wasm_api;
#[cfg(all(target_arch = "wasm32", feature = "dev"))]
mod wasm_hotreload;
#[cfg(target_arch = "wasm32")]
pub mod wasm_hydrate;
#[cfg(all(target_arch = "wasm32", feature = "app-issues"))]
pub mod wasm_issues;
#[cfg(all(target_arch = "wasm32", feature = "issues-probes"))]
pub mod wasm_close_bridge_probe;
#[cfg(all(target_arch = "wasm32", feature = "issues-probes"))]
pub mod wasm_dom_probe;
#[cfg(all(target_arch = "wasm32", feature = "issues-probes"))]
pub mod wasm_overlay_probe;
#[cfg(all(target_arch = "wasm32", feature = "issues-probes"))]
pub mod wasm_tv_probe;
#[cfg(all(target_arch = "wasm32", feature = "app-issues"))]
mod wasm_query_stream;
#[cfg(all(target_arch = "wasm32", feature = "dev"))]
pub mod wasm_pair;
#[cfg(all(target_arch = "wasm32", feature = "multi-window"))]
pub mod wasm_popup;
#[cfg(all(target_arch = "wasm32", feature = "dev"))]
pub mod wasm_probe;
#[cfg(target_arch = "wasm32")]
mod wasm_snapshot;
#[cfg(target_arch = "wasm32")]
mod wasm_wake;
#[cfg(target_arch = "wasm32")]
mod wasm_appearance;

#[cfg(feature = "web-host-services")]
pub use host_services::WebHostServices;
pub use hydrate::{CapturedInteraction, HydrateTrigger, IslandHydrationScheduler, IslandSpec};

/// Plugin set for dev sessions rendering arbitrary remote apps (pairing, probes): `nve-web-html`,
/// canvas, and motion. App sessions register their family's shared set
/// (`SessionApp::register_web_systems`).
#[cfg(feature = "dev")]
pub fn register_dev_web_systems(backend: &mut nve_backend_web::WebBackend) {
    nve_web_html_web::register_web_html_systems(backend.table_registry_mut());
    backend.register_table_system(nve_canvas_web::CanvasWebSystem::new());
    backend.register_table_system(nve_motion_web::MotionWebSystem::new());
}
#[cfg(feature = "multi-window")]
pub use multi_window::{
    PopupWindowError, PopupWindowSpec, WebMultiWindowHost, DEFAULT_WINDOW_SYNC_CHANNEL,
};
#[cfg(feature = "dev")]
pub use pairing::{
    PairClientMachine, PairClientState, PairInboundOutcome, PairedWebSession,
};
#[cfg(feature = "dev")]
pub use session::HotReloadAction;
pub use session::{WebAppSession, DEFAULT_SURFACE_ID};
#[cfg(target_arch = "wasm32")]
pub use wasm_api::{active_shell_count, active_surface_ids, WebAppController};
#[cfg(all(target_arch = "wasm32", feature = "app-kitchen-sink"))]
pub use wasm_api::{start_kitchen_sink_app, start_kitchen_sink_app_on_surface};
#[cfg(all(target_arch = "wasm32", feature = "app-studio"))]
pub use wasm_api::{start_studio_workspace_app, start_studio_workspace_app_on_surface};
#[cfg(all(target_arch = "wasm32", feature = "app-issues"))]
pub use wasm_issues::start_issues_app;
#[cfg(all(target_arch = "wasm32", feature = "issues-probes"))]
pub use wasm_dom_probe::{nve_probe_container_root_hydration, nve_probe_native_overlays};
#[cfg(all(target_arch = "wasm32", feature = "issues-probes"))]
pub use wasm_issues::{nve_probe_cross_origin_cors, nve_probe_fetch_transport};
#[cfg(target_arch = "wasm32")]
pub use wasm_hydrate::{ProgressiveIslandHost, SsrRenderResult};
#[cfg(all(target_arch = "wasm32", feature = "app-kitchen-sink"))]
pub use wasm_hydrate::{
    hydrate_kitchen_sink_app, hydrate_kitchen_sink_app_on_surface, render_kitchen_sink_ssr_bundle,
};
#[cfg(all(target_arch = "wasm32", feature = "app-studio"))]
pub use wasm_hydrate::{
    hydrate_studio_workspace_app, hydrate_studio_workspace_app_on_surface,
    render_studio_ssr_bundle, render_studio_ssr_document,
};
#[cfg(all(target_arch = "wasm32", feature = "dev"))]
pub use wasm_pair::NveWebHost;
#[cfg(all(target_arch = "wasm32", feature = "multi-window"))]
pub use wasm_popup::{
    broadcast_window_commands, close_popup_window, open_popup_window, WebPopupController,
};
#[cfg(all(target_arch = "wasm32", feature = "dev"))]
pub use wasm_probe::run_semantic_style_hydration_probe;
