mod commands;
mod decoder;
mod encoder;
#[cfg(test)]
mod tests;
mod types;

pub const NVE_ABI_MAGIC: u32 = u32::from_le_bytes(*b"NVE1");
pub const NVE_ABI_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 24;

pub use commands::{ChildIdIter, Command, CommandRef};
pub use decoder::CommandDecoder;
pub use encoder::CommandBufferBuilder;
pub use types::{
    ColorRgba, CommandBufferHeader, ComponentKind, DecodeError, Dimension, EdgeInsets, FlexAlign,
    FlexDirection, FlexJustify, LayoutProps, Opcode, RawPropValue, RawPropValueRef, StyleProps,
};
