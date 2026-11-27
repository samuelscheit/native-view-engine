//! Pure-Dart emitter for `nve_tables.generated.dart`: one `FieldSpec` set, word count, key, and
//! default row per core published `#[derive(NveTable)]` table, plus every `word_enum!` value those
//! tables use — single-sourced from the compiled Rust `TableDesc`s so Dart readers never hand-copy
//! word / shift / bit layouts.

use nve_abi::table::{FieldKind, Storage, TableDesc};
use nve_components::tables::{
    text_decoration, AlignItems, AriaAutoComplete, AriaLive, AriaOrientation, AriaRole,
    AriaTriState, Display, FlexDirection, FontStyle, ImageFit, InputKind, JustifyContent,
    LinkTarget, Overflow, OverflowWrap, Position, ScrollAxes, Scrollbars, TextAlign,
    VerticalAlign, Visibility, WhiteSpace, WordBreak,
};
use nve_components::tables::{
    Accessibility, DebugId, GridItem, GridLayout, Image, Interaction, Layout, Link, Motion,
    Responsive, Scroll, StreamBoundary, TextContent, TextInput, Transform, Typography, Visual,
};
use nve_store::NveTable;

use super::core_abi::dart_enum_variant_ident;
use crate::naming::{to_camel_case, to_pascal_case};
use crate::writer::CodeWriter;

/// Every core table registered by `nve_components::register_core_tables` (module tables stay in
/// their own crates: hosts never depend on optional modules).
fn core_tables() -> [&'static TableDesc; 17] {
    [
        &Layout::DESC,
        &GridLayout::DESC,
        &GridItem::DESC,
        &Visual::DESC,
        &Typography::DESC,
        &TextContent::DESC,
        &Transform::DESC,
        &Motion::DESC,
        &Responsive::DESC,
        &Interaction::DESC,
        &Accessibility::DESC,
        &DebugId::DESC,
        &StreamBoundary::DESC,
        &Link::DESC,
        &Scroll::DESC,
        &Image::DESC,
        &TextInput::DESC,
    ]
}

/// `(DartName, [(variant, value)])` for one `word_enum!`.
type WordEnum = (&'static str, Vec<(String, u32)>);

macro_rules! word_enums {
    ($($ty:ident),+ $(,)?) => {
        vec![$((
            stringify!($ty),
            $ty::ALL.iter().map(|v| (format!("{v:?}"), v.to_word())).collect::<Vec<_>>(),
        )),+]
    };
}

fn core_word_enums() -> Vec<WordEnum> {
    word_enums!(
        Overflow,
        Visibility,
        FontStyle,
        TextAlign,
        WhiteSpace,
        WordBreak,
        OverflowWrap,
        VerticalAlign,
        Display,
        FlexDirection,
        JustifyContent,
        AlignItems,
        Position,
        ScrollAxes,
        Scrollbars,
        ImageFit,
        InputKind,
        LinkTarget,
        AriaRole,
        AriaTriState,
        AriaLive,
        AriaOrientation,
        AriaAutoComplete,
    )
}

/// Bit-mask constants that belong to one table (`Typography::SET_*`, `text_decoration::*`).
fn table_masks(name: &str) -> &'static [(&'static str, u32)] {
    match name {
        "nve.typography" => &[
            ("setColor", Typography::SET_COLOR),
            ("setFontSize", Typography::SET_FONT_SIZE),
            ("setLineHeight", Typography::SET_LINE_HEIGHT),
            ("setLetterSpacing", Typography::SET_LETTER_SPACING),
            ("setFontFamily", Typography::SET_FONT_FAMILY),
            ("setMaxLines", Typography::SET_MAX_LINES),
            ("setFontWeight", Typography::SET_FONT_WEIGHT),
            ("setFontStyle", Typography::SET_FONT_STYLE),
            ("setWhiteSpace", Typography::SET_WHITE_SPACE),
            ("setWordBreak", Typography::SET_WORD_BREAK),
            ("setOverflowWrap", Typography::SET_OVERFLOW_WRAP),
            ("setTextAlign", Typography::SET_TEXT_ALIGN),
            ("setTextDecoration", Typography::SET_TEXT_DECORATION),
            ("setVerticalAlign", Typography::SET_VERTICAL_ALIGN),
            ("decorationUnderline", text_decoration::UNDERLINE as u32),
            ("decorationOverline", text_decoration::OVERLINE as u32),
            ("decorationLineThrough", text_decoration::LINE_THROUGH as u32),
        ],
        _ => &[],
    }
}

