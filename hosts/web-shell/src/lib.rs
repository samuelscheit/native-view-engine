//! Web WASM shell mounting NVE apps onto browser DOM with live interactive events,
//! zero-copy `NveSceneView` linear-memory exports, and 2-tier WebSocket hot-reload
//! (`HotPatchMsg::TemplatePatch` + `DylibReload` + `DevStatus`).

pub mod session;
#[cfg(target_arch = "wasm32")]
pub mod wasm_api;

pub use session::{
    is_preinitialized, take_preinit_frame0_command_buffer, take_preinit_frame0_command_buffer_for,
    wizer_initialize, HotReloadAction, PreinitApp, WebAppSession, PREINIT_SURFACE_ID,
};
#[cfg(target_arch = "wasm32")]
pub use wasm_api::{
    nve_web_scene_view_ptr, start_counter_app, start_studio_workspace_app, WebAppController,
};
