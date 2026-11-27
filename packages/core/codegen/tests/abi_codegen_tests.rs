//! Host ABI codegen (`nve codegen abi`): determinism, output layout, template hygiene, 53-bit
//! safety of the Dart / TypeScript output, golden XXH64 vectors, table validation, and a real C11 /
//! C++20 compile of the generated header on every data model NVE runs on.

mod common;

use std::path::Path;
use std::process::Command;

use nve_abi::table::{FieldDesc, FieldKind, Storage, TableDesc};
use nve_codegen::{
    CodegenError, GeneratedFile, generate_abi_files, generate_all_abi_files, generate_c_abi_header,
    generate_hash_vector_files, generate_table_files,
};
use nve_components::tables::CORE_TABLE_DESCS;

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

fn all_files() -> Vec<GeneratedFile> {
    generate_all_abi_files(CORE_TABLE_DESCS).expect("generate every host ABI file")
}

#[test]
fn generation_is_deterministic_lf_only_and_lands_in_the_host_packages() {
    let first = all_files();
    let second = all_files();
    assert_eq!(
        first, second,
        "ABI codegen must be byte-for-byte deterministic"
    );
    assert_eq!(first.len(), 15);
    for f in &first {
        assert!(
            !f.content.contains('\r'),
            "{} contains a carriage return",
            f.relative_path
        );
        assert!(
            f.content.ends_with('\n'),
            "{} must end with a newline",
            f.relative_path
        );
        assert!(
            !f.content.contains(" \n"),
            "{} has trailing spaces",
            f.relative_path
        );
        assert!(
            ["cxx", "swift", "java", "dart", "ts"].iter().any(|h| f
                .relative_path
                .starts_with(&format!("packages/core/host-{h}/"))),
            "{} must land next to its static runtime",
            f.relative_path
        );
    }
    let update = std::env::var("NVE_UPDATE_ABI").is_ok();
    common::assert_generated_file_naming_and_optional_disk_parity(&first, repo_root(), update);
}

