//! Zero-copy `#[repr(C)]` `CameraFrameDescriptor`, 40-byte `CameraFrameWireLayout` (`NVCF`),
//! and in-process [`CameraFrameProcessor`] callback trait.

use std::fmt;
use std::mem::{offset_of, size_of};

pub const CAMERA_FRAME_MAGIC: [u8; 4] = *b"NVCF";
pub const CAMERA_FRAME_VERSION: u8 = 1;
pub const CAMERA_FRAME_DESCRIPTOR_SIZE: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraCodecError {
    Truncated,
    TrailingBytes(usize),
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u8),
    InvalidEnum(&'static str, u32),
    TypeMismatch { expected: &'static str },
    PayloadTooLarge,
}

impl fmt::Display for CameraCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CameraCodecError {}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CameraPixelFormat {
    #[default]
    Rgba8888 = 0,
    Bgra8888 = 1,
    Nv12 = 2,
    Yuv420_888 = 3,
}

impl CameraPixelFormat {
    #[inline]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Rgba8888),
            1 => Some(Self::Bgra8888),
            2 => Some(Self::Nv12),
            3 => Some(Self::Yuv420_888),
            _ => None,
        }
    }

    #[inline]
    pub fn from_u32(v: u32) -> Result<Self, CameraCodecError> {
        u8::try_from(v)
            .ok()
            .and_then(Self::from_u8)
            .ok_or(CameraCodecError::InvalidEnum("CameraPixelFormat", v))
    }

    #[inline]
    pub const fn min_stride_bytes(self, width: u32) -> Option<u32> {
        width.checked_mul(match self {
            Self::Rgba8888 | Self::Bgra8888 => 4,
            Self::Nv12 | Self::Yuv420_888 => 1,
        })
    }
}

/// Zero-copy in-process frame processor callback receiving [`CameraFrameDescriptor`] directly by
/// reference (zero 40-byte `NVCF` serialization/deserialization copy on the Rust hot path).
pub trait CameraFrameProcessor: Send + Sync {
    fn on_frame(&self, frame: &CameraFrameDescriptor);
}

impl<F: Fn(&CameraFrameDescriptor) + Send + Sync> CameraFrameProcessor for F {
    #[inline]
    fn on_frame(&self, frame: &CameraFrameDescriptor) {
        self(frame);
    }
}

/// 40-byte `#[repr(C)]` zero-padding wire layout for [`CameraFrameDescriptor`] (`NVCF`),
/// locked at compile time for direct C/C++/Swift/Kotlin FFI interoperability.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraFrameWireLayout {
    pub magic: [u8; 4],
    pub version: u8,
    pub pixel_format: u8,
    pub rotation_deg: u16,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub reserved: u32,
    pub timestamp_ns: u64,
    pub native_handle_id: u64,
}

const _: () = {
    assert!(size_of::<CameraFrameWireLayout>() == CAMERA_FRAME_DESCRIPTOR_SIZE);
    assert!(offset_of!(CameraFrameWireLayout, magic) == 0);
    assert!(offset_of!(CameraFrameWireLayout, version) == 4);
    assert!(offset_of!(CameraFrameWireLayout, pixel_format) == 5);
    assert!(offset_of!(CameraFrameWireLayout, rotation_deg) == 6);
    assert!(offset_of!(CameraFrameWireLayout, width) == 8);
    assert!(offset_of!(CameraFrameWireLayout, height) == 12);
    assert!(offset_of!(CameraFrameWireLayout, stride_bytes) == 16);
    assert!(offset_of!(CameraFrameWireLayout, reserved) == 20);
    assert!(offset_of!(CameraFrameWireLayout, timestamp_ns) == 24);
    assert!(offset_of!(CameraFrameWireLayout, native_handle_id) == 32);
};

impl CameraFrameWireLayout {
    #[inline]
    pub fn from_descriptor(desc: &CameraFrameDescriptor) -> Result<Self, CameraCodecError> {
        desc.validate()?;
        Ok(Self {
            magic: CAMERA_FRAME_MAGIC,
            version: CAMERA_FRAME_VERSION,
            pixel_format: desc.pixel_format as u8,
            rotation_deg: desc.rotation_deg,
            width: desc.width,
            height: desc.height,
            stride_bytes: desc.stride_bytes,
            reserved: 0,
            timestamp_ns: desc.timestamp_ns,
            native_handle_id: desc.native_handle_id,
        })
    }

    #[inline]
    pub fn to_descriptor(&self) -> Result<CameraFrameDescriptor, CameraCodecError> {
        if self.magic != CAMERA_FRAME_MAGIC {
            return Err(CameraCodecError::InvalidMagic(self.magic));
        }
        if self.version != CAMERA_FRAME_VERSION {
            return Err(CameraCodecError::UnsupportedVersion(self.version));
        }
        if self.reserved != 0 {
            return Err(CameraCodecError::InvalidEnum(
                "CameraFrameReserved",
                self.reserved,
            ));
        }
        let desc = CameraFrameDescriptor {
            width: self.width,
            height: self.height,
            stride_bytes: self.stride_bytes,
            pixel_format: CameraPixelFormat::from_u32(u32::from(self.pixel_format))?,
            rotation_deg: self.rotation_deg,
            timestamp_ns: self.timestamp_ns,
            native_handle_id: self.native_handle_id,
        };
        desc.validate()?;
        Ok(desc)
    }

    #[cfg(target_endian = "little")]
    #[inline]
    pub fn as_bytes(&self) -> &[u8; CAMERA_FRAME_DESCRIPTOR_SIZE] {
        // SAFETY: `CameraFrameWireLayout` is `#[repr(C)]` with size 40 and zero padding bytes.
        unsafe { &*(self as *const Self as *const [u8; CAMERA_FRAME_DESCRIPTOR_SIZE]) }
    }
}

