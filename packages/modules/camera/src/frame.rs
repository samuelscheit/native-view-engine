//! Zero-copy `#[repr(C)]` `CameraFrameDescriptor`, 40-byte `CameraFrameWireLayout` (`NVCF`),
//! and in-process [`CameraFrameProcessor`] callback trait.

use std::fmt;
use std::mem::{offset_of, size_of};

use nve_codec::{write_fixed, ByteReader, ByteSink, CodecError, NveCodec, SliceWriter};

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

impl From<CodecError> for CameraCodecError {
    #[inline]
    fn from(err: CodecError) -> Self {
        match err {
            CodecError::Truncated { .. } => Self::Truncated,
            CodecError::TrailingBytes(n) => Self::TrailingBytes(n),
            CodecError::BadEnum { ty, raw } => Self::InvalidEnum(ty, raw as u32),
            CodecError::NonZeroReserved(v) => Self::InvalidEnum("CameraFrameReserved", v as u32),
            CodecError::LenOverflow { .. } | CodecError::DepthExceeded { .. } => {
                Self::PayloadTooLarge
            }
            CodecError::BadMagic
            | CodecError::BadVersion(_)
            | CodecError::BadBool(_)
            | CodecError::InvalidUtf8 => Self::Truncated,
        }
    }
}

impl fmt::Display for CameraCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CameraCodecError {}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, NveCodec)]
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
            pixel_format: CameraPixelFormat::from_repr(self.pixel_format).ok_or(
                CameraCodecError::InvalidEnum("CameraPixelFormat", u32::from(self.pixel_format)),
            )?,
            rotation_deg: self.rotation_deg,
            timestamp_ns: self.timestamp_ns,
            native_handle_id: self.native_handle_id,
        };
        desc.validate()?;
        Ok(desc)
    }

    #[inline]
    fn write_to_sink<S: ByteSink>(&self, w: &mut S) -> Result<(), CodecError> {
        w.put_raw(&self.magic)?;
        w.put_u8(self.version)?;
        w.put_u8(self.pixel_format)?;
        w.put_u16(self.rotation_deg)?;
        w.put_u32(self.width)?;
        w.put_u32(self.height)?;
        w.put_u32(self.stride_bytes)?;
        w.put_u32(self.reserved)?;
        w.put_u64(self.timestamp_ns)?;
        w.put_u64(self.native_handle_id)
    }

    /// Encodes this 40-byte wire layout directly into `dst` with zero heap allocations.
    #[inline]
    pub fn write_into(
        &self,
        dst: &mut [u8; CAMERA_FRAME_DESCRIPTOR_SIZE],
    ) -> Result<(), CodecError> {
        let mut w = SliceWriter::new(dst);
        self.write_to_sink(&mut w)?;
        w.finish()
    }

    /// Encodes this 40-byte wire layout into canonical little-endian bytes using `write_fixed`
    /// (zero heap allocations and zero `unsafe` pointer casts).
    #[inline]
    pub fn to_bytes(&self) -> [u8; CAMERA_FRAME_DESCRIPTOR_SIZE] {
        write_fixed::<CAMERA_FRAME_DESCRIPTOR_SIZE>(|w| self.write_to_sink(w))
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
        Ok(self.to_wire_layout()?.to_bytes())
    }

    pub fn encode_into(
        &self,
        dst: &mut [u8; CAMERA_FRAME_DESCRIPTOR_SIZE],
    ) -> Result<(), CameraCodecError> {
        self.to_wire_layout()?.write_into(dst)?;
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
        let mut r = ByteReader::new(bytes);
        let magic = r.read_array::<4>()?;
        if magic != CAMERA_FRAME_MAGIC {
            return Err(CameraCodecError::InvalidMagic(magic));
        }
        let version = r.read_u8()?;
        if version != CAMERA_FRAME_VERSION {
            return Err(CameraCodecError::UnsupportedVersion(version));
        }
        let raw_pixel_format = r.read_u8()?;
        let pixel_format = CameraPixelFormat::from_repr(raw_pixel_format).ok_or(
            CameraCodecError::InvalidEnum("CameraPixelFormat", u32::from(raw_pixel_format)),
        )?;
        let rotation_deg = r.read_u16()?;
        let width = r.read_u32()?;
        let height = r.read_u32()?;
        let stride_bytes = r.read_u32()?;
        let reserved = r.read_u32()?;
        if reserved != 0 {
            return Err(CameraCodecError::InvalidEnum(
                "CameraFrameReserved",
                reserved,
            ));
        }
        let timestamp_ns = r.read_u64()?;
        let native_handle_id = r.read_u64()?;
        r.finish()?;
        let desc = Self {
            width,
            height,
            stride_bytes,
            pixel_format,
            rotation_deg,
            timestamp_ns,
            native_handle_id,
        };
        desc.validate()?;
        Ok(desc)
    }
}
