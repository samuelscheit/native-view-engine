//! Offline Dart packages for generated `dart:ffi` bindings: `package:nve_host` from this checkout,
//! strict analysis options, `dart analyze --fatal-infos`, and running a driver on the Dart VM.

use std::path::Path;
use std::process::Command;

/// Strict analyzer settings every generated Dart file must pass.
pub const STRICT_ANALYSIS_OPTIONS: &str = r#"
analyzer:
  language:
    strict-casts: true
    strict-inference: true
    strict-raw-types: true

linter:
  rules:
    - always_declare_return_types
    - annotate_overrides
    - avoid_dynamic_calls
    - avoid_init_to_null
    - curly_braces_in_flow_control_structures
    - no_leading_underscores_for_local_identifiers
    - non_constant_identifier_names
    - prefer_const_constructors
    - prefer_final_fields
    - prefer_final_locals
    - type_annotate_public_apis
    - unnecessary_this
"#;

fn to_file_uri(p: &Path) -> String {
    let canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = canonical.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

/// Makes `dir` the offline package `pkg_name` depending on `package:nve_host` (copied into
/// `dir/nve_host_pkg`), with its `.dart_tool/package_config.json` and strict analysis options.
pub fn setup_offline_dart_pkg(dir: &Path, pkg_name: &str) {
    let host_root = super::prepare_offline_nve_host_pkg(dir);
    std::fs::write(dir.join("analysis_options.yaml"), STRICT_ANALYSIS_OPTIONS).expect("write analysis_options.yaml");
    let dart_tool = dir.join(".dart_tool");
    std::fs::create_dir_all(&dart_tool).expect("create .dart_tool");
    std::fs::write(
        dir.join("pubspec.yaml"),
        format!(
            "name: {pkg_name}\nenvironment:\n  sdk: '>=3.4.0 <4.0.0'\ndependencies:\n  nve_host:\n    path: nve_host_pkg\n"
        ),
    )
    .expect("write pubspec.yaml");
    let config = format!(
        r#"{{
  "configVersion": 2,
  "packages": [
    {{
      "name": "{pkg_name}",
      "rootUri": "{}",
      "packageUri": "lib/",
      "languageVersion": "3.4"
    }},
    {{
      "name": "nve_host",
      "rootUri": "{}",
      "packageUri": "lib/",
      "languageVersion": "3.4"
    }}
  ]
}}
"#,
        to_file_uri(dir),
        to_file_uri(&host_root),
    );
    std::fs::write(dart_tool.join("package_config.json"), config)
        .expect("write package_config.json");
}

/// `dart analyze --fatal-infos dir`, failing the test with the analyzer's report.
pub fn analyze(dir: &Path) {
    let out = Command::new("dart").args(["analyze", "--fatal-infos"]).arg(dir).output().expect("run dart analyze");
    assert!(
        out.status.success(),
        "`dart analyze --fatal-infos` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs `driver` with asserts on in package `dir`, returning its stdout.
pub fn run(dir: &Path, driver: &Path, args: &[&Path]) -> String {
    let out = Command::new("dart")
        .arg("--enable-asserts")
        .arg(format!("--packages={}", dir.join(".dart_tool/package_config.json").display()))
        .arg(driver)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run the Dart VM");
    assert!(
        out.status.success(),
        "Dart driver {} failed:\nstdout:\n{}\nstderr:\n{}",
        driver.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
