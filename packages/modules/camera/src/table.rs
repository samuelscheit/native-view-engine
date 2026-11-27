//! `#[derive(NveTable)]` schema for [`CameraPreviewTable`] (`nve.camera.preview`) — zero
//! dependency on `nve-components` or backends.

use nve_store::{word_enum, FieldKind, NveTable, Str, WordValue};

use crate::types::CameraFacing;

impl WordValue for CameraFacing {
    const BITS: u8 = 2;
    const KIND: FieldKind = FieldKind::Enum;
    #[inline(always)]
    fn to_bits(self) -> u32 {
        self as u32
    }
    #[inline(always)]
    fn from_bits(bits: u32) -> Self {
        Self::from_u8(bits as u8).unwrap_or(Self::Back)
    }
}

word_enum! {
    pub enum CameraTorch: 2 { Off = 0, On = 1, Auto = 2 }
}

word_enum! {
    pub enum CameraStabilization: 2 { Off = 0, Standard = 1, Cinematic = 2 }
}

/// Camera preview surface table (`nve.camera.preview`).
#[derive(NveTable)]
#[nve_table(name = "nve.camera.preview", storage = "sparse", flags = "publish")]
pub struct CameraPreviewTable {
    #[effect(realize)]
    pub device_id: Str,
    #[effect(realize, paint)]
    #[bits(2)]
    #[default(CameraFacing::Back)]
    pub facing: CameraFacing,
    #[effect(realize, paint)]
    #[bits(2)]
    pub torch: CameraTorch,
    #[effect(realize, paint)]
    #[bits(2)]
    pub stabilization: CameraStabilization,
    #[effect(realize, paint)]
    #[bits(1)]
    pub hdr: bool,
    #[effect(realize, paint)]
    #[bits(1)]
    pub enable_frame_processor: bool,
    #[effect(realize, paint)]
    #[bits(1)]
    pub audio_enabled: bool,
    #[effect(realize, paint)]
    #[bits(1)]
    #[default(true)]
    pub active: bool,
    #[effect(realize, paint)]
    #[bits(16)]
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
