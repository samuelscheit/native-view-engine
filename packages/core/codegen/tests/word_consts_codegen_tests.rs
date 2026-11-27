//! Word enums / flag sets ride with the table headers (`nve codegen abi`): C `#define`s and Swift
//! enums whose case names are keyword-escaped; descriptor sets that name one mirror twice, or an
//! empty one, are refused.

use nve_abi::table::WordConstsDesc;
use nve_codegen::CodegenError;
use nve_codegen::abi::generate_table_files;
use nve_components::tables::{CORE_TABLE_DESCS, CORE_WORD_CONSTS};

#[test]
fn word_consts_emit_c_defines_and_swift_enums() {
    let files = generate_table_files(CORE_TABLE_DESCS, CORE_WORD_CONSTS).expect("core consts");
    let text = |suffix: &str| &files.iter().find(|f| f.relative_path.ends_with(suffix)).expect(suffix).content;
    let c = text("nve_tables.generated.h");
    assert!(c.contains("#define NVE_IMAGE_FIT_SCALE_DOWN 4u"), "word enum");
    assert!(c.contains("#define NVE_TYPOGRAPHY_SET_COLOR 1u"), "flag set value");
    assert!(c.contains("NVE_TABLE_VISUAL_WORD_EFFECTS["), "per-word effects for `DeclareTables`");
    let swift = text("NveTables.generated.swift");
    assert!(swift.contains("public enum NveImageFit: UInt32, Sendable, CaseIterable {"));
    assert!(swift.contains("case none_ = 3") && swift.contains("case scaleDown = 4"), "escaped + camel");
    assert!(swift.contains("public enum NveTypographySet {"), "flag sets are namespaces, not enums");
}

#[test]
fn duplicate_or_empty_word_consts_are_refused() {
    const A: WordConstsDesc = WordConstsDesc { name: "Dup", flags: false, entries: &[("A", 0)] };
    const EMPTY: WordConstsDesc = WordConstsDesc { name: "Empty", flags: false, entries: &[] };
    for bad in [&[&A, &A][..], &[&EMPTY][..]] {
        assert!(matches!(generate_table_files(&[], bad), Err(CodegenError::Validation(_))));
    }
}
