//! Unified camera types shared between `NVNP`/`NVNC` native-view specs and the
//! `SERVICE_CAMERA = 0x0023` host service.

/// Unified physical lens facing direction for camera enumeration and live previews.
///
/// Discriminants match `packages/modules/camera/{cxx,apple,android,web}` (`1..=3` for preview
/// props/commands; `0 = Unknown` for enumerated devices with unspecified position).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, nve_store::WordEnum)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CameraFacing {
    Unknown = 0,
    #[default]
    Back = 1,
    Front = 2,
    External = 3,
}

impl CameraFacing {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Unknown),
            1 => Some(Self::Back),
            2 => Some(Self::Front),
            3 => Some(Self::External),
            _ => None,
        }
    }

    /// Parses a preview facing discriminant (`1..=3`), rejecting `0 = Unknown`.
    #[inline]
    pub const fn from_preview_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Back),
            2 => Some(Self::Front),
            3 => Some(Self::External),
            _ => None,
        }
    }
}
