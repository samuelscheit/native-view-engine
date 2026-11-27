//! `#[derive(NveTable)]` schema for [`CameraPreviewTable`] (`nve.camera.preview`) — zero
//! dependency on `nve-components` or backends.

use nve_store::{NveTable, Str, WordEnum};

use crate::types::CameraFacing;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum CameraTorch {
    #[default]
    Off = 0,
    On = 1,
    Auto = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, WordEnum)]
pub enum CameraStabilization {
    #[default]
    Off = 0,
    Standard = 1,
    Cinematic = 2,
}

/// Camera preview surface table (`nve.camera.preview`).
#[derive(NveTable)]
#[nve_table(name = "nve.camera.preview", storage = "sparse", flags = "publish")]
pub struct CameraPreviewTable {
    #[effect(realize)]
    pub device_id: Str,
    #[effect(realize, paint)]
    #[default(CameraFacing::Back)]
    pub facing: CameraFacing,
    #[effect(realize, paint)]
    pub torch: CameraTorch,
    #[effect(realize, paint)]
    pub stabilization: CameraStabilization,
    #[effect(realize, paint)]
    pub hdr: bool,
    #[effect(realize, paint)]
    pub enable_frame_processor: bool,
    #[effect(realize, paint)]
    pub audio: bool,
    #[effect(realize, paint)]
    #[default(true)]
    pub active: bool,
    #[effect(realize, paint)]
    #[default(30u16)]
    pub fps: u16,
    #[effect(paint)]
    #[default(1.0)]
    pub zoom: f32,
}

/// Typed `TableEvent` IDs emitted back from camera preview hosts.
pub mod events {
    pub const READY: u32 = 1;
    pub const FRAME: u32 = 2;
    pub const PHOTO_CAPTURED: u32 = 3;
    pub const ERROR: u32 = 4;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    #[test]
    fn camera_preview_table_defaults_and_writes() {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<CameraPreviewTable>(n)
            .device_id("cam-0")
            .facing(CameraFacing::Front)
            .torch(CameraTorch::On)
            .zoom(2.5)
            .fps(60);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let row = store.try_row::<CameraPreviewTable>(n).unwrap();
        assert_eq!(row.device_id(), "cam-0");
        assert_eq!(row.facing(), CameraFacing::Front);
        assert_eq!(row.torch(), CameraTorch::On);
        assert_eq!(row.zoom(), 2.5);
        assert_eq!(row.fps(), 60);
        assert!(row.active());
    }
}
