//! Core table host mirrors (`nve_tables.generated.{h,swift}`) match `nve_components` descriptors
//! field for field and compile cleanly as C11 / C++20.

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_abi::table::FieldKind;
use nve_codegen::targets::tables::{generate_all_table_files, generate_c_tables_h, generate_swift_tables};
use nve_components::tables::{Overflow, Typography, CORE_TABLE_DESCS};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn type_upper(table: &str) -> String {
    table.trim_start_matches("nve.").to_ascii_uppercase()
}

#[test]
fn every_core_field_is_mirrored_with_its_exact_word_layout() {
    let c = generate_c_tables_h();
    let swift = generate_swift_tables();
    assert_eq!(CORE_TABLE_DESCS.len(), 17);
    for desc in CORE_TABLE_DESCS {
        let t = type_upper(desc.name);
        assert!(c.contains(&format!("#define NVE_TABLE_{t}_KEY UINT64_C(0x{:016X})", desc.key.0)), "{}", desc.name);
        assert!(c.contains(&format!("#define NVE_TABLE_{t}_WORDS {}u", desc.defaults.len())));
        assert!(swift.contains(&format!("public static let key = Key64(0x{:016X})", desc.key.0)));
        for f in desc.fields {
            let bits = if f.kind == FieldKind::Str { 32 } else { f.bits };
            let line = format!(
                "static const NveFieldSpec NVE_{t}_{} = {{{}u, {}u, {bits}u,",
                f.name.trim_end_matches('_').to_ascii_uppercase(),
                f.word,
                f.shift
            );
            assert!(c.contains(&line), "missing `{line}`");
            assert!(swift.contains(&format!("spec({}, {}, {bits}, ", f.word, f.shift)));
        }
    }
    // Enum values and flag bits come from the Rust definitions.
    assert!(c.contains(&format!("#define NVE_OVERFLOW_SCROLL {}u", Overflow::Scroll.to_word())));
    assert!(c.contains(&format!("#define NVE_TYPOGRAPHY_SET_FONT_SIZE 0x{:X}u", Typography::SET_FONT_SIZE)));
    assert!(swift.contains("public enum NveOverflow: UInt32, Sendable, CaseIterable {"));
    assert!(swift.contains("    case scroll = 2"));
    assert!(swift.contains("public enum NveLinkTarget: UInt32, Sendable, CaseIterable {\n    case default_ = 0"));
    let paths: Vec<_> = generate_all_table_files().into_iter().map(|f| f.relative_path).collect();
    assert_eq!(
        paths,
        [
            "packages/core/host-cxx/include/nve/host/nve_tables.generated.h",
            "hosts/swiftui-kit/Sources/NveSwiftUI/NveTables.generated.swift",
        ]
    );
}

/// Validates every emitted spec with the host-side `nve_field_valid` / `nve_field_fits` helpers.
fn check_program() -> String {
    let mut body = String::from("#include \"nve_tables.generated.h\"\n#include <stdio.h>\nint main(void) {\n  int bad = 0;\n");
    for desc in CORE_TABLE_DESCS {
        let t = type_upper(desc.name);
        for f in desc.fields {
            let spec = format!("NVE_{t}_{}", f.name.trim_end_matches('_').to_ascii_uppercase());
            body.push_str(&format!(
                "  bad += !nve_field_valid({spec}) || !nve_field_fits({spec}, NVE_TABLE_{t}_WORDS) || NVE_TABLE_{t}_DEFAULTS[0] != {}u;\n",
                desc.defaults[0]
            ));
        }
    }
    body.push_str("  printf(\"ok %d\\n\", bad);\n  return bad;\n}\n");
    body
}

#[test]
fn c_header_compiles_as_c11_and_cxx20_and_every_spec_is_valid() {
    let c = generate_c_tables_h();
    let dir = std::env::temp_dir().join(format!("nve_tables_codegen_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let include = workspace_root().join("packages/core/host-cxx/include/nve/host");
    std::fs::write(dir.join("nve_tables.generated.h"), &c).unwrap();
    let program = check_program();
    std::fs::write(dir.join("check.c"), &program).unwrap();
    std::fs::write(dir.join("check.cpp"), &program).unwrap();
    for (compiler, std, src) in [("clang", "-std=c11", "check.c"), ("clang++", "-std=c++20", "check.cpp")] {
        let bin = dir.join(format!("{src}.bin"));
        let Ok(out) = Command::new(compiler)
            .current_dir(&dir)
            .args([std, "-Wall", "-Wextra", "-Werror", "-pedantic", "-I."])
            .arg(format!("-I{}", include.display()))
            .args([src, "-o"])
            .arg(&bin)
            .output()
        else {
            return; // No toolchain on this host.
        };
        assert!(out.status.success(), "{compiler} failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let run = Command::new(&bin).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "ok 0");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
