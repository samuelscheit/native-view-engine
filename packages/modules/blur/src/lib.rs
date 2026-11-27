//! Standalone backdrop blur surface capability (`packages/modules/blur`).

use nve_store::{word_enum, NveTable, Rgba};

word_enum! {
    /// Platform-native backdrop blur material style (AppKit `NSVisualEffectView`,
    /// UIKit `UIBlurEffect`, WinUI Acrylic/Mica, Flutter `BackdropFilter`, CSS `backdrop-filter`).
    ///
    /// Wire discriminants `0..=5` match `nve::blur::apple::BlurMaterial`,
    /// `nve::blur::win::BlurMaterial`, and Dart `NveBlurMaterial`.
    pub enum BlurMaterial: 3 {
        UltraThin = 0,
        Thin = 1,
        Regular = 2,
        Thick = 3,
        Chrome = 4,
        SystemMaterial = 5,
    }
}

impl BlurMaterial {
    /// Approximate CSS blur radius in logical px for web fallback.
    #[inline]
    pub const fn css_blur_px(self) -> f32 {
        match self {
            Self::UltraThin => 6.0,
            Self::Thin => 10.0,
            Self::Regular => 16.0,
            Self::Thick => 24.0,
            Self::Chrome => 20.0,
            Self::SystemMaterial => 18.0,
        }
    }
}

/// Backdrop blur surface table (`nve.blur_surface`).
#[derive(NveTable)]
#[nve_table(name = "nve.blur_surface", storage = "sparse", flags = "publish")]
pub struct BlurSurface {
    #[effect(realize, paint)]
    #[bits(3)]
    #[default(BlurMaterial::Regular)]
    pub material: BlurMaterial,
    #[effect(paint)]
    #[bits(1)]
    pub tint_enabled: bool,
    #[effect(paint)]
    #[default(1.0)]
    pub intensity: f32,
    #[effect(paint)]
    pub tint: Rgba,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    #[test]
    fn blur_material_wire_discriminants_match_cross_platform_adapters() {
        assert_eq!(BlurMaterial::UltraThin as u32, 0);
        assert_eq!(BlurMaterial::Thin as u32, 1);
        assert_eq!(BlurMaterial::Regular as u32, 2);
        assert_eq!(BlurMaterial::Thick as u32, 3);
        assert_eq!(BlurMaterial::Chrome as u32, 4);
        assert_eq!(BlurMaterial::SystemMaterial as u32, 5);
    }

    #[test]
    fn blur_surface_defaults_and_writes() {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let def_n = tx.create();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[def_n, n]);
        let _ = tx.edit::<BlurSurface>(def_n);
        tx.edit::<BlurSurface>(n)
            .material(BlurMaterial::UltraThin)
            .intensity(0.8)
            .tint_enabled(true)
            .tint(Rgba::new(255, 255, 255, 48));
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let def_row = store.try_row::<BlurSurface>(def_n).unwrap();
        assert_eq!(def_row.material(), BlurMaterial::Regular);
        assert_eq!(def_row.intensity(), 1.0);

        let row = store.try_row::<BlurSurface>(n).unwrap();
        assert_eq!(row.material(), BlurMaterial::UltraThin);
        assert_eq!(row.intensity(), 0.8);
        assert!(row.tint_enabled());
        assert_eq!(row.tint(), Rgba::new(255, 255, 255, 48));
    }
}
