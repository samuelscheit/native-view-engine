//! Web WASM shell mounting NVE apps onto browser DOM with live interactive events and SSR
//! hydration. Cargo features select what a bundle contains (see `Cargo.toml`): one app per
//! production bundle (`app-studio` / `app-kitchen-sink`), plus `dev` (binary `NVP1` hot reload,
//! `NVT1`/`NVD1` pairing), `multi-window` (popups + `NVW1` window sync) and `scene-view`
//! (zero-copy `NveSceneView` linear-memory export).

#[cfg(not(any(feature = "app-studio", feature = "app-kitchen-sink", feature = "app-issues")))]
compile_error!(
    "nve-web-shell bundles at least one app: enable `app-studio`, `app-kitchen-sink`, and/or `app-issues`"
);

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();

pub mod hydrate;
#[cfg(feature = "multi-window")]
pub mod multi_window;
#[cfg(feature = "dev")]
pub mod pairing;
#[cfg(feature = "scene-view")]
pub mod scene;
pub mod session;
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

pub use hydrate::{CapturedInteraction, HydrateTrigger, IslandHydrationScheduler, IslandSpec};
#[cfg(feature = "multi-window")]
pub use multi_window::{
    PopupWindowError, PopupWindowSpec, WebMultiWindowHost, DEFAULT_WINDOW_SYNC_CHANNEL,
};
#[cfg(feature = "dev")]
pub use pairing::{
    PairClientMachine, PairClientState, PairInboundOutcome, PairedWebSession,
};
#[cfg(feature = "scene-view")]
pub use scene::SceneMirror;
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
pub use wasm_issues::{nve_probe_cross_origin_cors, nve_probe_fetch_transport};
#[cfg(all(target_arch = "wasm32", feature = "scene-view"))]
pub use wasm_api::{nve_web_scene_view_ptr, nve_web_scene_view_ptr_for_surface};
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