fn dart_field_kind(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::U32 => "u32",
        FieldKind::I32 => "i32",
        FieldKind::F32 => "f32",
        FieldKind::Bool => "boolean",
        FieldKind::Enum => "enumeration",
        FieldKind::Color => "color",
        FieldKind::Length => "length",
        FieldKind::Str => "str",
        FieldKind::Resource => "resource",
        FieldKind::Node => "node",
    }
}

/// `nve.text_input` → `NveTextInputTable`.
fn table_class_name(name: &str) -> String {
    let bare = name.strip_prefix("nve.").unwrap_or(name).replace('.', "_");
    format!("Nve{}Table", to_pascal_case(&bare))
}

fn emit_table(out: &mut CodeWriter, desc: &TableDesc) {
    let class = table_class_name(desc.name);
    let sparse = matches!(desc.storage, Storage::Sparse);
    let key = desc.key.0;
    writeln!(
        out,
        "/// `{}` ({}, {} words).",
        desc.name,
        if sparse { "sparse" } else { "dense" },
        desc.words()
    );
    writeln!(out, "abstract final class {class} {{");
    writeln!(out, "  static const String name = '{}';", desc.name);
    writeln!(
        out,
        "  static const Key64 key = Key64(0x{:08X}, 0x{:08X});",
        key as u32,
        (key >> 32) as u32
    );
    writeln!(out, "  static const int words = {};", desc.words());
    writeln!(out, "  static const bool sparse = {sparse};");
    let defaults: Vec<String> = desc.defaults.iter().map(|w| format!("0x{w:08X}")).collect();
    writeln!(out, "  static const List<int> defaults = <int>[{}];", defaults.join(", "));
    for (mask, value) in table_masks(desc.name) {
        writeln!(out, "  static const int {mask} = 0x{value:X};");
    }
    for f in desc.fields {
        let mut args = format!("{}, FieldKind.{}", f.word, dart_field_kind(f.kind));
        if f.shift != 0 {
            args.push_str(&format!(", shift: {}", f.shift));
        }
        // Rust reports `Str` as 64 bits (two whole words); Dart `FieldSpec` spans them implicitly.
        if f.bits != 32 && f.kind != FieldKind::Str {
            args.push_str(&format!(", bits: {}", f.bits));
        }
        writeln!(out, "  static final FieldSpec {} = FieldSpec({args});", to_camel_case(f.name));
    }
    writeln!(out, "}}");
    writeln!(out);
}

/// Generates `packages/core/host-dart/lib/src/nve_tables.generated.dart`.
pub fn generate_dart_tables() -> String {
    let mut out = CodeWriter::with_capacity(16 * 1024);
    writeln!(out, "// @generated by nve-codegen (Dart table layout generator) — DO NOT EDIT MANUALLY.");
    writeln!(
        out,
        "// Core `#[derive(NveTable)]` word layouts from packages/core/components/src/tables."
    );
    writeln!(out);
    writeln!(out, "import 'key64.dart';");
    writeln!(out, "import 'table_reader.dart';");
    writeln!(out);
    for desc in core_tables() {
        emit_table(&mut out, desc);
    }
    for (name, variants) in core_word_enums() {
        writeln!(out, "/// `{name}` word values.");
        writeln!(out, "abstract final class Nve{name} {{");
        for (variant, value) in variants {
            writeln!(
                out,
                "  static const int {} = {value};",
                dart_enum_variant_ident(&variant)
            );
        }
        writeln!(out, "}}");
        writeln!(out);
    }
    out.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_every_core_table_with_exact_layout() {
        let src = generate_dart_tables();
        for desc in core_tables() {
            assert!(src.contains(&format!("static const String name = '{}';", desc.name)));
        }
        assert!(src.contains("abstract final class NveVisualTable {"));
        assert!(src.contains("abstract final class NveTextInputTable {"));
        assert!(src.contains("static final FieldSpec text = FieldSpec(0, FieldKind.str);"));
        assert!(src.contains("abstract final class NveOverflow {"));
        assert!(src.contains("static const int setColor = 0x1;"));
        assert!(!src.contains("static const int default ="));
    }
}
