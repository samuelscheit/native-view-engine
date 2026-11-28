//! Zero-dependency proc-macro crate implementing `#[derive(NveAbi)]`, `#[derive(ReprEnum)]`,
//! `abi_consts!`, `abi_aliases!`, `abi_callbacks!`, `#[derive(NveTable)]`, `#[derive(NveCodec)]`,
//! `#[derive(WordEnum)]`, and `#[derive(EventEnum)]`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate proc_macro;

mod abi_items;
mod codec;
mod emit;
mod event_enum;
mod fn_sig;
mod lit;
mod parse;
mod parse_attrs;
mod repr_enum;
mod table;
mod tok;
mod word_enum;

use proc_macro::{Group, LexError, Span, TokenStream, TokenTree};

use crate::fn_sig::ParsedFnSig;

pub(crate) enum ParsedItem {
    Struct(ParsedRecord),
    Union(ParsedRecord),
    Enum(ParsedEnum),
    Vtable(ParsedVtable),
    Alias(ParsedAlias),
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

pub(crate) struct ParsedVtable {
    pub rust_ident: String,
    pub abi_name: String,
    pub module_name: Option<String>,
    pub interface_id: Option<String>,
    pub slots_offset: Option<usize>,
    pub doc: Option<String>,
    pub crate_path: String,
    pub header_fields: Vec<ParsedField>,
    pub methods: Vec<ParsedVtableMethod>,
}

pub(crate) struct ParsedVtableMethod {
    pub rust_ident: String,
    pub abi_name: String,
    pub doc: Option<String>,
    pub sig: ParsedFnSig,
}

pub(crate) struct ParsedAlias {
    pub rust_ident: String,
    pub abi_name: String,
    pub rust_target: String,
    pub abi_target: String,
    pub doc: Option<String>,
    pub crate_path: String,
}

/// Derives `NveStructSchema` / `NveUnionSchema` / `NveEnumSchema` / `NveVtableSchema` /
/// `NveAliasSchema` (plus `NveSchema`) and compile-time layout assertions on `#[repr(C)]`
/// structs/unions/vtables, `#[repr(transparent)]` handle newtypes, and `#[repr(u8|u16|u32|i32)]`
/// enums.
#[proc_macro_derive(NveAbi, attributes(nve_abi))]
pub fn derive_nve_abi(input: TokenStream) -> TokenStream {
    match parse::parse_item(input) {
        Ok(item) => parse_at_call_site(&emit::emit_item(&item))
            .unwrap_or_else(|e| compile_error(&format!("nve-abi-macros emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

/// Declares ABI constants verbatim alongside a `const` `&[AbiConstDescriptor]` slice.
#[proc_macro]
pub fn abi_consts(input: TokenStream) -> TokenStream {
    match abi_items::expand_consts(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("abi_consts! emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

/// Declares ABI `pub type` aliases verbatim alongside a `const` `&[AbiAliasDescriptor]` slice.
#[proc_macro]
pub fn abi_aliases(input: TokenStream) -> TokenStream {
    match abi_items::expand_aliases(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("abi_aliases! emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

/// Declares ABI `extern "C" fn` callback typedefs verbatim alongside a `const`
/// `&[AbiCallbackDescriptor]` slice.
#[proc_macro]
pub fn abi_callbacks(input: TokenStream) -> TokenStream {
    match abi_items::expand_callbacks(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("abi_callbacks! emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

/// Zero-dependency derive for unit-only `#[repr(u8|u16|u32|u64|i8|i16|i32|i64)]` enums emitting
/// `COUNT`, `ALL`, `as_<repr>`, `try_from_<repr>`, and (only with `#[repr_enum(fallback = V)]`)
/// `from_<repr>`.
#[proc_macro_derive(ReprEnum, attributes(repr_enum))]
pub fn derive_repr_enum(input: TokenStream) -> TokenStream {
    match repr_enum::derive(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("ReprEnum emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

fn compile_error(msg: &str) -> TokenStream {
    parse_at_call_site(&format!("::core::compile_error!({});", emit::quote_str(msg)))
        .expect("compile_error! must parse as TokenStream")
}

/// Parses emitted source and pins every token to `Span::call_site()`.
///
/// rustc already gives string-parsed tokens the call-site span, but rust-analyzer's proc-macro
/// server keeps the call-site anchor while replacing each range with the token's byte offset in
/// `src`. Those ranges point past the derive (often past EOF) and panic IDE features such as the
/// impls code lens (`line-index: invalid offset`). See docs/UPSTREAM.md.
fn parse_at_call_site(src: &str) -> Result<TokenStream, LexError> {
    Ok(respan(src.parse()?, Span::call_site()))
}

fn respan(stream: TokenStream, span: Span) -> TokenStream {
    stream
        .into_iter()
        .map(|mut tt| {
            if let TokenTree::Group(g) = &tt {
                tt = TokenTree::Group(Group::new(g.delimiter(), respan(g.stream(), span)));
            }
            tt.set_span(span);
            tt
        })
        .collect()
}

/// Derives `nve_store::NveTable` for a word-table schema struct (see `table.rs` docs).
#[proc_macro_derive(NveTable, attributes(nve_table, effect, default, bits, no_inherit))]
pub fn derive_nve_table(input: TokenStream) -> TokenStream {
    match table::derive(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("NveTable emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

/// Derives `nve_codec::{NveEncode, NveDecode}` for Tier-2 little-endian binary structs and enums.
#[proc_macro_derive(NveCodec, attributes(nve_codec))]
pub fn derive_nve_codec(input: TokenStream) -> TokenStream {
    match codec::derive(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("NveCodec emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

/// Derives `nve_store::{WordValue, OptWord}` + `COUNT`/`BITS`/`ALL`/`to_word`/`from_word`
/// for fieldless `#[repr(u8)]` contiguous `0..N` enums.
#[proc_macro_derive(WordEnum, attributes(word_enum))]
pub fn derive_word_enum(input: TokenStream) -> TokenStream {
    match word_enum::derive(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("WordEnum emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}

/// Derives `nve_abi::EventEnum` + `COUNT`/`ALL`/`event_id`/`bit`/`set`/`from_event_id`/`BitOr`
/// for table-scoped `#[repr(u8)]` contiguous `0..N` (`1 <= N <= 32`) event enums.
#[proc_macro_derive(EventEnum, attributes(event_enum))]
pub fn derive_event_enum(input: TokenStream) -> TokenStream {
    match event_enum::derive(input) {
        Ok(code) => parse_at_call_site(&code)
            .unwrap_or_else(|e| compile_error(&format!("EventEnum emit error: {e}"))),
        Err(msg) => compile_error(&msg),
    }
}
