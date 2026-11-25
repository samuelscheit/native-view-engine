//! Web WASM shell mounting NVE apps onto browser DOM with live interactive events,
//! zero-copy `NveSceneView` linear-memory exports, and 2-tier WebSocket hot-reload
//! (`HotPatchMsg::TemplatePatch` + `DylibReload` + `DevStatus`).

pub mod hydrate;
pub mod multi_window;
pub mod pairing;
pub mod session;
#[cfg(target_arch = "wasm32")]
pub mod wasm_api;
#[cfg(target_arch = "wasm32")]
pub mod wasm_hydrate;
#[cfg(target_arch = "wasm32")]
pub mod wasm_pair;
#[cfg(target_arch = "wasm32")]
pub mod wasm_popup;

pub use hydrate::{
    CapturedInteraction, HydrateTrigger, IslandHydrationScheduler, IslandSpec,
};
#[cfg(target_arch = "wasm32")]
pub use wasm_hydrate::{
    hydrate_counter_app, hydrate_counter_app_on_surface, hydrate_studio_workspace_app,
    hydrate_studio_workspace_app_on_surface, render_counter_ssr_bundle, render_studio_ssr_bundle,
    render_studio_ssr_document, run_semantic_style_hydration_probe, ProgressiveIslandHost,
    SsrRenderResult,
};

pub use multi_window::{
    PopupWindowError, PopupWindowSpec, WebMultiWindowHost, DEFAULT_WINDOW_SYNC_CHANNEL,
};
pub use pairing::{
    encode_nvt1_envelope, PairClientMachine, PairClientState, PairInboundOutcome, PairedWebSession,
};
pub use session::{
    is_preinitialized, take_preinit_frame0_command_buffer, take_preinit_frame0_command_buffer_for,
    wizer_initialize, HotReloadAction, PreinitApp, WebAppSession, PREINIT_SURFACE_ID,
};
#[cfg(target_arch = "wasm32")]
pub use wasm_api::{
    active_shell_count, active_surface_ids, nve_web_scene_view_ptr,
    nve_web_scene_view_ptr_for_surface, start_counter_app, start_counter_app_on_surface,
    start_studio_workspace_app, start_studio_workspace_app_on_surface, WebAppController,
};
#[cfg(target_arch = "wasm32")]
pub use wasm_pair::NveWebHost;
#[cfg(target_arch = "wasm32")]
pub use wasm_popup::{
    broadcast_window_commands, close_popup_window, open_popup_window, WebPopupController,
};
