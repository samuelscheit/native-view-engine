//! Web DOM & SSR adapter for `nve-camera` (`WebTableSystem`).
//!
//! Lives as a thin leaf crate in `packages/modules/camera/web/` so `nve-backend-web` has
//! ZERO dependency on `nve-camera`.

use core::fmt::Write as _;

use nve_backend_web::html::write_attr;
use nve_backend_web::{WebSystemCommitCtx, WebTableSystem};
use nve_camera::table::{CameraPreviewTable, CameraTorch};
use nve_camera::CameraFacing;
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};

/// `WebTableSystem` plugin for [`CameraPreviewTable`] (`nve.camera.preview`).
#[derive(Debug, Default, Clone, Copy)]
pub struct CameraWebSystem;

fn facing_str(f: CameraFacing) -> &'static str {
    match f {
        CameraFacing::Front => "front",
        CameraFacing::External => "external",
        CameraFacing::Back | CameraFacing::Unknown => "back",
    }
}

fn torch_str(t: CameraTorch) -> &'static str {
    match t {
        CameraTorch::Off => "off",
        CameraTorch::On => "on",
        CameraTorch::Auto => "auto",
    }
}

impl WebTableSystem for CameraWebSystem {
    fn slot(&self) -> TableSlot {
        CameraPreviewTable::slot()
    }

    fn priority(&self) -> u8 {
        110
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("video")
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<CameraPreviewTable>(node) else {
            return;
        };
        let _ = write!(
            out,
            " autoplay playsinline muted data-nve-camera=\"1\" data-nve-camera-facing=\"{}\" data-nve-camera-torch=\"{}\" data-nve-camera-zoom=\"{}\" data-nve-camera-fps=\"{}\" data-nve-camera-active=\"{}\"",
            facing_str(row.facing()),
            torch_str(row.torch()),
            row.zoom(),
            row.fps(),
            if row.active() { "1" } else { "0" }
        );
        let dev = row.device_id();
        if !dev.is_empty() {
            write_attr("data-nve-camera-device", dev, out);
        }
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let Some(row) = store.try_row::<CameraPreviewTable>(node) else {
            return;
        };
        ctx.set_attr(node, "data-nve-camera", "1");
        ctx.set_attr(node, "data-nve-camera-facing", facing_str(row.facing()));
        ctx.set_attr(node, "data-nve-camera-torch", torch_str(row.torch()));
        ctx.set_attr(node, "data-nve-camera-zoom", &row.zoom().to_string());
        ctx.set_attr(node, "data-nve-camera-fps", &row.fps().to_string());
        ctx.set_attr(
            node,
            "data-nve-camera-active",
            if row.active() { "1" } else { "0" },
        );
        let dev = row.device_id();
        if !dev.is_empty() {
            ctx.set_attr(node, "data-nve-camera-device", dev);
        } else {
            ctx.remove_attr(node, "data-nve-camera-device");
        }
    }
}
