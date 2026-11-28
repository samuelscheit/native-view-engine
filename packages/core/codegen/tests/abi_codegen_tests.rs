//! Host ABI codegen (`nve codegen abi`): determinism, output layout, template hygiene, 53-bit
//! safety of the Dart / TypeScript output, golden XXH64 vectors, table validation, and a real C11 /
//! C++20 compile of the generated header on every data model NVE runs on.

mod common;

use std::path::Path;
use std::process::Command;

use nve_abi::table::{FieldDesc, FieldKind, NO_VAL_WORD, OptEncoding, Storage, TableDesc};
use nve_codec::{
    FlatOpDesc, FlatOpFieldDesc, FlatOpFieldType, FlatOpScopeDesc, FlatOpStreamDesc,
    FloatConstraint,
};
use nve_codegen::{
    CodegenError, GeneratedFile, generate_abi_files, generate_all_abi_files, generate_c_abi_header,
    generate_flat_op_codec_files, generate_hash_vector_files, generate_table_files,
};
use nve_tables::tables::CORE_TABLE_DESCS;

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
    assert_eq!(first.len(), 21);
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
    // One template per generated file, plus the shared `c/records.h` macros.
    assert_eq!(checked, 27, "one template per generated file");
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
fn c_headers_carry_their_schema_hashes_and_no_hand_written_layout_cases() {
    let h = generate_c_abi_header().expect("C header");
    assert!(h.contains(&format!(
        "#define NVE_CORE_ABI_SCHEMA_HASH 0x{:016X}ULL",
        nve_abi::NVE_CORE_ABI_SCHEMA_HASH
    )));
    // Uniform 64-byte pointer-free `NveEvent` (Rust slab-aligns to 64, C keeps natural alignment
    // for Swift ClangImporter compatibility) and `NveEventBatch`.
    assert!(h.contains("sizeof(NveEvent) == 64u"));
    assert!(h.contains("offsetof(NveEvent, bytes) == 56u"));
    assert!(h.contains("sizeof(NveEventBatch) == (sizeof(void*) == 8u ? 24u : 16u)"));
    assert!(h.contains(&format!(
        "#define NVE_TABLE_CUSTOM 0x{:016X}ULL",
        nve_abi::NVE_TABLE_CUSTOM.0
    )));
    assert!(!h.contains("NveDirectEvent"));
    // `NveValue` has no explicit alignment: i386 differs from other 32-bit targets.
    assert!(h.contains("sizeof(NveValue) == (sizeof(void*) == 8u ? 24u : (NVE_ABI_U64_FIELD_ALIGN == 8u ? 16u : 12u))"));
    assert!(h.contains("offsetof(NveTableView, row_index_pages) == (sizeof(void*) == 8u ? 64u : 48u)"));
    assert!(h.contains("#include \"../host/api.h\""));
    let files = generate_abi_files().expect("abi");
    assert_eq!(files[0].content, h);
    let rt = &files[1].content;
    assert!(rt.contains(&format!(
        "#define NVE_RUNTIME_SCHEMA_HASH 0x{:016X}ULL",
        nve_abi::NVE_RUNTIME_SCHEMA_HASH
    )));
    assert!(rt.contains("offsetof(NveSceneView, rows) == 56u"));
    assert!(rt.contains("offsetof(NveSceneView, tables) == (sizeof(void*) == 8u ? 104u : 80u)"));
}

