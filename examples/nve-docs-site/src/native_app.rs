//! Native `NveApp` implementation and `export_nve_app!` C ABI export so the exact same
//! `native-view-engine` documentation site runs natively inside Desktop Skia, macOS, Windows,
//! iOS, Android, SwiftUI, and Flutter hosts.

use nve_abi::{
    FrameId, NveGestureDisposition, NveHostEventBatchRef, NveHostEventKind, NveIntrinsicSize,
    NvePropsRef, NveSizeConstraints, SurfaceId, ValidatedEvents, SERVICE_SYSTEM_ENV,
};
use nve_dioxus::DioxusSurface;
use nve_docs::DocsViewport;
use nve_engine::{HotPatchMsg, NveApp};
use nve_system_env::{ThemeSnapshot, EVENT_THEME_CHANGED};

use crate::host::DocsHost;
use crate::site::embedded_compiled_site;

/// Native host module exporting the embedded `nve-docs-site` application over `nve_get_interface`.
pub struct DocsSiteModule {
    surface_id: SurfaceId,
    route: String,
    viewport: DocsViewport,
    /// Seeded from the OS appearance; host theme events (`NVE_IFACE_HOST`) publish into it live.
    host: DocsHost,
    surface: DioxusSurface,
    last_state_buf: Vec<u8>,
}

impl DocsSiteModule {
    /// A module on an explicit host (tests, embedders with their own appearance source).
    pub fn with_host(surface_id: SurfaceId, host: DocsHost) -> Self {
        let route = "/".to_string();
        let viewport = DocsViewport::desktop_default();
        let surface = host.mount(surface_id, embedded_compiled_site(), &route, "", viewport);
        Self { surface_id, route, viewport, host, surface, last_state_buf: Vec::new() }
    }

    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
    }

    pub fn host(&self) -> &DocsHost {
        &self.host
    }

    fn rebuild_surface(&mut self) {
        self.surface = self.host.mount(self.surface_id, embedded_compiled_site(), &self.route, "", self.viewport);
    }
}

impl NveApp for DocsSiteModule {
    fn create(surface_id: SurfaceId) -> Self {
        Self::with_host(surface_id, DocsHost::native())
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        self.surface.render_nve2_slice(frame_id)
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        self.surface.dispatch_events(events)
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

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        if let Some(route) = props.get("route").and_then(|v| v.as_str()) {
            self.route = route.to_string();
            self.rebuild_surface();
            return true;
        }
        false
    }

    /// Host appearance changes (canonical `NveHostEventBatch` `Push` records for
    /// `SERVICE_SYSTEM_ENV` / `EVENT_THEME_CHANGED`) re-theme the mounted docs through
    /// `nve-system-env`, with no rebuild.
    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        let Ok(parsed) = NveHostEventBatchRef::parse(batch) else {
            return false;
        };
        let mut applied = false;
        for rec in parsed.iter() {
            if rec.kind == NveHostEventKind::Push
                && rec.service_id.raw() == SERVICE_SYSTEM_ENV
                && rec.method_or_event_or_channel == EVENT_THEME_CHANGED
                && let Ok(theme) = ThemeSnapshot::from_repr_c_bytes(rec.payload)
            {
                self.host.publish_theme(theme);
                applied = true;
            }
        }
        applied
    }

    fn apply_hot_patch(&mut self, msg: &HotPatchMsg) -> bool {
        self.surface.apply_hot_patch(msg)
    }

    fn snapshot_state(&mut self) -> &[u8] {
        // Binary route snapshot: magic "NVDS" + UTF-8 route bytes.
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
