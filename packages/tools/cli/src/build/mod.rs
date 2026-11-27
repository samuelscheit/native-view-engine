//! `nve build` subcommand — Apple Tier-3 staticlib & `.xcframework` build recipes.

pub(crate) mod apple_tier3;
pub(crate) mod probe;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use apple_tier3::{
    parse_targets, plan_tier3_build, AppleTier3BuildPlan, Tier3BuildRequest,
    DEFAULT_HEADERS_REL_PATH,
};
use probe::{check_prerequisites, EnvironmentProbe, PrerequisiteIssue, SystemProbe};

pub(crate) struct BuildOptions {
    pub targets: Vec<String>,
    pub package: String,
    pub lib_name: Option<String>,
    pub release: bool,
    pub toolchain: String,
    pub xcframework: Option<PathBuf>,
    pub headers: Option<PathBuf>,
    pub target_dir: Option<PathBuf>,
    pub dry_run: bool,
}

pub(crate) fn find_workspace_root() -> PathBuf {
    if let Ok(mut cur) = env::current_dir() {
        loop {
            let candidate = cur.join("Cargo.toml");
            if candidate.is_file()
                && fs::read_to_string(&candidate)
                    .map(|s| s.contains("[workspace]"))
                    .unwrap_or(false)
            {
                return cur;
            }
            if !cur.pop() {
                break;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn resolve_target_dir(repo_root: &Path, override_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = override_dir {
        return if dir.is_absolute() {
            dir
        } else {
            repo_root.join(dir)
        };
    }
    match env::var("CARGO_TARGET_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let p = PathBuf::from(dir.trim());
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        }
        _ => repo_root.join("target"),
    }
}

fn resolve_headers_dir(repo_root: &Path, override_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = override_dir {
        return if dir.is_absolute() {
            dir
        } else {
            repo_root.join(dir)
        };
    }
    repo_root.join(DEFAULT_HEADERS_REL_PATH)
}

pub(crate) fn format_build_plan(
    plan: &AppleTier3BuildPlan,
    issues: &[PrerequisiteIssue],
) -> String {
    let mut out = String::new();
    let profile = if plan.release { "release" } else { "debug" };
    out.push_str(&format!(
        "Native View Engine — Apple Tier-3 Build Plan ({}, profile={profile}, toolchain={})\n",
        plan.package, plan.toolchain
    ));
    out.push_str(
        "Linkage: staticlib (zero dlopen) -> register nve_get_interface via NveStaticInterfaceTable (NveInterfaceResolver.swift)\n\n",
    );

    for (idx, step) in plan.cargo_steps.iter().enumerate() {
        let spec = step.target.spec();
        out.push_str(&format!(
            "[step {}/{}] {} ({}, sdk={}, min_os={})\n",
            idx + 1,
            plan.cargo_steps.len(),
            spec.cli_name,
            spec.rust_triple,
            spec.sdk_name,
            spec.min_os_version
        ));
        for (k, v) in &step.env {
            out.push_str(&format!("  env  {k}={v}\n"));
        }
        out.push_str(&format!("  cmd  {} {}\n", step.program, step.args.join(" ")));
        out.push_str(&format!("  out  {}\n", step.output_staticlib.display()));
        out.push_str(&format!("  link {}\n\n", step.linkage.swift_registration));
    }

    if let Some(xcf) = &plan.xcframework {
        out.push_str(&format!(
            "[xcframework] {} ({} slice(s), headers={})\n",
            xcf.output_path.display(),
            xcf.slices.len(),
            xcf.headers_dir.display()
        ));
        out.push_str(&format!("  cmd  {} {}\n", xcf.program, xcf.args.join(" ")));
        let present: HashSet<_> = xcf.slices.iter().map(|s| s.target).collect();
        for slice in &xcf.slices {
            if !present.contains(&slice.target.counterpart()) {
                let [dev, sim] = slice.target.spec().family.pair();
                out.push_str(&format!(
                    "  tip  pair device + simulator slices with '--target {},{}'\n",
                    dev.spec().cli_name,
                    sim.spec().cli_name
                ));
            }
        }
        out.push('\n');
    }

    if !issues.is_empty() {
        out.push_str("Prerequisite warnings:\n");
        for issue in issues {
            out.push_str(&format!("  [WARN] {}\n         fix: {}\n", issue.summary, issue.remediation));
        }
    }
    out
}

pub(crate) fn build_request_from_options(
    opts: &BuildOptions,
    repo_root: &Path,
) -> Result<Tier3BuildRequest, apple_tier3::Tier3Error> {
    let targets = parse_targets(&opts.targets)?;
    let lib_name = opts
        .lib_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| opts.package.replace('-', "_"));
    let target_dir = resolve_target_dir(repo_root, opts.target_dir.clone());
    let headers_dir = resolve_headers_dir(repo_root, opts.headers.clone());
    let xcframework_out = opts.xcframework.as_ref().map(|p| {
        if p.as_os_str().is_empty() || p == Path::new("auto") {
            target_dir
                .join("xcframework")
                .join(format!("{lib_name}.xcframework"))
        } else if p.is_absolute() {
            p.clone()
        } else {
            repo_root.join(p)
        }
    });

    Ok(Tier3BuildRequest {
        targets,
        package: opts.package.clone(),
        lib_name,
        release: opts.release,
        toolchain: opts.toolchain.clone(),
        target_dir,
        headers_dir,
        xcframework_out,
    })
}

pub(crate) fn run_build_with_probe(
    opts: BuildOptions,
    repo_root: &Path,
    probe: &dyn EnvironmentProbe,
) -> Result<String, String> {
    let req = build_request_from_options(&opts, repo_root).map_err(|e| e.to_string())?;
    let issues = check_prerequisites(
        &req.targets,
        &req.toolchain,
        req.xcframework_out.is_some(),
        &req.headers_dir,
        probe,
    );
    let plan = plan_tier3_build(&req, probe).map_err(|e| e.to_string())?;

    if opts.dry_run {
        return Ok(format_build_plan(&plan, &issues));
    }

    if !issues.is_empty() {
        return Err(apple_tier3::Tier3Error::MissingPrerequisites(issues).to_string());
    }

    for step in &plan.cargo_steps {
        println!(
            "==> Building '{}' for {} ({})...",
            plan.package,
            step.target.spec().cli_name,
            step.target.spec().rust_triple
        );
        let status = Command::new(&step.program)
            .current_dir(repo_root)
            .args(&step.args)
            .envs(&step.env)
            .status()
            .map_err(|e| format!("failed to spawn {}: {e}", step.program))?;
        if !status.success() {
            return Err(format!(
                "cargo build failed for target '{}' with exit status {status}",
                step.target.spec().cli_name
            ));
        }
    }

    if let Some(xcf) = &plan.xcframework {
        if xcf.output_path.exists() {
            fs::remove_dir_all(&xcf.output_path).map_err(|e| {
                format!(
                    "failed to remove stale xcframework '{}': {e}",
                    xcf.output_path.display()
                )
            })?;
        }
        if let Some(parent) = xcf.output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!("failed to create xcframework parent '{}': {e}", parent.display())
            })?;
        }
        println!("==> Bundling XCFramework '{}'...", xcf.output_path.display());
        let status = Command::new(&xcf.program)
            .current_dir(repo_root)
            .args(&xcf.args)
            .status()
            .map_err(|e| format!("failed to spawn {}: {e}", xcf.program))?;
        if !status.success() {
            return Err(format!(
                "xcodebuild -create-xcframework failed with exit status {status}"
            ));
        }
    }

    Ok(format_build_plan(&plan, &[]))
}

pub(crate) fn cmd_build(opts: BuildOptions) {
    let repo_root = find_workspace_root();
    match run_build_with_probe(opts, &repo_root, &SystemProbe) {
        Ok(report) => print!("{report}"),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}
