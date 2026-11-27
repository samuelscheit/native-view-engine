//! `nve_tables.generated.h` named field values (`word_enum!` variants, `#[values]` flag bits),
//! `NveTableDesc` aggregates, and capability-module table headers: complete, validated, and
//! compiling as C11 / C++20 with every `_DESC` equal to its Rust `TableDesc`.

mod common;

use std::path::Path;
use std::process::Command;

use nve_abi::table::{FieldDesc, FieldKind, Storage, TableDesc, ValueSetDesc};
use nve_codegen::abi::spell::screaming;
use nve_codegen::{CodegenError, generate_module_table_header, generate_table_files};
use nve_components::tables::{AriaRole, CORE_TABLE_DESCS, CursorKind, Typography, text_decoration};

fn core_header() -> String {
    let files = generate_table_files(CORE_TABLE_DESCS, &[]).expect("core tables");
    files.into_iter().find(|f| f.relative_path.ends_with(".h")).expect("C header").content
}

fn module_header(tables: &[&TableDesc]) -> Result<String, CodegenError> {
    generate_module_table_header("demo_cap", tables, "demo/nve_demo_cap_tables.generated.h".into())
        .map(|f| f.content)
}

#[test]
fn every_named_field_value_is_mirrored_once() {
    let h = core_header();
    let mut sets = 0;
    for desc in CORE_TABLE_DESCS {
        for f in desc.fields {
            // Every enum field names its variants (`word_enum!`), so hosts never hand-copy them;
            // packed codecs are not lists: `GridLine` (`Auto` / `Line(i)` / `Span(n)`) and
            // `OptAutofillHint` (`0` = none, else `AutofillHint + 1`).
            let packed = desc.name == "nve.grid_item" || f.name == "autofill";
            assert!(f.kind != FieldKind::Enum || packed || f.values.is_some(), "{}.{}", desc.name, f.name);
            let Some(set) = f.values else { continue };
            let banner = format!("/* `{}` field values. */", set.name);
            assert_eq!(h.matches(&banner).count(), 1, "{} emitted once", set.name);
            sets += 1;
            for (name, value) in set.values {
                let line = format!("#define NVE_{}_{} {value}u\n", screaming(set.name), screaming(name));
                assert!(h.contains(&line), "missing `{line}`");
            }
        }
    }
    assert!(sets > 30, "{sets} value-carrying fields");
    assert!(h.contains(&format!("#define NVE_ARIA_ROLE_SWITCH {}u", AriaRole::Switch.to_word())));
    assert!(h.contains(&format!("#define NVE_CURSOR_KIND_NOT_ALLOWED {}u", CursorKind::NotAllowed.to_word())));
    assert!(h.contains(&format!("#define NVE_TEXT_DECORATION_LINE_THROUGH {}u", text_decoration::LINE_THROUGH)));
    assert!(h.contains(&format!("#define NVE_TYPOGRAPHY_SET_FONT_SIZE {}u", Typography::SET_FONT_SIZE)));
}

static ROLE: ValueSetDesc = ValueSetDesc { name: "Mode", values: &[("Off", 0), ("On", 1)] };
static ROLE_OTHER: ValueSetDesc = ValueSetDesc { name: "Mode", values: &[("Off", 0), ("Auto", 1)] };
static WIDE: ValueSetDesc = ValueSetDesc { name: "Wide", values: &[("Big", 4)] };
static DUP: ValueSetDesc = ValueSetDesc { name: "Dup", values: &[("FooBar", 0), ("foo_bar", 1)] };

const fn field(name: &'static str, shift: u8, bits: u8, values: &'static ValueSetDesc) -> FieldDesc {
    FieldDesc { name, kind: FieldKind::Enum, word: 0, shift, bits, effects: 0, values: Some(values) }
}

const fn table(name: &'static str, fields: &'static [FieldDesc]) -> TableDesc {
    TableDesc {
        key: nve_hash::Key64::of(name),
        name,
        schema_hash: 7,
        storage: Storage::Sparse,
        flags: 2,
        defaults: &[0],
        word_effects: &[4],
        str_words: 0,
        resource_words: 0,
        fields,
    }
}

static SAME_TWICE: TableDesc = table("nve.same", &[field("a", 0, 2, &ROLE), field("b", 2, 2, &ROLE)]);
static CONFLICT: TableDesc = table("nve.conflict", &[field("a", 0, 2, &ROLE), field("b", 2, 2, &ROLE_OTHER)]);
static TOO_WIDE: TableDesc = table("nve.too_wide", &[field("a", 0, 2, &WIDE)]);
static DUPLICATE: TableDesc = table("nve.duplicate", &[field("a", 0, 2, &DUP)]);

