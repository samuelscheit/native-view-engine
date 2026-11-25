//! Native `NveApp` implementation and `export_nve_app!` C ABI export so the exact same
//! `native-view-engine` documentation site runs natively inside Desktop Skia, macOS, Windows,
//! iOS, Android, SwiftUI, and Flutter hosts.

use nve_abi::{
    BinValueView, FrameId, NveEventBatchRef, NveGestureDisposition, NveIntrinsicSize, NvePropsRef,
    NveSizeConstraints, SurfaceId, decode_events, encode_events,
};
use nve_dioxus::DioxusSurface;
use nve_docs::DocsViewport;
use nve_engine::{HotPatchMsg, NveApp};

use crate::create_nve_docs_surface_on;

/// Native host module exporting the embedded `nve-docs-site` application over `nve_get_interface`.
pub struct DocsSiteModule {
    surface_id: SurfaceId,
    route: String,
    viewport: DocsViewport,
    surface: DioxusSurface,
    last_frame_buf: Vec<u8>,
    last_state_buf: Vec<u8>,
    last_emitted_events_buf: Vec<u8>,
}

impl DocsSiteModule {
    fn rebuild_surface(&mut self) {
        self.surface = create_nve_docs_surface_on(self.surface_id, &self.route, "", self.viewport);
    }
}

impl NveApp for DocsSiteModule {
    fn create(surface_id: SurfaceId) -> Self {
        let route = "/".to_string();
        let viewport = DocsViewport::desktop_default();
        let surface = create_nve_docs_surface_on(surface_id, &route, "", viewport);
        Self {
            surface_id,
            route,
            viewport,
            surface,
            last_frame_buf: Vec::new(),
            last_state_buf: Vec::new(),
            last_emitted_events_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let (_, _, bytes) = self.surface.render_frame(frame_id);
        self.last_frame_buf = bytes;
        &self.last_frame_buf
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        let Ok(decoded) = decode_events(encoded_events) else {
            return false;
        };
        self.surface.dispatch_events(&decoded)
    }

    fn dispatch_events_direct(&mut self, events: NveEventBatchRef<'_>) -> bool {
        self.surface.dispatch_events_direct(events)
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        let w = if constraints.max_w.is_finite() && constraints.max_w > 0.0 {
            constraints.max_w.max(constraints.min_w)
        } else {
            self.viewport.width
        };
        let h = if constraints.max_h.is_finite() && constraints.max_h > 0.0 {
            constraints.max_h.max(constraints.min_h)
        } else {
            self.viewport.height
        };
        if (w - self.viewport.width).abs() > 1.0 || (h - self.viewport.height).abs() > 1.0 {
            let prev_mode = self.viewport.mode_label();
            self.viewport = DocsViewport::new(w, h);
            if self.viewport.mode_label() != prev_mode {
                self.rebuild_surface();
            }
        }
        NveIntrinsicSize {
            width: w,
            height: h,
        }
    }

    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        NveGestureDisposition::Claim
    }

    fn set_props(&mut self, props_bytes: &[u8]) -> bool {
        if let Ok(view) = BinValueView::parse(props_bytes)
            && let Some(route) = view.get("route").and_then(|v| v.as_str())
        {
            self.route = route.to_string();
            self.rebuild_surface();
            return true;
        }
        false
    }

    fn set_props_direct(&mut self, props: NvePropsRef<'_>) -> bool {
        if let Some(route) = props.get("route").and_then(|v| v.as_str()) {
            self.route = route.to_string();
            self.rebuild_surface();
            return true;
        }
        false
    }

    fn drain_emitted_events(&mut self) -> &[u8] {
        self.last_emitted_events_buf = encode_events(&[]);
        &self.last_emitted_events_buf
    }

    fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        let Ok(msg) = HotPatchMsg::from_json(patch_json) else {
            return false;
        };
        self.surface.apply_hot_patch(&msg)
    }

    fn snapshot_state(&mut self) -> &[u8] {
        // Zero-JSON binary route snapshot: magic "NVDS" + UTF-8 route bytes.
        self.last_state_buf.clear();
        self.last_state_buf.extend_from_slice(b"NVDS");
        self.last_state_buf.extend_from_slice(self.route.as_bytes());
        &self.last_state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        if let Some(rest) = state.strip_prefix(b"NVDS")
            && let Ok(route) = std::str::from_utf8(rest)
            && !route.is_empty()
        {
            self.route = route.to_string();
            self.rebuild_surface();
        }
    }
}

#[cfg(all(feature = "cdylib-export", not(target_arch = "wasm32")))]
mod cdylib_export {
    nve_engine::export_nve_app!(super::DocsSiteModule);
}
