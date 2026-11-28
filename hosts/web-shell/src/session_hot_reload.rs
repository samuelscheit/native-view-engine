//! Dev-only v2 binary hot-reload action types and `WebAppSession::handle_hot_patch*` methods.

use nve_engine::HotPatchMsg;

use super::WebAppSession;

/// Outcome of handling one `nve dev` WebSocket `HotPatchMsg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotReloadAction {
    /// Tier-1 binary `RowPatch` / text patch applied live without reloading the page.
    TemplatePatched {
        generation: u64,
        template_key: u64,
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
    /// Decodes and applies one binary v2 hot-patch frame from `nve dev`.
    pub fn handle_hot_patch_frame(&mut self, raw: &[u8]) -> Result<HotReloadAction, String> {
        let msg = HotPatchMsg::from_bytes(raw).map_err(|e| format!("invalid NVP1 frame: {e}"))?;
        self.handle_hot_patch(msg)
    }

    /// Applies one decoded `HotPatchMsg` from `nve dev` through the unified `app.apply_hot_patch`
    /// -> `render_and_submit()` path (identical to desktop Skia and native hosts).
    pub fn handle_hot_patch(&mut self, msg: HotPatchMsg) -> Result<HotReloadAction, String> {
        match msg {
            ref msg @ HotPatchMsg::TemplatePatch {
                generation,
                template_key,
                ..
            } => {
                let mut mutations = 0u32;
                if self.app.apply_hot_patch(msg) {
                    let timing = self
                        .render_and_submit()
                        .map_err(|e| format!("failed to commit hot patch: {e}"))?;
                    mutations = timing.mutation_count;
                }
                Ok(HotReloadAction::TemplatePatched {
                    generation,
                    template_key,
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
