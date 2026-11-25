//! Zero-dependency proc-macro crate implementing `#[derive(NveAbi)]` for single-source-of-truth
//! C ABI metadata and compile-time layout assertions (`size_of`, `align_of`, `offset_of!`).

extern crate proc_macro;

mod emit;
mod parse;

use proc_macro::TokenStream;

pub(crate) enum ParsedItem {
    Struct(ParsedRecord),
    Union(ParsedRecord),
    Enum(ParsedEnum),
}

pub(crate) struct ParsedRecord {
    pub rust_ident: String,
    pub abi_name: String,
    pub doc: Option<String>,
    pub crate_path: String,
    pub explicit_align: Option<usize>,
    pub expected_size: Option<usize>,
    pub expected_size_64: Option<usize>,
    pub fields: Vec<ParsedField>,
}

pub(crate) struct ParsedField {
    pub rust_ident: String,
    pub abi_name: String,
    pub ty: String,
    pub c_type: Option<String>,
    pub doc: Option<String>,
    pub expected_offset: Option<usize>,
    pub expected_offset_64: Option<usize>,
}

pub(crate) struct ParsedEnum {
    pub rust_ident: String,
    pub abi_name: String,
    pub repr: String,
    pub doc: Option<String>,
    pub crate_path: String,
    pub c_prefix: Option<String>,
    pub dart_fallback: Option<String>,
    pub variants: Vec<ParsedVariant>,
}

pub(crate) struct ParsedVariant {
    pub rust_ident: String,
    pub abi_name: String,
    pub doc: Option<String>,
}

/// Derives `NveStructSchema` / `NveUnionSchema` / `NveEnumSchema` (plus `NveSchema`) and
/// compile-time layout assertions on `#[repr(C)]` structs/unions and `#[repr(u8|u16|u32|i32)]` enums.
#[proc_macro_derive(NveAbi, attributes(nve_abi))]
pub fn derive_nve_abi(input: TokenStream) -> TokenStream {
    match parse::parse_item(input) {
        Ok(item) => emit::emit_item(&item)
            .parse::<TokenStream>()
            .unwrap_or_else(|e| compile_error(&format!("nve-abi-macros emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

fn compile_error(msg: &str) -> TokenStream {
    format!("::core::compile_error!({});", emit::quote_str(msg))
        .parse()
        .expect("compile_error! must parse as TokenStream")
}
