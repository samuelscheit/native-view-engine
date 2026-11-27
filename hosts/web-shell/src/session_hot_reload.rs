//! Dev-only `NVP1` binary hot-reload action types and `WebAppSession::handle_hot_patch*` methods.

use nve_engine::HotPatchMsg;
use nve_store::ChangeSet;

use super::WebAppSession;

/// Outcome of handling one `nve dev` WebSocket `HotPatchMsg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotReloadAction {
    /// Tier-1 template/style patch applied live without reloading the page.
    TemplatePatched {
        generation: u64,
        template_id: String,
        mutations: u32,
    },
    /// Tier-2 dylib/WASM rebuild finished; browser shell should log and reload cleanly.
    ReloadPage {
        generation: u64,
        dylib_path: String,
        build_ms: u64,
    },
    /// Status heartbeat from `nve dev`.
    DevStatus { message: String, latency_ms: u64 },
}

impl WebAppSession {
    /// Decodes and applies one binary `NVP1` hot-patch frame from `nve dev`.
    pub fn handle_hot_patch_frame(&mut self, raw: &[u8]) -> Result<HotReloadAction, String> {
        let msg = HotPatchMsg::from_bytes(raw).map_err(|e| format!("invalid NVP1 frame: {e}"))?;
        self.handle_hot_patch(msg)
    }

    /// Applies one decoded `HotPatchMsg` from `nve dev`.
    pub fn handle_hot_patch(&mut self, msg: HotPatchMsg) -> Result<HotReloadAction, String> {
        match msg {
            ref msg @ HotPatchMsg::TemplatePatch {
                generation,
                ref template_id,
                ..
            } => {
                let template_id = template_id.clone();
                let mut patch_changes = ChangeSet::default();
                let mutations = self.backend.apply_template_patch_into(
                    self.app.store_mut(),
                    msg,
                    &mut patch_changes,
                );
                let next_frame = self.frame_id.wrapping_add(1).max(1);
                self.frame_id = next_frame;
                #[cfg(feature = "scene-view")]
                self.scene
                    .sync(self.app.store(), &patch_changes, next_frame);
                Ok(HotReloadAction::TemplatePatched {
                    generation,
                    template_id,
                    mutations,
                })
            }
            HotPatchMsg::DylibReload {
                generation,
                dylib_path,
                build_ms,
            } => Ok(HotReloadAction::ReloadPage {
                generation,
                dylib_path,
                build_ms,
            }),
            HotPatchMsg::DevStatus {
                message,
                latency_ms,
            } => Ok(HotReloadAction::DevStatus {
                message,
                latency_ms,
            }),
        }
    }
}
