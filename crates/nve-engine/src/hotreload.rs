use serde::{Deserialize, Serialize};

/// Unified 2-tier hot-reload wire protocol broadcast by `nve dev` over WebSocket
/// to Desktop Skia, iOS Simulator/Device, Android Emulator/Device, and Web clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HotPatchMsg {
    /// Tier 1 (<10ms, zero recompile, works on physical iOS devices & browsers):
    /// Patches targeted text or style literals inside a live node/template without restarting Rust.
    TemplatePatch {
        generation: u64,
        template_id: String,
        attr_updates: Vec<(String, String)>,
        text_updates: Vec<(usize, String)>,
        /// Optional `test_id` / `data-testid` / `testID` targeting a specific RSX element.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_test_id: Option<String>,
        /// Optional structural child index path inside the template tree (from root).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        node_path: Vec<usize>,
        /// Exact `(old_text, new_text)` static text replacements for surgical multi-node patching.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        text_replacements: Vec<(String, String)>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_patch_targeted_fields_roundtrip_and_default_cleanly() {
        let targeted = HotPatchMsg::TemplatePatch {
            generation: 7,
            template_id: "examples/studio-workspace/src/ui.rs".into(),
            attr_updates: vec![("style".into(), "padding: 16px; background-color: #111827;".into())],
            text_updates: vec![(2, "AETHER PRO".into())],
            target_test_id: Some("workspace-root".into()),
            node_path: vec![0, 1, 0],
            text_replacements: vec![("AETHER".into(), "AETHER PRO".into())],
        };
        let json = targeted.to_json();
        assert!(json.contains("\"target_test_id\":\"workspace-root\""));
        assert!(json.contains("\"node_path\":[0,1,0]"));
        assert!(json.contains("\"text_replacements\":[[\"AETHER\",\"AETHER PRO\"]]"));
        let decoded = HotPatchMsg::from_json(&json).expect("targeted patch roundtrip");
        assert_eq!(decoded, targeted);

        // Legacy wire payload omitting the optional targeted fields must deserialize cleanly.
        let legacy_json = r#"{"type":"template_patch","generation":3,"template_id":"card","attr_updates":[["opacity","0.5"]],"text_updates":[[0,"Hi"]]}"#;
        let legacy = HotPatchMsg::from_json(legacy_json).expect("legacy JSON deserialize");
        assert_eq!(
            legacy,
            HotPatchMsg::TemplatePatch {
                generation: 3,
                template_id: "card".into(),
                attr_updates: vec![("opacity".into(), "0.5".into())],
                text_updates: vec![(0, "Hi".into())],
                target_test_id: None,
                node_path: vec![],
                text_replacements: vec![],
            }
        );

        // Empty optional fields are skipped when serializing to keep wire JSON compact.
        let compact_json = legacy.to_json();
        assert!(!compact_json.contains("target_test_id"));
        assert!(!compact_json.contains("node_path"));
        assert!(!compact_json.contains("text_replacements"));
    }
}
