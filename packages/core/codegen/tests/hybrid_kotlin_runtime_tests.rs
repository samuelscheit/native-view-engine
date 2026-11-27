//! Kotlin / JNI hybrid target, compiled and run for real:
//! - the generated Kotlin (two specs in one package) compiles with `kotlinc -Werror` against the
//!   static `dev.nve.host` runtime and round-trips on the JVM; its `native` methods match the JNI
//!   glue's exports one to one;
//! - a dedicated `OverlayView` spec (non-POD struct with `Vec` + `Option` + `NveValue` + enum, as
//!   props, arguments, and results) runs its JNI glue live against a Rust provider, on a fake
//!   `JNIEnv` that checks every `NveValue` tag `nativeSetProps` builds, and on a real JVM.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::{
    GeneratedFile, HybridFileSpec, HybridTarget, generate_hybrid_target, parse_hybrid_rust_spec,
};

const REALISTIC_HYBRID_SPEC: &str = include_str!("fixtures/realistic_hybrid.nve.rs");
const FAKE_JNI_H: &str = include_str!("fixtures/fake_jni.h");
const JNI_TEST_SUPPORT_H: &str = include_str!("fixtures/jni_test_support.h");
const JNI_OVERLAY_HARNESS_CPP: &str = include_str!("fixtures/jni_overlay_harness.cpp");
const KOTLIN_MAIN_KT: &str = include_str!("fixtures/kotlin_hybrid_main.kt");
/// `OverlayView`: a non-POD struct (Vec + Option + NveValue + enum) as props, arguments, results.
const OVERLAY_SPEC: &str = include_str!("fixtures/overlay_hybrid.nve.rs");
/// The live Rust `OverlayView` provider (staticlib exporting `nve_get_interface`).
const OVERLAY_PROVIDER_RS: &str = include_str!("fixtures/overlay_provider.rs");
/// Kotlin-only edge cases (compiled with `kotlinc -Werror`, natives checked against the glue): a
/// parameter named `handle` and fields / parameters named after Kotlin hard keywords.
const EDGE_SPEC: &str = include_str!("fixtures/edge_hybrid.nve.rs");

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nve_kotlin_hybrid_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn specs() -> (HybridFileSpec, HybridFileSpec) {
    (
        parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("realistic spec parses"),
        parse_hybrid_rust_spec(OVERLAY_SPEC).expect("overlay spec parses"),
    )
}

/// Every spec the Kotlin app compiles together in `dev.nve.hybrid`.
fn kotlin_specs() -> [HybridFileSpec; 3] {
    let (realistic, overlay) = specs();
    [
        realistic,
        overlay,
        parse_hybrid_rust_spec(EDGE_SPEC).expect("edge spec parses"),
    ]
}

/// Generates `target` for `spec` into `dir`, returning the files.
fn generate(spec: &HybridFileSpec, target: HybridTarget, dir: &Path) -> Vec<GeneratedFile> {
    let out = generate_hybrid_target(spec, target).expect("hybrid codegen");
    out.write_to_dir(dir).expect("write generated files");
    out.files
}

