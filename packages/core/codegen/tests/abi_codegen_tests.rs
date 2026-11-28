//! Host ABI codegen (`nve codegen abi`): determinism, output layout, template hygiene, 53-bit
//! safety of the Dart / TypeScript output, golden XXH64 vectors, table validation, and a real C11 /
//! C++20 compile of the generated header on every data model NVE runs on.

mod common;

use std::path::Path;
use std::process::Command;

use nve_abi::table::{FieldDesc, FieldKind, NO_VAL_WORD, OptEncoding, Storage, TableDesc};
use nve_codegen::{
    CodegenError, GeneratedFile, generate_abi_files, generate_all_abi_files, generate_c_abi_header,
    generate_hash_vector_files, generate_table_files,
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
    assert_eq!(checked, 22, "one template per generated file");
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