#[test]
fn templates_are_lf_only_and_under_650_lines() {
    let dir = repo_root().join("packages/core/codegen/templates");
    let mut checked = 0;
    for lang in std::fs::read_dir(&dir).expect("templates dir").flatten() {
        for t in std::fs::read_dir(lang.path())
            .expect("template language dir")
            .flatten()
        {
            let text = std::fs::read_to_string(t.path()).expect("read template");
            assert!(
                !text.contains('\r'),
                "{} contains a carriage return",
                t.path().display()
            );
            assert!(
                text.lines().count() < 650,
                "{} has 650+ lines",
                t.path().display()
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 15, "one template per generated file");
}

/// Every integer literal in Dart / TypeScript output stays below 2^53 (JavaScript numbers are
/// exact only up to there), and no 64-bit read or `BigInt` truncation is emitted.
#[test]
fn dart_and_typescript_output_is_53_bit_safe() {
    let mut scanned = 0;
    for f in all_files()
        .iter()
        .filter(|f| f.relative_path.ends_with(".dart") || f.relative_path.ends_with(".ts"))
    {
        for banned in [
            "getUint64",
            "getInt64",
            "setUint64",
            "setInt64",
            "BigInt",
            "Number(",
        ] {
            assert!(
                !f.content.contains(banned),
                "{} emits `{banned}`",
                f.relative_path
            );
        }
        for token in f
            .content
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        {
            let token = token.replace('_', "");
            let value = match token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
            {
                Some(hex) => u128::from_str_radix(hex, 16).ok(),
                None if token.bytes().all(|b| b.is_ascii_digit()) && !token.is_empty() => {
                    token.parse::<u128>().ok()
                }
                None => None,
            };
            if let Some(v) = value {
                assert!(
                    v < (1u128 << 53),
                    "{}: literal {token} is not exact in JavaScript",
                    f.relative_path
                );
            }
        }
        scanned += 1;
    }
    assert_eq!(scanned, 7, "4 Dart + 3 TypeScript files");
}

#[test]
fn golden_hash_vectors_come_from_nve_hash() {
    let c = generate_hash_vector_files().expect("vectors").remove(0);
    assert!(c.relative_path.ends_with("hash_vectors.generated.h"));
    let expected = nve_hash::xxh64(b"nve.visual", 0);
    assert!(c.content.contains(&format!("\"nve.visual\", \"6e76652e76697375616c\", UINT64_C(0x0000000000000000), UINT64_C(0x{expected:016X})")));
    assert_eq!(c.content.matches("UINT64_C(").count(), 2 * 19);
    let empty = nve_hash::xxh64(b"", 0);
    assert!(c.content.contains(&format!(
        "{{\"empty\", \"\", UINT64_C(0x0000000000000000), UINT64_C(0x{empty:016X})}}"
    )));
}

#[test]
fn c_header_carries_both_schema_hashes_and_no_hand_written_layout_cases() {
    let h = generate_c_abi_header().expect("C header");
    assert!(h.contains(&format!(
        "#define NVE_CORE_ABI_SCHEMA_HASH 0x{:016X}ULL",
        nve_abi::NVE_CORE_ABI_SCHEMA_HASH
    )));
    assert!(h.contains(&format!(
        "#define NVE_SCENE_SCHEMA_HASH 0x{:016X}ULL",
        nve_abi::NVE_SCENE_SCHEMA_HASH
    )));
    // i386 System V aligns 64-bit fields to 4: `NveDirectEvent` is 96 / 72 / 68 bytes.
    assert!(h.contains("sizeof(NveDirectEvent) == (sizeof(void*) == 8u ? 96u : (NVE_ABI_U64_FIELD_ALIGN == 8u ? 72u : 68u))"));
    assert!(h.contains("offsetof(NveSceneView, tables) == (sizeof(void*) == 8u ? 80u : 60u)"));
    assert!(h.contains("#include \"../host/api.h\""));
    assert_eq!(generate_abi_files().expect("abi")[0].content, h);
}

const fn field(name: &'static str, kind: FieldKind, word: u8, shift: u8, bits: u8) -> FieldDesc {
    FieldDesc {
        name,
        kind,
        word,
        shift,
        bits,
        effects: 0,
    }
}

const fn table(
    name: &'static str,
    words: &'static [u32],
    fields: &'static [FieldDesc],
) -> TableDesc {
    TableDesc {
        key: nve_hash::Key64::of(name),
        name,
        schema_hash: 1,
        storage: Storage::Sparse,
        flags: 0,
        defaults: words,
        word_effects: &[0, 0, 0],
        str_words: 0,
        resource_words: 0,
        fields,
    }
}

/// The validation message `t` must be rejected with, or the unexpected outcome.
fn try_table_error(t: &TableDesc) -> Result<String, String> {
    match generate_table_files(&[t]) {
        Err(CodegenError::Validation(msg)) => Ok(msg),
        other => Err(format!("expected a validation error, got {other:?}")),
    }
}

#[test]
fn table_specs_are_validated_before_any_host_sees_them() {
    let table_error = |t: &TableDesc| try_table_error(t).expect("validation error");
    static OK: TableDesc = table(
        "nve.test",
        &[0, 0, 0],
        &[
            field("title", FieldKind::Str, 0, 0, 64),
            field("flag", FieldKind::Bool, 2, 31, 1),
        ],
    );
    let files = generate_table_files(&[&OK]).expect("valid table");
    // A two-word `Str` is `bits = 64` in Rust and `(shift 0, bits 32)` on every host.
    assert!(
        files[0]
            .content
            .contains("NVE_TABLE_TEST_TITLE = { 0u, 0u, 32u, NVE_FIELD_STR }")
    );
    assert!(
        files[0]
            .content
            .contains("NVE_TABLE_TEST_FLAG = { 2u, 31u, 1u, NVE_FIELD_BOOL }")
    );

    static STR_PAST_END: TableDesc = table(
        "nve.test",
        &[0, 0, 0],
        &[field("s", FieldKind::Str, 2, 0, 64)],
    );
    assert!(table_error(&STR_PAST_END).contains("field `s` does not fit its row"));
    static BITS_PAST_WORD: TableDesc = table(
        "nve.test",
        &[0, 0, 0],
        &[field("b", FieldKind::U32, 0, 30, 4)],
    );
    assert!(table_error(&BITS_PAST_WORD).contains("field `b` does not fit its row"));
    static ZERO_BITS: TableDesc = table(
        "nve.test",
        &[0, 0, 0],
        &[field("z", FieldKind::Enum, 0, 0, 0)],
    );
    assert!(table_error(&ZERO_BITS).contains("field `z`"));
    static SHIFTED_STR: TableDesc = table(
        "nve.test",
        &[0, 0, 0],
        &[field("s", FieldKind::Str, 0, 8, 64)],
    );
    assert!(table_error(&SHIFTED_STR).contains("field `s`"));
    let mut wrong_key = table("nve.test", &[0, 0, 0], &[]);
    wrong_key.key = nve_hash::Key64::of("nve.other");
    assert!(table_error(&wrong_key).contains("key is not `xxh64(name)`"));
    let mut bad_effects = table("nve.test", &[0, 0, 0], &[]);
    bad_effects.word_effects = &[0];
    assert!(table_error(&bad_effects).contains("word count"));
    assert!(
        matches!(generate_table_files(&[&OK, &OK]), Err(CodegenError::Validation(m)) if m.contains("duplicate"))
    );
}

/// Compiles the generated header strictly as C11 and C++20 for every data model: the layout
/// asserts are checked by the compiler, not just rendered.
#[test]
fn generated_c_header_compiles_on_every_data_model() {
    if !common::tool_available("clang") {
        eprintln!("SKIP: clang not found");
        return;
    }
    let dir = std::env::temp_dir().join(format!("nve_abi_codegen_matrix_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    common::write_generated_c_abi_headers(&dir);
    let src = dir.join("check.c");
    std::fs::write(
        &src,
        "#include <nve/generated/nve_abi.generated.h>\nint f(void) { NveModule m = nve_resolve_module(0); return (int)sizeof m; }\n",
    )
    .expect("write C check");
    let targets = [
        None,
        Some("i386-unknown-none"),
        Some("i686-pc-windows-msvc"),
        Some("armv7-none-eabi"),
        Some("wasm32-unknown-unknown"),
        Some("aarch64-none-elf"),
    ];
    for target in targets {
        for (driver, std) in [("clang", "-std=c11"), ("clang++", "-std=c++20")] {
            let mut cmd = Command::new(driver);
            if let Some(t) = target {
                cmd.arg(format!("--target={t}")).arg("-ffreestanding");
            }
            if driver == "clang++" {
                cmd.args(["-x", "c++"]);
            }
            let out = cmd
                .args([
                    std,
                    "-fsyntax-only",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-pedantic",
                    "-I",
                ])
                .arg(&dir)
                .arg(&src)
                .output()
                .expect("run clang");
            assert!(
                out.status.success(),
                "{driver} {std} {target:?} rejected the generated header:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