/// Runs `cmd`, asserting success; returns stdout.
fn run(cmd: &mut Command, what: &str) -> String {
    let out = cmd.output().expect("start the command");
    assert!(
        out.status.success(),
        "{what} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `javac`, `java`, and `kotlinc` (which only knows `-version`); a missing one skips the caller
/// unless `NVE_REQUIRE_<TOOL>` is set.
fn jvm_tools_available() -> bool {
    let kotlinc = Command::new("kotlinc")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !kotlinc {
        assert!(
            std::env::var_os("NVE_REQUIRE_KOTLINC").is_none(),
            "`kotlinc` is required (NVE_REQUIRE_KOTLINC)"
        );
        eprintln!("SKIP: `kotlinc` not found");
        return false;
    }
    common::require_tool("javac") && common::require_tool("java")
}

/// Compiles the generated Kotlin of both specs plus the driver (`kotlinc -Werror`) against the
/// static `dev.nve.host` runtime (`javac --release 11 -Werror`). Returns the run classpath.
fn build_kotlin_app(dir: &Path) -> String {
    let kt_dir = dir.join("kotlin");
    for spec in &kotlin_specs() {
        for file in generate(spec, HybridTarget::Kotlin, &kt_dir) {
            if file.relative_path.ends_with(".kt") {
                // The static runtime is reused, never re-emitted into generated code.
                assert!(
                    !file.content.contains("class NveBin"),
                    "{} copies a codec",
                    file.relative_path
                );
                assert!(file.content.contains("import dev.nve.host.BinWriter"));
            }
        }
    }
    std::fs::write(kt_dir.join("KotlinHybridMain.kt"), KOTLIN_MAIN_KT)
        .expect("write Kotlin driver");

    let host_classes = dir.join("host-java");
    let host_sources: Vec<PathBuf> = walk(
        &workspace_root().join("packages/core/host-java/src/main"),
        "java",
    );
    run(
        Command::new("javac")
            .args(["--release", "11", "-Xlint:all", "-Werror", "-d"])
            .arg(&host_classes)
            .args(&host_sources),
        "javac host-java",
    );
    assert_no_covariant_buffer_calls(&host_classes);
    let jar = dir.join("kotlin-hybrid.jar");
    let kt_sources = walk(&kt_dir, "kt");
    run(
        Command::new("kotlinc")
            .args(["-Werror", "-include-runtime", "-cp"])
            .arg(&host_classes)
            .args(&kt_sources)
            .arg("-d")
            .arg(&jar),
        "kotlinc -Werror generated Kotlin",
    );
    std::env::join_paths([jar, host_classes])
        .expect("classpath")
        .to_string_lossy()
        .into_owned()
}

fn walk(root: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).expect("read dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path, ext));
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Android runtimes before the Java 9 covariant `ByteBuffer` overrides (`flip`, `clear`,
/// `position(int)`, ...) throw `NoSuchMethodError` for them: host-java must call them through
/// `Buffer`. Checked on the bytecode `javac --release 11` emitted.
fn assert_no_covariant_buffer_calls(classes: &Path) {
    let class_files = walk(classes, "class");
    let disassembly = run(
        Command::new("javap").args(["-c", "-p"]).args(&class_files),
        "javap host-java",
    );
    const COVARIANT: [&str; 7] = [
        "flip", "clear", "rewind", "mark", "reset", "position", "limit",
    ];
    let offending: Vec<&str> = disassembly
        .lines()
        .filter(|l| {
            l.split("Method java/nio/ByteBuffer.")
                .nth(1)
                .is_some_and(|m| {
                    COVARIANT
                        .iter()
                        .any(|name| m.starts_with(&format!("{name}:")))
                        && m.ends_with(")Ljava/nio/ByteBuffer;")
                })
        })
        .collect();
    assert!(
        offending.is_empty(),
        "covariant ByteBuffer calls:\n{}",
        offending.join("\n")
    );
}

/// `<Class> <method> <return> <params,...>` (JNI C types, Java parameters only) of every
/// `JNIEXPORT` in the generated JNI glue.
fn jni_exports(glue: &str) -> BTreeSet<String> {
    glue.lines()
        .filter_map(|line| line.strip_prefix("JNIEXPORT "))
        .map(|line| {
            let (ret, rest) = line
                .split_once(" JNICALL Java_dev_nve_hybrid_")
                .expect("JNI symbol");
            let (name, params) = rest.split_once('(').expect("parameter list");
            let (class, method) = name.rsplit_once('_').expect("class_method");
            let params = params.split_once(')').expect("closing paren").0;
            // `JNIEnv*` and `jclass` precede the Java parameters.
            let types: Vec<&str> = params
                .split(", ")
                .skip(2)
                .map(|p| p.split_whitespace().next().expect("parameter type"))
                .collect();
            format!("{class} {method} {ret} {}", types.join(","))
        })
        .collect()
}

#[test]
fn generated_kotlin_compiles_with_werror_and_round_trips_on_the_jvm() {
    if !jvm_tools_available() {
        return;
    }
    let dir = temp_test_dir("codec");
    let classpath = build_kotlin_app(&dir);
    let stdout = run(
        Command::new("java").args([
            "-cp",
            &classpath,
            "dev.nve.hybrid.KotlinHybridMainKt",
            "codec",
        ]),
        "Kotlin codec driver",
    );

    // Schema constants: `const val` literals hold the exact u32 / u64 bits.
    let mut expected_schema = BTreeSet::new();
    for spec in &kotlin_specs() {
        for m in &spec.modules {
            expected_schema.insert(format!(
                "schema {} {} {}",
                m.name,
                m.interface_id().cast_signed(),
                m.schema_hash(spec).cast_signed()
            ));
        }
        for v in &spec.views {
            expected_schema.insert(format!(
                "schema {} {} {}",
                v.name,
                v.interface_id().cast_signed(),
                v.schema_hash(spec).cast_signed()
            ));
        }
        expected_schema.insert(format!(
            "file {} 1 {}",
            spec.stem_pascal(),
            spec.file_schema_hash().cast_signed()
        ));
    }
    let printed: BTreeSet<String> = stdout
        .lines()
        .filter(|l| l.starts_with("schema ") || l.starts_with("file "))
        .map(str::to_string)
        .collect();
    assert_eq!(printed, expected_schema);

    // Every Kotlin `native` binds to exactly one JNI export with the same JNI return and
    // parameter types, and vice versa.
    let natives: BTreeSet<String> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("native "))
        .map(str::to_string)
        .collect();
    let mut exports = BTreeSet::new();
    for glue in walk(&dir.join("kotlin"), "cpp") {
        exports.extend(jni_exports(
            &std::fs::read_to_string(glue).expect("read JNI glue"),
        ));
    }
    assert!(natives.len() >= 20, "too few natives: {natives:?}");
    assert_eq!(natives, exports);
    assert!(stdout.contains("kotlin hybrid: ok"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `rustc`: the generated `overlay_view_hybrid` rlib, then the provider staticlib exporting
/// `nve_get_interface`.
fn build_overlay_provider(dir: &Path) -> PathBuf {
    let (nve_abi_rlib, dep_dirs) = rust_deps::locate_nve_abi_and_dep_dirs();
    let extern_abi = format!("nve_abi={}", nve_abi_rlib.display());
    let rlib = dir.join("liboverlay_view_hybrid.rlib");
    let mut lib = Command::new("rustc");
    lib.args([
        "--edition=2024",
        "--crate-type=lib",
        "--crate-name=overlay_view_hybrid",
        "-D",
        "warnings",
    ]);
    rust_deps::add_dep_search_dirs(&mut lib, dir, &dep_dirs);
    run(
        lib.args(["--extern", &extern_abi])
            .arg(dir.join("overlay_view_hybrid.generated.rs"))
            .arg("-o")
            .arg(&rlib),
        "rustc generated overlay rlib",
    );
    let provider = dir.join("overlay_provider.rs");
    std::fs::write(&provider, OVERLAY_PROVIDER_RS).expect("write provider");
    let staticlib = dir.join("liboverlay_provider.a");
    let mut cmd = Command::new("rustc");
    cmd.args(["--edition=2024", "--crate-type=staticlib", "-D", "warnings"]);
    rust_deps::add_dep_search_dirs(&mut cmd, dir, &dep_dirs);
    run(
        cmd.args(["--extern", &extern_abi])
            .arg("--extern")
            .arg(format!("overlay_view_hybrid={}", rlib.display()))
            .arg(&provider)
            .arg("-o")
            .arg(&staticlib),
        "rustc overlay provider staticlib",
    );
    staticlib
}

/// `<java.home>/include` and its platform directory (`jni_md.h`).
fn jdk_include_dirs() -> Vec<PathBuf> {
    let out = Command::new("java")
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .expect("java");
    let props = String::from_utf8_lossy(&out.stderr);
    let home = props
        .lines()
        .find_map(|l| l.trim().strip_prefix("java.home = "))
        .expect("java.home in -XshowSettings:properties");
    let include = PathBuf::from(home).join("include");
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    vec![include.join(platform), include]
}

#[test]
fn overlay_jni_glue_runs_live_on_a_fake_jni_env_and_a_real_jvm() {
    let _touch = nve_abi::NveValue::null();
    if !common::require_tool("clang++") {
        return;
    }
    let (_, overlay) = specs();
    let dir = temp_test_dir("overlay_jni");
    for target in [HybridTarget::Rust, HybridTarget::Cpp, HybridTarget::Kotlin] {
        generate(&overlay, target, &dir);
    }
    common::write_generated_c_abi_headers(&dir);
    let staticlib = build_overlay_provider(&dir);
    let glue = dir.join("OverlayViewHybridJni.generated.cpp");

    // 1. Fake `JNIEnv` under ASan/UBSan (every borrowed slice must outlive its use): exact prop
    //    tags, zero-copy slices, exceptions, CheckJNI, local refs.
    let fake = dir.join("fake_jni");
    std::fs::create_dir_all(&fake).expect("fake jni dir");
    std::fs::write(fake.join("jni.h"), FAKE_JNI_H).expect("write fake jni.h");
    std::fs::write(dir.join("jni_test_support.h"), JNI_TEST_SUPPORT_H).expect("write support");
    let harness = dir.join("jni_overlay_harness.cpp");
    std::fs::write(&harness, JNI_OVERLAY_HARNESS_CPP).expect("write harness");
    let harness_bin = dir.join("jni_overlay_harness");
    run(
        Command::new("clang++")
            .args([
                "-std=c++20",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-g",
                "-fsanitize=address,undefined",
            ])
            .arg("-fno-sanitize-recover=all")
            .arg(format!("-I{}", fake.display()))
            .arg(format!("-I{}", dir.display()))
            .args([&harness, &glue, &staticlib])
            .arg("-o")
            .arg(&harness_bin),
        "link the OverlayView JNI harness",
    );
    let stdout = run(&mut Command::new(&harness_bin), "OverlayView JNI harness");
    assert!(stdout.contains("jni overlay harness: ok"));

    // 2. A real JVM (`-Xcheck:jni`): the Kotlin client -> JNI glue -> Rust provider.
    if cfg!(windows) || !jvm_tools_available() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let library = dir.join(if cfg!(target_os = "macos") {
        "libnve_overlay_jni.dylib"
    } else {
        "libnve_overlay_jni.so"
    });
    let mut link = Command::new("clang++");
    link.args([
        "-std=c++20",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-shared",
        "-fPIC",
    ]);
    for include in jdk_include_dirs() {
        link.arg(format!("-I{}", include.display()));
    }
    run(
        link.arg(format!("-I{}", dir.display()))
            .args([&glue, &staticlib])
            .arg("-o")
            .arg(&library),
        "link the OverlayView JNI library against the JDK's jni.h",
    );
    let classpath = build_kotlin_app(&dir.join("jvm"));
    // HotSpot prints CheckJNI warnings to stdout unless `DisplayVMOutputToStderr`; OpenJ9 ignores
    // unknown `-XX:` options. Both streams are scanned anyway.
    let out = Command::new("java")
        .args(["-Xcheck:jni", "-XX:+DisplayVMOutputToStderr"])
        .args([
            "-cp",
            &classpath,
            "dev.nve.hybrid.KotlinHybridMainKt",
            "jni",
        ])
        .arg(&library)
        .output()
        .expect("run java");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        out.status.success() && stdout.contains("kotlin hybrid: ok"),
        "real JVM run failed:\n{stdout}\n{stderr}"
    );
    // CheckJNI reports misuse as warnings: HotSpot `WARNING in native method` / `WARNING: JNI
    // local refs`, OpenJ9 `JVMJNCK<nnn>W` / `E` (`JVMJNCK001I` only announces the checker).
    let checkjni_issue = stdout.lines().chain(stderr.lines()).any(|l| {
        l.contains("WARNING in native method")
            || l.contains("WARNING: JNI local refs")
            || l.split_whitespace()
                .any(|w| w.starts_with("JVMJNCK") && (w.ends_with('W') || w.ends_with('E')))
    });
    assert!(!checkjni_issue, "CheckJNI warnings:\n{stdout}\n{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Locates the `nve_abi` rlib and Cargo's dependency directories for `rustc` invocations, across
/// the classic `target/<profile>/deps/` and Cargo 1.100+ per-unit `build/<pkg>/<hash>/out/`
/// layouts (prefers an rlib built with the `wire` feature, then the newest).
mod rust_deps {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::SystemTime;

    fn is_nve_abi_rlib(path: &Path) -> bool {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("libnve_abi-") && n.ends_with(".rlib"))
    }

    fn has_wire_feature(path: &Path) -> bool {
        const NEEDLE: &[u8] = b"encode_bin_value_to_vec";
        std::fs::read(path).is_ok_and(|bytes| bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE))
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|it| it.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    }

    fn best(candidates: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
        let mut ranked: Vec<(bool, SystemTime, PathBuf)> = candidates
            .map(|p| {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                (has_wire_feature(&p), mtime, p)
            })
            .collect();
        ranked.sort_by_key(|(wire, mtime, _)| (*wire, *mtime));
        ranked.pop().map(|(_, _, p)| p)
    }

    pub fn locate_nve_abi_and_dep_dirs() -> (PathBuf, Vec<PathBuf>) {
        let exe = std::env::current_exe().expect("current_exe");
        let exe_dir = exe.parent().expect("test executable dir");
        if let Some(rlib) = best(files(exe_dir).into_iter().filter(|p| is_nve_abi_rlib(p))) {
            return (rlib, vec![exe_dir.to_path_buf()]);
        }
        let build_root = exe
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "build"))
            .expect("Cargo deps/ or build/ ancestor of the test executable");
        let out_dirs: Vec<PathBuf> = files(build_root)
            .into_iter()
            .flat_map(|pkg| files(&pkg))
            .map(|hash| hash.join("out"))
            .filter(|out| out.is_dir())
            .collect();
        let rlib = best(
            out_dirs
                .iter()
                .flat_map(|d| files(d))
                .filter(|p| is_nve_abi_rlib(p)),
        )
        .expect("libnve_abi-*.rlib in the Cargo build dir");
        let dep_dirs = out_dirs
            .into_iter()
            .filter(|d| {
                files(d).iter().any(|p| {
                    p.extension().is_some_and(|e| {
                        matches!(e.to_str(), Some("rlib" | "dll" | "so" | "dylib"))
                    })
                })
            })
            .collect();
        (rlib, dep_dirs)
    }

    /// `-L dependency=` for each dir, through a response file once there are many (Windows'
    /// 32 KiB command-line limit).
    pub fn add_dep_search_dirs(cmd: &mut Command, temp_dir: &Path, dep_dirs: &[PathBuf]) {
        let args: Vec<String> = dep_dirs
            .iter()
            .map(|d| format!("dependency={}", d.display()))
            .collect();
        if args.len() <= 8 {
            for arg in args {
                cmd.arg("-L").arg(arg);
            }
            return;
        }
        let rsp = temp_dir.join("rustc_dep_dirs.rsp");
        let contents: String = args.iter().map(|a| format!("-L\n{a}\n")).collect();
        std::fs::write(&rsp, contents).expect("write rustc response file");
        cmd.arg(format!("@{}", rsp.display()));
    }
}
