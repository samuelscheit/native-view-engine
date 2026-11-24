use serde::{Deserialize, Serialize};

/// Unified 2-tier hot-reload wire protocol broadcast by `nve dev` over WebSocket
/// to Desktop Skia, iOS Simulator/Device, Android Emulator/Device, and Web clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HotPatchMsg {
    /// Tier 1 (<50ms, zero recompile, works on physical iOS devices):
    /// Patches a text or style literal inside a live node/template without restarting Rust.
    TemplatePatch {
        generation: u64,
        template_id: String,
        attr_updates: Vec<(String, String)>,
        text_updates: Vec<(usize, String)>,
    },
    /// Tier 2 (150-400ms, desktop + iOS Simulator + Android):
    /// Instructs the host shell to hot-swap the compiled Rust app cdylib while preserving state.
    DylibReload {
        generation: u64,
        dylib_path: String,
        build_ms: u64,
    },
    /// Heartbeat / diagnostic banner sent to the live on-screen HUD.
    DevStatus {
        message: String,
        latency_ms: u64,
    },
}

impl HotPatchMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("HotPatchMsg serialization cannot fail")
    }

    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}
