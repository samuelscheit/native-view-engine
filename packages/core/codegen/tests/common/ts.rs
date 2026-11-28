//! Offline `@nve/host` TypeScript package for generated bindings, strict `tsc --noEmit`, and Node.

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::generate_all_abi_files;

/// The strict `tsconfig.json` compiler options of `host-ts`, over every `.ts` file in the directory.
pub const STRICT_TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2024", "DOM"],
    "types": [],
    "strict": true,
    "noEmit": true,
    "erasableSyntaxOnly": true,
    "verbatimModuleSyntax": true,
    "allowImportingTsExtensions": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitOverride": true,
    "noFallthroughCasesInSwitch": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "isolatedModules": true,
    "skipLibCheck": false
  },
  "include": ["./*.ts"]
}"#;

/// Materializes a self-contained offline `@nve/host` package under `dir/host_ts_pkg` (for Node's
/// native TypeScript type-stripping, which refuses `.ts` paths under `node_modules/`) AND under
/// `dir/node_modules/@nve/host` (for `tsc` `NodeNext` module resolution), plus a tiny ESM resolve
/// hook (`dir/resolve_host.mjs`) mapping `'@nve/host'` to `dir/host_ts_pkg/src/index.ts`.
pub fn prepare_offline_host_ts_pkg(dir: &Path) -> PathBuf {
    let src = super::workspace_root().join("packages/core/host-ts");
    let abi_files = generate_all_abi_files(nve_tables::tables::CORE_TABLE_DESCS)
        .expect("generate host ABI files");

    for dst in [dir.join("node_modules/@nve/host"), dir.join("host_ts_pkg")] {
        super::copy_tree(&src.join("src"), &dst.join("src"));
        std::fs::copy(src.join("package.json"), dst.join("package.json"))
            .expect("copy host-ts package.json");
        for file in &abi_files {
            if let Some(rel) = file.relative_path.strip_prefix("packages/core/host-ts/") {
                let out = dst.join(rel);
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).expect("create host-ts generated dir");
                }
                std::fs::write(out, &file.content).expect("write host-ts generated file");
            }
        }
    }

    let hook = dir.join("resolve_host.mjs");
    std::fs::write(
        &hook,
        r#"import { registerHooks } from 'node:module';
const target = new URL('./host_ts_pkg/src/index.ts', import.meta.url).href;
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === '@nve/host') return { url: target, shortCircuit: true };
    return nextResolve(specifier, context);
  },
});
"#,
    )
    .expect("write resolve_host.mjs");
    hook
}

/// `tsc --noEmit -p tsconfig` (or `npx -p typescript@5.9 tsc`), failing the test with its report.
pub fn run_tsc_no_emit(tsconfig: &Path) {
    let mut cmd = if super::tool_available("tsc") {
        let mut c = Command::new("tsc");
        c.args(["--noEmit", "-p"]).arg(tsconfig);
        c
    } else if super::tool_available("npx") {
        let mut c = Command::new("npx");
        c.args(["-y", "-p", "typescript@5.9", "tsc", "--noEmit", "-p"])
            .arg(tsconfig);
        c
    } else {
        assert!(
            std::env::var_os("NVE_REQUIRE_TSC").is_none() && std::env::var_os("CI").is_none(),
            "`tsc` (or `npx`) is required (NVE_REQUIRE_TSC / CI) but neither was found"
        );
        eprintln!("SKIP: neither `tsc` nor `npx` found");
        return;
    };
    let out = cmd.output().expect("run tsc --noEmit");
    assert!(
        out.status.success(),
        "tsc --noEmit failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Runs the TypeScript `driver` on Node with `@nve/host` resolved through `hook`, returning stdout.
pub fn run_node(hook: &Path, driver: &Path) -> String {
    let out = Command::new("node").arg("--import").arg(hook).arg(driver).output().expect("run node");
    assert!(
        out.status.success(),
        "node {} failed:\nstdout:\n{}\nstderr:\n{}",
        driver.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
