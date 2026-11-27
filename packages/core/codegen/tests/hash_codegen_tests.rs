//! Cross-language verification of the generated `nve_hash.generated.*` XXH64 mirrors: every host
//! compiles its mirror and runs its embedded golden-vector self-check (one-shot + chunked
//! streaming), plus `const_id("abc")` against Rust. Dart additionally runs compiled to JavaScript
//! to prove the `(lo, hi)` halves stay exact on 53-bit doubles. Missing toolchains skip.

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::targets::hash::{generate_all_hash_files, HASH_VECTORS};

const ABC_ID: u64 = nve_hash::const_id("abc");

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nve_hash_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Writes the generated mirror whose path ends with `suffix` into `dir/name`.
fn write_mirror(dir: &Path, suffix: &str, name: &str) {
    let file = generate_all_hash_files()
        .into_iter()
        .find(|f| f.relative_path.ends_with(suffix))
        .expect("generated hash mirror should exist for suffix");
    std::fs::write(dir.join(name), file.content).expect("write mirror");
}

/// Runs `cmd`; `None` when the tool is not installed, panics on failure.
fn run(cmd: &mut Command) -> Option<String> {
    let out = match cmd.output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping: {:?} not installed", cmd.get_program());
            return None;
        }
        Err(e) => {
            #[expect(clippy::panic, reason = "test helper fails when external tool spawn fails")]
            {
                panic!("spawn {:?}: {e}", cmd.get_program());
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{:?} failed:\n{stdout}\n{}",
        cmd.get_program(),
        String::from_utf8_lossy(&out.stderr)
    );
    Some(stdout)
}

fn assert_ok_line(stdout: &str) {
    assert_eq!(stdout.trim(), format!("ok {ABC_ID:016x}"));
}

#[test]
fn golden_vectors_match_canonical_rust_xxh64() {
    assert!(HASH_VECTORS.len() >= 8);
    for v in HASH_VECTORS {
        assert_eq!(v.expected(), nve_hash::xxh64(v.bytes, v.seed), "{}", v.label);
    }
    assert_eq!(ABC_ID, 0x44BC_2CF5_AD77_0999);
    let files = generate_all_hash_files();
    assert_eq!(files.len(), 5);
    for f in &files {
        assert!(f.relative_path.contains(".generated.") || f.relative_path.ends_with("NveHash.java"));
        assert!(f.content.starts_with("// @generated"), "{}", f.relative_path);
    }
}

#[test]
fn c11_and_cpp20_mirrors_compile_and_verify() {
    let dir = temp_dir("c");
    write_mirror(&dir, "nve_hash.generated.h", "nve_hash.generated.h");
    write_mirror(&dir, "nve_hash.generated.hpp", "nve_hash.generated.hpp");
    std::fs::write(
        dir.join("c.c"),
        "#include \"nve_hash.generated.h\"\n#include <stdio.h>\nint main(void) {\n  \
         if (!nve_hash_verify_test_vectors()) return 1;\n  \
         printf(\"ok %016llx\\n\", (unsigned long long)nve_xxh64(\"abc\", 3, 0));\n  return 0;\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("cpp.cpp"),
        "#include \"nve_hash.generated.hpp\"\n#include <cstdio>\n\
         static_assert(nve::Key64::of(\"abc\").value == nve::const_id(\"abc\"));\n\
         static_assert(nve::Key64::from_halves(nve::Key64::of(\"abc\").lo(), nve::Key64::of(\"abc\").hi()) == nve::Key64::of(\"abc\"));\n\
         int main() { std::printf(\"ok %016llx\\n\", (unsigned long long)nve::const_id(\"abc\")); }\n",
    )
    .unwrap();
    let Some(_) = run(Command::new("clang")
        .current_dir(&dir)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic", "c.c", "-o", "c_bin"]))
    else {
        return;
    };
    assert_ok_line(&run(&mut Command::new(dir.join("c_bin"))).unwrap());
    run(Command::new("clang++")
        .current_dir(&dir)
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "cpp.cpp", "-o", "cpp_bin"]))
    .expect("clang++ next to clang");
    assert_ok_line(&run(&mut Command::new(dir.join("cpp_bin"))).unwrap());
}