/// Zero-copy `#[repr(C)]` frame descriptor matching `packages/modules/camera/cxx` & `android`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CameraFrameDescriptor {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub pixel_format: CameraPixelFormat,
    pub rotation_deg: u16,
    pub timestamp_ns: u64,
    pub native_handle_id: u64,
}

const _: () = {
    assert!(size_of::<CameraFrameDescriptor>() == 32);
    assert!(offset_of!(CameraFrameDescriptor, width) == 0);
    assert!(offset_of!(CameraFrameDescriptor, height) == 4);
    assert!(offset_of!(CameraFrameDescriptor, stride_bytes) == 8);
    assert!(offset_of!(CameraFrameDescriptor, pixel_format) == 12);
    assert!(offset_of!(CameraFrameDescriptor, rotation_deg) == 14);
    assert!(offset_of!(CameraFrameDescriptor, timestamp_ns) == 16);
    assert!(offset_of!(CameraFrameDescriptor, native_handle_id) == 24);
};

impl CameraFrameDescriptor {
    pub fn validate(&self) -> Result<(), CameraCodecError> {
        if self.width == 0 {
            return Err(CameraCodecError::TypeMismatch {
                expected: "non-zero frame width",
            });
        }
        if self.height == 0 {
            return Err(CameraCodecError::TypeMismatch {
                expected: "non-zero frame height",
            });
        }
        if !matches!(self.rotation_deg, 0 | 90 | 180 | 270) {
            return Err(CameraCodecError::InvalidEnum(
                "CameraRotationDeg",
                u32::from(self.rotation_deg),
            ));
        }
        let min_stride = self
            .pixel_format
            .min_stride_bytes(self.width)
            .ok_or(CameraCodecError::PayloadTooLarge)?;
        if self.stride_bytes < min_stride {
            return Err(CameraCodecError::TypeMismatch {
                expected: "stride_bytes >= width * bytes_per_pixel",
            });
        }
        Ok(())
    }

    #[inline]
    pub fn dispatch_to<P: CameraFrameProcessor + ?Sized>(
        &self,
        processor: &P,
    ) -> Result<(), CameraCodecError> {
        self.validate()?;
        processor.on_frame(self);
        Ok(())
    }

    #[inline]
    pub fn to_wire_layout(&self) -> Result<CameraFrameWireLayout, CameraCodecError> {
        CameraFrameWireLayout::from_descriptor(self)
    }

    pub fn encode(&self) -> Result<[u8; CAMERA_FRAME_DESCRIPTOR_SIZE], CameraCodecError> {
        let mut out = [0u8; CAMERA_FRAME_DESCRIPTOR_SIZE];
        self.encode_into(&mut out)?;
        Ok(out)
    }

    pub fn encode_into(
        &self,
        dst: &mut [u8; CAMERA_FRAME_DESCRIPTOR_SIZE],
    ) -> Result<(), CameraCodecError> {
        self.validate()?;
        dst[0..4].copy_from_slice(&CAMERA_FRAME_MAGIC);
        dst[4] = CAMERA_FRAME_VERSION;
        dst[5] = self.pixel_format as u8;
        dst[6..8].copy_from_slice(&self.rotation_deg.to_le_bytes());
        dst[8..12].copy_from_slice(&self.width.to_le_bytes());
        dst[12..16].copy_from_slice(&self.height.to_le_bytes());
        dst[16..20].copy_from_slice(&self.stride_bytes.to_le_bytes());
        dst[20..24].copy_from_slice(&0u32.to_le_bytes());
        dst[24..32].copy_from_slice(&self.timestamp_ns.to_le_bytes());
        dst[32..40].copy_from_slice(&self.native_handle_id.to_le_bytes());
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, CameraCodecError> {
        Ok(self.encode()?.to_vec())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CameraCodecError> {
        if bytes.len() < CAMERA_FRAME_DESCRIPTOR_SIZE {
            return Err(CameraCodecError::Truncated);
        }
        if bytes.len() > CAMERA_FRAME_DESCRIPTOR_SIZE {
            return Err(CameraCodecError::TrailingBytes(
                bytes.len() - CAMERA_FRAME_DESCRIPTOR_SIZE,
            ));
        }
        let le32 = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let le64 = |o: usize| {
            u64::from_le_bytes([
                bytes[o],
                bytes[o + 1],
                bytes[o + 2],
                bytes[o + 3],
                bytes[o + 4],
                bytes[o + 5],
                bytes[o + 6],
                bytes[o + 7],
            ])
        };
        let magic: [u8; 4] = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != CAMERA_FRAME_MAGIC {
            return Err(CameraCodecError::InvalidMagic(magic));
        }
        if bytes[4] != CAMERA_FRAME_VERSION {
            return Err(CameraCodecError::UnsupportedVersion(bytes[4]));
        }
        let pixel_format = CameraPixelFormat::from_u32(u32::from(bytes[5]))?;
        let rotation_deg = u16::from_le_bytes([bytes[6], bytes[7]]);
        let width = le32(8);
        let height = le32(12);
        let stride_bytes = le32(16);
        let reserved = le32(20);
        if reserved != 0 {
            return Err(CameraCodecError::InvalidEnum(
                "CameraFrameReserved",
                reserved,
            ));
        }
        let desc = Self {
            width,
            height,
            stride_bytes,
            pixel_format,
            rotation_deg,
            timestamp_ns: le64(24),
            native_handle_id: le64(32),
        };
        desc.validate()?;
        Ok(desc)
    }
}