#[test]
fn value_sets_are_deduplicated_and_validated() {
    let h = module_header(&[&SAME_TWICE]).expect("shared set");
    assert_eq!(h.matches("#define NVE_MODE_ON 1u").count(), 1);
    for (t, why) in [(&CONFLICT, "defined twice differently"), (&TOO_WIDE, "exceeds field"), (&DUPLICATE, "duplicate name")] {
        match module_header(&[t]) {
            Err(CodegenError::Validation(m)) => assert!(m.contains(why), "{}: {m}", t.name),
            other => panic!("{}: expected `{why}`, got {other:?}", t.name),
        }
    }
}

#[test]
fn module_header_mirrors_only_the_given_tables() {
    let visual = CORE_TABLE_DESCS.iter().find(|d| d.name == "nve.visual").unwrap();
    let h = module_header(&[visual]).expect("module header");
    assert!(h.contains("#ifndef NVE_DEMO_CAP_TABLES_GENERATED_H"));
    assert!(h.contains("#include <nve/host/table_word.h>"));
    assert!(h.contains("NVE_TABLE_CONST NveTableDesc NVE_TABLE_VISUAL_DESC = {"));
    assert!(!h.contains("NVE_TABLE_LAYOUT_KEY"), "only the given tables");
    assert!(!h.contains("NVE_TABLE_COUNT") && !h.contains("NVE_TABLE_LIST"), "core-only lists");
}

/// A program returning the number of `_DESC` fields that differ from the Rust descriptors.
fn desc_check(tables: &[&TableDesc], include: &str) -> String {
    let mut body = format!("#include {include}\n#include <stdio.h>\nint main(void) {{\n  int bad = 0;\n");
    for d in tables {
        let t = d.name.trim_start_matches("nve.").replace('.', "_").to_ascii_uppercase();
        let last = d.word_effects.len() - 1;
        body.push_str(&format!(
            "  bad += NVE_TABLE_{t}_DESC.key != {}ULL || NVE_TABLE_{t}_DESC.schema_hash != {}ULL\n    \
             || NVE_TABLE_{t}_DESC.name_len != {}u || NVE_TABLE_{t}_DESC.name[0] != 'n'\n    \
             || NVE_TABLE_{t}_DESC.storage != {}u || NVE_TABLE_{t}_DESC.flags != {}u\n    \
             || NVE_TABLE_{t}_DESC.words != {}u || NVE_TABLE_{t}_DESC.str_words != {}u\n    \
             || NVE_TABLE_{t}_DESC.resource_words != {}u || NVE_TABLE_{t}_DESC.defaults[{last}] != {}u\n    \
             || NVE_TABLE_{t}_DESC.word_effects[{last}] != {}u;\n",
            d.key.0,
            d.schema_hash,
            d.name.len(),
            d.storage as u8,
            d.flags,
            d.words(),
            d.str_words,
            d.resource_words,
            d.defaults[last],
            d.word_effects[last],
        ));
    }
    body.push_str("  printf(\"bad %d\\n\", bad);\n  return bad;\n}\n");
    body
}

fn compile_and_run(dir: &Path, src: &str, compiler: &str, std: &str) {
    let bin = dir.join(format!("{src}.{compiler}.bin"));
    let out = Command::new(compiler)
        .current_dir(dir)
        .args([std, "-Wall", "-Wextra", "-Werror", "-pedantic", "-I.", src, "-o"])
        .arg(&bin)
        .output()
        .expect("spawn compiler");
    assert!(out.status.success(), "{compiler} {src}:\n{}", String::from_utf8_lossy(&out.stderr));
    let run = Command::new(&bin).output().expect("run check");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "bad 0", "{compiler} {src}");
}

#[test]
fn desc_aggregates_match_rust_in_c11_and_cxx20() {
    if !common::require_tool("clang") || !common::require_tool("clang++") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("nve_table_values_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    common::copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../host-cxx/include/nve/host"),
        &dir.join("nve/host"),
    );
    std::fs::create_dir_all(dir.join("nve/generated")).unwrap();
    std::fs::write(dir.join("nve/generated/nve_tables.generated.h"), core_header()).unwrap();
    let visual = CORE_TABLE_DESCS.iter().find(|d| d.name == "nve.visual").unwrap();
    std::fs::create_dir_all(dir.join("demo")).unwrap();
    std::fs::write(dir.join("demo/nve_demo_cap_tables.generated.h"), module_header(&[visual]).unwrap()).unwrap();
    let core = desc_check(CORE_TABLE_DESCS, "<nve/generated/nve_tables.generated.h>");
    let module = desc_check(&[visual], "<demo/nve_demo_cap_tables.generated.h>");
    for (name, program) in [("core", core), ("module", module)] {
        for (ext, compiler, std) in [("c", "clang", "-std=c11"), ("cpp", "clang++", "-std=c++20")] {
            let src = format!("{name}.{ext}");
            std::fs::write(dir.join(&src), &program).unwrap();
            compile_and_run(&dir, &src, compiler, std);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