#[test]
fn java_mirror_compiles_and_verifies() {
    let dir = temp_dir("java");
    let pkg = dir.join("dev/nve/android/generated");
    std::fs::create_dir_all(&pkg).unwrap();
    write_mirror(&pkg, "NveHash.java", "NveHash.java");
    std::fs::write(
        dir.join("Check.java"),
        "import dev.nve.android.generated.NveHash;\npublic final class Check {\n  \
         public static void main(String[] a) {\n    if (!NveHash.verifyTestVectors()) throw new AssertionError();\n    \
         long id = NveHash.constId(\"abc\");\n    \
         if (NveHash.fromHalves(NveHash.lo(id), NveHash.hi(id)) != id) throw new AssertionError();\n    \
         System.out.println(\"ok \" + String.format(\"%016x\", id));\n  }\n}\n",
    )
    .unwrap();
    let Some(_) = run(Command::new("javac").current_dir(&dir).args([
        "-Xlint:all",
        "-Werror",
        "-d",
        "classes",
        "Check.java",
        "dev/nve/android/generated/NveHash.java",
    ])) else {
        return;
    };
    assert_ok_line(&run(Command::new("java").current_dir(&dir).args(["-cp", "classes", "Check"])).unwrap());
}

const JS_CHECK: &str = "if (!nveHashVerifyTestVectors()) throw new Error('vectors');\n\
    const k = nveConstId('abc');\n\
    const out = new Uint8Array(8); k.writeLE(out); if (!NveKey64.readLE(out).equals(k)) throw new Error('roundtrip');\n\
    console.log('ok ' + k.toHex());\n";

#[test]
fn typescript_mirror_runs_on_node_without_bigint() {
    let dir = temp_dir("ts");
    write_mirror(&dir, "nve_hash.generated.ts", "nve_hash.generated.ts");
    let src = generate_all_hash_files()
        .into_iter()
        .find(|f| f.relative_path.ends_with(".ts"))
        .unwrap()
        .content;
    assert!(!src.contains("bigint") && !src.contains("BigInt"));
    std::fs::write(
        dir.join("check.ts"),
        format!(
            "import {{ NveKey64, nveConstId, nveHashVerifyTestVectors }} from './nve_hash.generated.ts';\n{JS_CHECK}"
        ),
    )
    .unwrap();
    if let Some(out) = run(Command::new("node").current_dir(&dir).arg("check.ts")) {
        assert_ok_line(&out);
    }
}

#[test]
fn dart_mirror_verifies_on_vm_and_compiled_to_javascript() {
    let dir = temp_dir("dart");
    write_mirror(&dir, "nve_hash.generated.dart", "nve_hash.generated.dart");
    std::fs::write(
        dir.join("check.dart"),
        "import 'nve_hash.generated.dart';\nvoid main() {\n  \
         if (!nveHashVerifyTestVectors()) throw StateError('vectors');\n  \
         final k = NveKey64.of('abc');\n  final out = List<int>.filled(8, 0);\n  k.writeLE(out);\n  \
         if (NveKey64.readLE(out) != k) throw StateError('roundtrip');\n  print('ok ${k.toHex()}');\n}\n",
    )
    .unwrap();
    let Some(_) = run(Command::new("dart")
        .current_dir(&dir)
        .args(["analyze", "--fatal-infos", "nve_hash.generated.dart", "check.dart"]))
    else {
        return;
    };
    assert_ok_line(&run(Command::new("dart").current_dir(&dir).args(["run", "check.dart"])).unwrap());
    run(Command::new("dart").current_dir(&dir).args(["compile", "js", "-O2", "-o", "check.js", "check.dart"]))
        .expect("dart compile js");
    if let Some(out) = run(Command::new("node").current_dir(&dir).arg("check.js")) {
        assert_ok_line(&out);
    }
}
