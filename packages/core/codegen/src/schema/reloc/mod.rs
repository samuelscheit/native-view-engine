//! Intermediate Representation (IR), `#[repr(C)]` layout computation, and validation for
//! NVE's zero-copy aligned `RelocSlice` & `VerifyMode` schema model.

pub mod ir;
mod validate;

pub use ir::{
    FieldLayout, RelocEnum, RelocEnumRepr, RelocEnumVariant, RelocField, RelocSchema, RelocStruct,
    RelocType, StructLayout, RELOC_HEADER_ALIGN, RELOC_HEADER_SIZE, RELOC_MAX_VERIFY_DEPTH,
    RELOC_SLICE_ALIGN, RELOC_SLICE_SIZE,
};