const fn field(name: &'static str, kind: FieldKind, word: u8, shift: u8, bits: u8) -> FieldDesc {
    FieldDesc {
        name,
        kind,
        opt_encoding: OptEncoding::Required,
        word,
        shift,
        bits,
        effects: 0,
        val_word: NO_VAL_WORD,
        no_inherit: false,
        values: None,
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
        events_word: NO_VAL_WORD,
        events: None,
        inherited: false,
        fields,
        resource_kinds: &[],
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
            .contains("NVE_TABLE_TEST_TITLE = { 0u, 0u, 32u, NVE_FIELD_STR, NVE_OPT_REQUIRED, 0xFFu, 0xFFu, 0u }")
    );
    assert!(
        files[0]
            .content
            .contains("NVE_TABLE_TEST_FLAG = { 2u, 31u, 1u, NVE_FIELD_BOOL, NVE_OPT_REQUIRED, 0xFFu, 0xFFu, 0u }")
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

/// Compiles every generated C header (ABI, tables, XXH64 vectors) strictly as C11 and C++20 for
/// every data model: the layout asserts are checked by the compiler, not just rendered. A missing
/// `clang` skips, unless `CI` or `NVE_REQUIRE_CLANG` is set.
#[test]
fn generated_c_headers_compile_on_every_data_model() {
    if !common::tool_available("clang") || !common::tool_available("clang++") {
        assert!(
            std::env::var_os("CI").is_none() && std::env::var_os("NVE_REQUIRE_CLANG").is_none(),
            "clang is required (CI / NVE_REQUIRE_CLANG)"
        );
        eprintln!("SKIP: clang not found");
        return;
    }
    let dir = std::env::temp_dir().join(format!("nve_abi_codegen_matrix_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    common::write_generated_c_abi_headers(&dir);
    let src = dir.join("check.c");
    std::fs::write(
        &src,
        "#include <nve/generated/nve_abi.generated.h>\n#include <nve/generated/nve_tables.generated.h>\n\
         #include \"hash_vectors.generated.h\"\n\
         int f(void) { NveModule m = nve_resolve_module(0); return (int)sizeof m + (int)NVE_TABLE_COUNT\n\
         + (int)NVE_HASH_VECTOR_COUNT + NVE_TABLE_TEXT_TEXT.word; }\n",
    )
    .expect("write C check");
    // Degenerate table sets render valid C too: no tables at all, and a table without fields.
    static EMPTY: TableDesc = table("nve.empty", &[0, 0, 0], &[]);
    for (name, tables) in [("zero_tables", &[][..]), ("zero_fields", &[&EMPTY][..])] {
        let files = generate_table_files(tables).expect("degenerate table set");
        let header = files
            .iter()
            .find(|f| f.relative_path.ends_with(".h"))
            .expect("tables header");
        let sub = dir.join(name).join("nve/generated");
        std::fs::create_dir_all(&sub).expect("degenerate dir");
        // The header includes the runtime relative to itself (`../host/table_word.h`).
        common::copy_tree(
            &repo_root().join("packages/core/host-cxx/include/nve/host"),
            &dir.join(name).join("nve/host"),
        );
        std::fs::write(sub.join("nve_tables.generated.h"), &header.content)
            .expect("degenerate header");
        std::fs::write(
            dir.join(name).join("check.c"),
            "#include <nve/generated/nve_tables.generated.h>\nint f(void) { return (int)NVE_TABLE_COUNT; }\n",
        )
        .expect("degenerate check");
    }
    let targets = [
        None,
        Some("i386-unknown-none"),
        Some("i686-linux-android"),
        Some("i686-pc-windows-msvc"),
        Some("x86_64-pc-windows-msvc"),
        Some("armv7-none-eabi"),
        Some("arm64_32-apple-watchos"),
        Some("wasm32-unknown-unknown"),
        Some("aarch64-none-elf"),
    ];
    // The export macro's three spellings: consumer (default), the library itself, and a DLL import.
    let variants: [(&std::path::Path, &[&str]); 5] = [
        (&dir, &[]),
        (&dir, &["-DNVE_BUILDING_LIBRARY"]),
        (&dir, &["-DNVE_IMPORTING_LIBRARY"]),
        (&dir.join("zero_tables"), &[]),
        (&dir.join("zero_fields"), &[]),
    ];
    for (target, (include, defines)) in targets
        .iter()
        .flat_map(|t| variants.iter().map(move |v| (*t, *v)))
    {
        let src = include.join("check.c");
        for (driver, std) in [("clang", "-std=c11"), ("clang++", "-std=c++20")] {
            let mut cmd = Command::new(driver);
            cmd.args(defines);
            if let Some(t) = target {
                // A macOS `SDKROOT` would otherwise be applied to the cross target.
                cmd.env_remove("SDKROOT");
                cmd.arg(format!("--target={t}"))
                    .args(["-ffreestanding", "-Wno-incompatible-sysroot"]);
            }
            if driver == "clang++" {
                cmd.args(["-x", "c++"]);
                if target.is_some() {
                    // Freestanding: clang's own C headers, not the host SDK's libc++ wrappers.
                    cmd.arg("-nostdinc++");
                }
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
                .arg(include)
                .arg(&src)
                .output()
                .expect("run clang");
            assert!(
                out.status.success(),
                "{driver} {std} {target:?} {defines:?} {} rejected the generated headers:\n{}",
                include.display(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The C layouts codegen computes for every data model are exactly what `rustc` lays out on a
/// real target of that model: i686 Android (i386 System V), armv7 Android and wasm32 (ILP32), and
/// the 64-bit host. Each target compiles `const` asserts of every record's size, alignment, and
/// field offsets against `nve-abi`. A missing Rust target skips that target, unless `CI` or
/// `NVE_REQUIRE_CROSS` is set.
#[test]
fn record_layouts_match_rustc_on_every_data_model() {
    use nve_codegen::abi::layout::Model;
    let model = nve_codegen::abi::abi_model().expect("ABI model");
    let runtime = nve_codegen::abi::runtime_abi_model(&model).expect("runtime ABI model");
    let required =
        std::env::var_os("CI").is_some() || std::env::var_os("NVE_REQUIRE_CROSS").is_some();
    let rustc = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("run rustc -vV");
    let rustc = String::from_utf8_lossy(&rustc.stdout).into_owned();
    let host = rustc
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .expect("rustc host triple")
        .to_owned();
    let targets = [
        ("i686-linux-android", Model::I386),
        ("armv7-linux-androideabi", Model::P32),
        ("wasm32-unknown-unknown", Model::P32),
        (host.as_str(), Model::host()),
    ];
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();
    let installed = installed
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let installed = format!("{installed}\n{host}");
    // A per-run crate (parallel runs never share sources) with a shared, incremental target dir.
    let base = repo_root().join("target/nve-abi-layout-check");
    let dir = base.join(format!("run-{}", std::process::id()));
    // Removed however the test ends (a failed assert included), so runs never leave crates behind.
    struct RemoveOnDrop(std::path::PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = RemoveOnDrop(dir.clone());
    std::fs::create_dir_all(dir.join("src")).expect("check crate dir");
    // The workspace lockfile pins `nve-abi`'s dependencies to the versions the workspace uses.
    // (`--locked` cannot apply: Cargo prunes the workspace's other packages from this root's lock.)
    std::fs::copy(repo_root().join("Cargo.lock"), dir.join("Cargo.lock")).expect("copy Cargo.lock");
    let abi = repo_root().join("packages/core/abi");
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"nve-abi-layout-check\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
             [dependencies]\nnve-abi = {{ path = {:?} }}\n\n[workspace]\n",
            abi.display().to_string()
        ),
    )
    .expect("write Cargo.toml");
    let mut checked = 0;
    for (target, data_model) in targets {
        if !installed.lines().any(|l| l.trim() == target) {
            assert!(
                !required,
                "Rust target {target} is not installed (CI / NVE_REQUIRE_CROSS)"
            );
            eprintln!("SKIP: Rust target {target} not installed");
            continue;
        }
        let mut asserts = String::from(
            "#![no_std]\n#![allow(unused_imports)]\nuse nve_abi::*;\nuse nve_abi::table::NveTableView;\n",
        );
        for r in model.records.iter().chain(&runtime.records) {
            let rust_align = r.align.at(data_model).max(r.slab_align.unwrap_or(1));
            asserts += &format!(
                "const _: () = assert!(core::mem::size_of::<{0}>() == {1} && core::mem::align_of::<{0}>() == {2}, \"{0} size/align\");\n",
                r.name,
                r.size.at(data_model),
                rust_align
            );
            for f in &r.fields {
                asserts += &format!(
                    "const _: () = assert!(core::mem::offset_of!({0}, {1}) == {2}, \"{0}.{1}\");\n",
                    r.name,
                    f.name(),
                    f.offset.at(data_model)
                );
            }
        }
        std::fs::write(dir.join("src/lib.rs"), asserts).expect("write asserts");
        let out = Command::new(env!("CARGO"))
            .args([
                "check",
                "--offline",
                "--quiet",
                "--target",
                target,
                "--manifest-path",
            ])
            .arg(dir.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", base.join("target"))
            .output()
            .expect("run cargo check");
        assert!(
            out.status.success(),
            "C layouts for {target} ({data_model:?}) differ from rustc:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        checked += 1;
    }
    assert!(checked > 0 || !required, "no data model was checked");
}

/// `CNveAbi` is defined once (host-cxx's module map; host-swift's only refers to it), so Swift
/// imports it with both header directories on the search path, as an app linking the SwiftPM
/// package and an `nve build --xcframework` framework does.
#[test]
fn cnveabi_module_is_defined_once() {
    if !common::tool_available("swiftc") {
        assert!(
            std::env::var_os("NVE_REQUIRE_SWIFT").is_none(),
            "swiftc is required (NVE_REQUIRE_SWIFT)"
        );
        eprintln!("SKIP: swiftc not found");
        return;
    }
    let cxx = repo_root().join("packages/core/host-cxx/include");
    let swift = repo_root().join("packages/core/host-swift/Sources/CNveAbi/include");
    assert!(
        cxx.join("nve/generated/nve_abi.generated.h").exists(),
        "run `cargo run --locked -p nve-cli -- codegen abi` first"
    );
    for (dir, expected) in [(&cxx, 1), (&swift, 0)] {
        let map = dir.join("module.modulemap");
        let text = std::fs::read_to_string(&map).expect("module map");
        let definitions = text
            .lines()
            .filter(|l| l.trim_start().starts_with("module CNveAbi"))
            .count();
        assert_eq!(
            definitions,
            expected,
            "{} defines `CNveAbi` {definitions} times",
            map.display()
        );
    }
    let scratch = std::env::temp_dir().join(format!("nve_cnveabi_once_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("scratch dir");
    let src = scratch.join("m.swift");
    std::fs::write(
        &src,
        "import CNveAbi\nlet version: UInt32 = NVE_ABI_VERSION\n",
    )
    .expect("swift source");
    // The real maps, in both search orders: whichever Clang finds first, `CNveAbi` is one module.
    for (first, second) in [(&cxx, &swift), (&swift, &cxx)] {
        let out = Command::new("swiftc")
            .args(["-typecheck", "-module-cache-path"])
            .arg(scratch.join("module-cache"))
            .arg("-I")
            .arg(first)
            .arg("-I")
            .arg(second)
            .arg(&src)
            .output()
            .expect("run swiftc");
        assert!(
            out.status.success(),
            "swiftc -I {} -I {} rejected the two maps:\n{}",
            first.display(),
            second.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn flat_op_codec_generation_validates_is_deterministic_and_compiles_cpp20() {
    const STREAM: FlatOpStreamDesc = FlatOpStreamDesc {
        name: "MiniDraw",
        kind_name: "nve.res.mini_draw",
        max_ops_const_name: "MAX_MINI_OPS",
        max_ops: 1024,
        scope: Some(FlatOpScopeDesc {
            limit_name: "MAX_MINI_CLIP",
            push_opcode: 3,
            pop_opcode: 4,
            max_depth: 16,
        }),
        ops: &[
            FlatOpDesc {
                name: "Dot",
                opcode: 1,
                fields: &[
                    FlatOpFieldDesc {
                        name: "x",
                        ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                    },
                    FlatOpFieldDesc {
                        name: "radius",
                        ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
                    },
                    FlatOpFieldDesc {
                        name: "color",
                        ty: FlatOpFieldType::ColorRgba,
                    },
                ],
            },
            FlatOpDesc {
                name: "Path",
                opcode: 2,
                fields: &[
                    FlatOpFieldDesc {
                        name: "width",
                        ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
                    },
                    FlatOpFieldDesc {
                        name: "points",
                        ty: FlatOpFieldType::Point2Slice {
                            max_len: 256,
                            max_const_name: "MAX_MINI_POINTS",
                            coord_constraint: FloatConstraint::Finite,
                        },
                    },
                ],
            },
            FlatOpDesc {
                name: "Push",
                opcode: 3,
                fields: &[FlatOpFieldDesc {
                    name: "tag",
                    ty: FlatOpFieldType::U32,
                }],
            },
            FlatOpDesc {
                name: "Pop",
                opcode: 4,
                fields: &[],
            },
        ],
    };

    let first = generate_flat_op_codec_files(&STREAM).expect("generate mini flat-op codecs");
    let second = generate_flat_op_codec_files(&STREAM).expect("second deterministic pass");
    assert_eq!(first, second, "flat-op codegen must be deterministic");
    assert_eq!(first.len(), 5, "C++, Swift, Java, Dart, TS");
    assert!(
        first[0].content.contains("ColorRgba color{};"),
        "C++ ColorRgba fields must emit single-braced value initialization"
    );
    assert!(
        !first[0].content.contains("{{}}"),
        "C++ header must never emit accidental double-braced `{{}}` initializers"
    );
    for f in &first {
        assert!(!f.content.contains('\r'), "{} contains CR", f.relative_path);
        assert!(f.content.ends_with('\n'), "{} missing trailing LF", f.relative_path);
        assert!(!f.content.contains(" \n"), "{} has trailing spaces", f.relative_path);
        assert!(
            !f.content.contains("DrawList") && !f.content.contains("StrokePolyline"),
            "{} must be generic (no hardcoded DrawList identifiers)",
            f.relative_path
        );
    }

    // Invalid descriptors rejected up-front.
    const DUP_OPS: [FlatOpDesc; 2] = [STREAM.ops[0], STREAM.ops[0]];
    let mut bad_dup = STREAM;
    bad_dup.ops = &DUP_OPS;
    assert!(generate_flat_op_codec_files(&bad_dup).is_err());

    let mut bad_scope_pop = STREAM;
    bad_scope_pop.scope = Some(FlatOpScopeDesc {
        limit_name: "MAX_MINI_CLIP",
        push_opcode: 3,
        pop_opcode: 1, // Dot has fields; pop op must have zero fields
        max_depth: 16,
    });
    assert!(generate_flat_op_codec_files(&bad_scope_pop).is_err());

    let mut bad_slice_pos = STREAM;
    bad_slice_pos.ops = &[FlatOpDesc {
        name: "BadSlice",
        opcode: 1,
        fields: &[
            FlatOpFieldDesc {
                name: "points",
                ty: FlatOpFieldType::Point2Slice {
                    max_len: 8,
                    max_const_name: "MAX_BAD_PTS",
                    coord_constraint: FloatConstraint::Finite,
                },
            },
            FlatOpFieldDesc {
                name: "width",
                ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
            },
        ],
    }];
    bad_slice_pos.scope = None;
    assert!(generate_flat_op_codec_files(&bad_slice_pos).is_err());

    // Compile generated C++20 header strictly when clang++ is available.
    if common::tool_available("clang++") {
        let dir = std::env::temp_dir().join(format!("nve_flat_op_cxx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        std::fs::write(dir.join("mini.hpp"), &first[0].content).expect("write hpp");
        let src = dir.join("check.cpp");
        std::fs::write(
            &src,
            "#include \"mini.hpp\"\n\
             struct Sink {\n\
               void on_dot(const nve::generated::mini_draw::DotOp&) {}\n\
               void on_path(const nve::generated::mini_draw::PathOp&) {}\n\
               void on_push(const nve::generated::mini_draw::PushOp&) {}\n\
               void on_pop(const nve::generated::mini_draw::PopOp&) {}\n\
             };\n\
             int check(std::span<const uint8_t> bytes) {\n\
               Sink s{};\n\
               return nve::generated::mini_draw::decode(bytes, s).ok() ? 0 : 1;\n\
             }\n",
        )
        .expect("write cpp");
        let out = Command::new("clang++")
            .args([
                "-std=c++20",
                "-fsyntax-only",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-pedantic",
                "-Wconversion",
                "-Wsign-conversion",
                "-Wshadow",
            ])
            .arg(&src)
            .output()
            .expect("run clang++");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            out.status.success(),
            "clang++ rejected generated flat-op header:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn flat_op_codec_hardening_scoping_opcode_bounds_reserved_ops_no_scope_and_non_neg_coords() {
    // Exercises: (a) fields colliding with decoder locals (`offset`, `pos`, `length`, `end`, `op`,
    // `total`, `view`, `bytes`, `sink`, `slice`, `pt_count`), (d) `scope: None`, (e) 0-scalar
    // `Point2Slice`, (f) `coord_non_negative: true`, and inclusive `i32::MAX` opcode bound.
    const EDGE_STREAM: FlatOpStreamDesc = FlatOpStreamDesc {
        name: "EdgeOps",
        kind_name: "nve.res.edge_ops",
        max_ops_const_name: "MAX_EDGE_OPS",
        max_ops: 256,
        scope: None,
        ops: &[
            FlatOpDesc {
                name: "Span",
                opcode: 1,
                fields: &[
                    FlatOpFieldDesc {
                        name: "offset",
                        ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                    },
                    FlatOpFieldDesc {
                        name: "pos",
                        ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
                    },
                    FlatOpFieldDesc {
                        name: "length",
                        ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                    },
                    FlatOpFieldDesc {
                        name: "end",
                        ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                    },
                    FlatOpFieldDesc {
                        name: "op",
                        ty: FlatOpFieldType::U32,
                    },
                    FlatOpFieldDesc {
                        name: "total",
                        ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                    },
                    FlatOpFieldDesc {
                        name: "view",
                        ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                    },
                    FlatOpFieldDesc {
                        name: "bytes",
                        ty: FlatOpFieldType::ColorRgba,
                    },
                    FlatOpFieldDesc {
                        name: "sink",
                        ty: FlatOpFieldType::U32,
                    },
                ],
            },
            FlatOpDesc {
                name: "PolyWithLocals",
                opcode: 2,
                fields: &[
                    FlatOpFieldDesc {
                        name: "pos",
                        ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
                    },
                    FlatOpFieldDesc {
                        name: "bytes",
                        ty: FlatOpFieldType::U32,
                    },
                    FlatOpFieldDesc {
                        name: "sink",
                        ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                    },
                    FlatOpFieldDesc {
                        name: "slice",
                        ty: FlatOpFieldType::ColorRgba,
                    },
                    FlatOpFieldDesc {
                        name: "pt_count",
                        ty: FlatOpFieldType::U32,
                    },
                    FlatOpFieldDesc {
                        name: "points",
                        ty: FlatOpFieldType::Point2Slice {
                            max_len: 64,
                            max_const_name: "MAX_EDGE_POINTS",
                            coord_constraint: FloatConstraint::Finite,
                        },
                    },
                ],
            },
            FlatOpDesc {
                name: "BarePoints",
                opcode: i32::MAX as u32,
                fields: &[FlatOpFieldDesc {
                    name: "verts",
                    ty: FlatOpFieldType::Point2Slice {
                        max_len: 32,
                        max_const_name: "MAX_BARE_VERTS",
                        coord_constraint: FloatConstraint::NonNegative,
                    },
                }],
            },
        ],
    };

    let files = generate_flat_op_codec_files(&EDGE_STREAM).expect("generate edge flat-op codecs");
    assert_eq!(files.len(), 5);
    let [cxx, swift, java, dart, ts] = &files[..] else {
        unreachable!();
    };

    // C++: single-braced ColorRgba init, no ScopeGuard, exact failing point byte offset, coord >= 0.
    assert!(cxx.content.contains("ColorRgba bytes{};"));
    assert!(!cxx.content.contains("{{}}"));
    assert!(!cxx.content.contains("ScopeGuard"));
    assert!(cxx.content.contains("if (px < 0.0f || py < 0.0f)"));
    assert!(cxx.content.contains("pos + static_cast<size_t>(p) * 8u + 8u"));

    // Swift: prefixed locals, @inlinable, UInt32 opCount, 0-scalar Point2Slice, coord >= 0, no scope.
    assert!(swift.content.contains("let _f_pos ="));
    assert!(swift.content.contains("let _f_bytes ="));
    assert!(swift.content.contains("let _f_sink ="));
    assert!(swift.content.contains("let _f_slice ="));
    assert!(swift.content.contains("onBarePoints(verts: NveEdgeOpsPointSlice)"));
    assert!(swift.content.contains("guard px >= 0, py >= 0 else { return .negativeFloat }"));
    assert!(swift.content.contains("public static func opCount(of bytes: UnsafeRawBufferPointer) -> UInt32"));
    assert!(swift.content.contains("@inlinable\n    public static func streamValidated<S: NveEdgeOpsSink>"));
    assert!(!swift.content.contains("scopeDepth"));

    // Java: prefixed float locals, 0-scalar Point2Slice, coord >= 0, no scope finally.
    assert!(java.content.contains("final float _f_offset ="));
    assert!(java.content.contains("final float _f_pos ="));
    assert!(java.content.contains("void onBarePointsStart(int count);"));
    assert!(java.content.contains("if (px < 0.0f || py < 0.0f) return Error.NEGATIVE_FLOAT;"));
    assert!(!java.content.contains("finally"));

    // Dart: shared ByteData view helpers, 0-scalar Point2Slice, coord >= 0, no scope finally.
    assert!(dart.content.contains("validateView(ByteData data)"));
    assert!(dart.content.contains("streamValidatedView(ByteData data, NveEdgeOpsSink sink)"));
    assert!(dart.content.contains("void onBarePointsStart(int count);"));
    assert!(dart.content.contains("if (px < 0.0 || py < 0.0)"));
    assert!(!dart.content.contains("finally"));

    // TS: prefixed float locals, shared DataView helpers, 0-scalar Point2Slice, coord >= 0, no finally.
    assert!(ts.content.contains("const _f_offset ="));
    assert!(ts.content.contains("const _f_pos ="));
    assert!(ts.content.contains("const _f_view ="));
    assert!(ts.content.contains("function validateEdgeOpsView(view: DataView)"));
    assert!(ts.content.contains("onBarePointsStart(count: number): void;"));
    assert!(ts.content.contains("if (px < 0 || py < 0)"));
    assert!(!ts.content.contains("finally"));

    // (b) `opcode > i32::MAX` (and 0) rejected up-front.
    const BAD_OPCODES: &[&[FlatOpDesc]] = &[
        &[FlatOpDesc { name: "Probe", opcode: 0, fields: &[] }],
        &[FlatOpDesc { name: "Probe", opcode: (i32::MAX as u32) + 1, fields: &[] }],
        &[FlatOpDesc { name: "Probe", opcode: u32::MAX, fields: &[] }],
    ];
    for &ops in BAD_OPCODES {
        let mut bad = EDGE_STREAM;
        bad.ops = ops;
        let err = generate_flat_op_codec_files(&bad).expect_err("invalid opcode must fail");
        assert!(
            matches!(err, CodegenError::Validation(ref m) if m.contains("opcode")),
            "unexpected error for opcode {}: {err:?}",
            ops[0].opcode
        );
    }

    // (c) Reserved op names (and reserved stream names) rejected up-front.
    const RESERVED_OP_CASES: &[&[FlatOpDesc]] = &[
        &[FlatOpDesc { name: "Return", opcode: 1, fields: &[] }],
        &[FlatOpDesc { name: "Switch", opcode: 1, fields: &[] }],
        &[FlatOpDesc { name: "Default", opcode: 1, fields: &[] }],
        &[FlatOpDesc { name: "None", opcode: 1, fields: &[] }],
        &[FlatOpDesc { name: "Fallthrough", opcode: 1, fields: &[] }],
    ];
    for &ops in RESERVED_OP_CASES {
        let mut bad = EDGE_STREAM;
        bad.ops = ops;
        let err = generate_flat_op_codec_files(&bad).expect_err("reserved op name must fail");
        assert!(
            matches!(err, CodegenError::Validation(ref m) if m.contains("reserved keyword")),
            "unexpected error for op `{}`: {err:?}",
            ops[0].name
        );
    }
    let mut bad_stream = EDGE_STREAM;
    bad_stream.name = "Switch";
    assert!(matches!(
        generate_flat_op_codec_files(&bad_stream),
        Err(CodegenError::Validation(ref m)) if m.contains("reserved keyword")
    ));

    // Compile the generated colliding-field / scope:None / 0-scalar Point2Slice codecs with real compilers.
    let dir = std::env::temp_dir().join(format!("nve_flat_op_edge_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");

    if common::tool_available("clang++") {
        std::fs::write(dir.join("edge.hpp"), &cxx.content).expect("write hpp");
        let cpp = dir.join("check.cpp");
        std::fs::write(
            &cpp,
            "#include \"edge.hpp\"\n\
             struct Sink {\n\
               void on_span(const nve::generated::edge_ops::SpanOp&) {}\n\
               void on_poly_with_locals(const nve::generated::edge_ops::PolyWithLocalsOp&) {}\n\
               void on_bare_points(const nve::generated::edge_ops::BarePointsOp&) {}\n\
             };\n\
             int check(std::span<const uint8_t> b) { Sink s{}; return nve::generated::edge_ops::decode(b, s).ok() ? 0 : 1; }\n",
        )
        .expect("write cpp");
        let out = Command::new("clang++")
            .args([
                "-std=c++20", "-fsyntax-only", "-Wall", "-Wextra", "-Werror", "-pedantic",
                "-Wconversion", "-Wsign-conversion", "-Wshadow",
            ])
            .arg(&cpp)
            .output()
            .expect("run clang++");
        assert!(
            out.status.success(),
            "clang++ rejected edge flat-op header:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    if common::tool_available("javac") {
        let jdir = dir.join("java");
        let jgen = jdir.join("dev/nve/host/generated");
        let jout = jdir.join("out");
        std::fs::create_dir_all(&jgen).expect("java gen dir");
        std::fs::create_dir_all(&jout).expect("java out dir");
        std::fs::write(
            jdir.join("dev/nve/host/Key64.java"),
            "package dev.nve.host;\npublic final class Key64 { public static Key64 fromHalves(int lo, int hi) { return new Key64(); } }\n",
        )
        .expect("write Key64 stub");
        let jsrc = jgen.join("NveEdgeOps.java");
        std::fs::write(&jsrc, &java.content).expect("write java");
        let out = Command::new("javac")
            .args(["--release", "11", "-Xlint:all", "-Werror", "-d"])
            .arg(&jout)
            .arg(jdir.join("dev/nve/host/Key64.java"))
            .arg(&jsrc)
            .output()
            .expect("run javac");
        assert!(
            out.status.success(),
            "javac rejected edge flat-op codec:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    if common::tool_available("swiftc") {
        let sdir = dir.join("swift");
        std::fs::create_dir_all(&sdir).expect("swift dir");
        let key_stub = sdir.join("Key64.swift");
        let ssrc = sdir.join("NveEdgeOps.swift");
        std::fs::write(
            &key_stub,
            "public struct Key64: Sendable { public init(lo: UInt32, hi: UInt32) {} }\n",
        )
        .expect("write swift Key64 stub");
        std::fs::write(&ssrc, &swift.content).expect("write swift");
        let out = Command::new("swiftc")
            .args(["-typecheck", "-parse-as-library", "-swift-version", "6", "-warnings-as-errors"])
            .arg(&key_stub)
            .arg(&ssrc)
            .output()
            .expect("run swiftc");
        assert!(
            out.status.success(),
            "swiftc rejected edge flat-op codec:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    if common::tool_available("tsc") {
        let tdir = dir.join("ts");
        let tgen = tdir.join("generated");
        std::fs::create_dir_all(&tgen).expect("ts gen dir");
        std::fs::write(
            tdir.join("key64.ts"),
            "export class Key64 { constructor(_lo: number, _hi: number) {} }\n",
        )
        .expect("write ts key64 stub");
        std::fs::write(tgen.join("nve_edge_ops.generated.ts"), &ts.content).expect("write ts");
        let tsconfig = tdir.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            common::ts::STRICT_TSCONFIG.replace("\"./*.ts\"", "\"./**/*.ts\""),
        )
        .expect("write tsconfig");
        common::ts::run_tsc_no_emit(&tsconfig);
    }

    let _ = std::fs::remove_dir_all(&dir);
}
