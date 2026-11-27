//! Host environment probes (`xcrun`, `rustup`, `xcodebuild`, ABI headers),
//! prerequisite checks, and `nve doctor` reporting for Apple Tier-3 SDKs.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::apple_tier3::{AppleTier3Target, NVE_ABI_HEADER_FILE, NVE_MODULE_MAP_FILE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingPrerequisite {
    Toolchain,
    RustSrc,
    Sdk(AppleTier3Target),
    Xcodebuild,
    AbiHeader,
    ModuleMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrerequisiteIssue {
    pub kind: MissingPrerequisite,
    pub summary: String,
    pub remediation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkProbeStatus {
    pub target: AppleTier3Target,
    pub sdk_path: Option<PathBuf>,
}

/// Environment probe abstraction for `xcrun`, `rustup`, `xcodebuild`, and header checks.
pub trait EnvironmentProbe {
    fn sdk_path(&self, sdk_name: &str) -> Option<PathBuf>;
    fn has_toolchain(&self, toolchain: &str) -> bool;
    fn has_rust_src(&self, toolchain: &str) -> bool;
    fn has_xcodebuild(&self) -> bool;
    fn file_exists(&self, path: &Path) -> bool;
}

/// Live host probe backed by `xcrun`, `rustup`, `xcodebuild`, and `std::path::Path`.
pub struct SystemProbe;

impl EnvironmentProbe for SystemProbe {
    fn sdk_path(&self, sdk_name: &str) -> Option<PathBuf> {
        let out = Command::new("xcrun")
            .args(["--sdk", sdk_name, "--show-sdk-path"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    fn has_toolchain(&self, toolchain: &str) -> bool {
        let Ok(out) = Command::new("rustup").args(["toolchain", "list"]).output() else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        let needle = toolchain.trim();
        String::from_utf8_lossy(&out.stdout).lines().any(|line| {
            let entry = line.split_whitespace().next().unwrap_or("");
            entry == needle || entry.starts_with(&format!("{needle}-"))
        })
    }

    fn has_rust_src(&self, toolchain: &str) -> bool {
        let Ok(out) = Command::new("rustup")
            .args(["component", "list", "--toolchain", toolchain, "--installed"])
            .output()
        else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.trim() == "rust-src")
    }

    fn has_xcodebuild(&self) -> bool {
        Command::new("xcodebuild")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.is_file()
    }
}

/// Validates all host prerequisites required to execute a Tier-3 build plan.
pub fn check_prerequisites(
    targets: &[AppleTier3Target],
    toolchain: &str,
    needs_xcframework: bool,
    headers_dir: &Path,
    probe: &dyn EnvironmentProbe,
) -> Vec<PrerequisiteIssue> {
    let mut issues = Vec::new();
    if !probe.has_toolchain(toolchain) {
        issues.push(PrerequisiteIssue {
            kind: MissingPrerequisite::Toolchain,
            summary: format!("Rust toolchain '{toolchain}' is not installed (required for Tier-3 -Z build-std=std,panic_abort)"),
            remediation: format!("rustup toolchain install {toolchain}"),
        });
    } else if !probe.has_rust_src(toolchain) {
        issues.push(PrerequisiteIssue {
            kind: MissingPrerequisite::RustSrc,
            summary: format!("rustup component 'rust-src' is missing on toolchain '{toolchain}' (required for -Z build-std=std,panic_abort)"),
            remediation: format!("rustup component add rust-src --toolchain {toolchain}"),
        });
    }
    for &target in targets {
        let spec = target.spec();
        if probe.sdk_path(spec.sdk_name).is_none() {
            let platform = spec.family.xcode_download_name();
            issues.push(PrerequisiteIssue {
                kind: MissingPrerequisite::Sdk(target),
                summary: format!(
                    "Apple SDK '{}' for target '{}' ({}) was not found via xcrun",
                    spec.sdk_name, spec.cli_name, spec.rust_triple
                ),
                remediation: format!(
                    "On macOS with Xcode installed, run: xcodebuild -downloadPlatform {platform}"
                ),
            });
        }
    }
    if needs_xcframework {
        if !probe.has_xcodebuild() {
            issues.push(PrerequisiteIssue {
                kind: MissingPrerequisite::Xcodebuild,
                summary: "xcodebuild was not found (required for -create-xcframework)".into(),
                remediation: "Install Xcode and run: sudo xcode-select -s /Applications/Xcode.app/Contents/Developer".into(),
            });
        }
        let header_path = headers_dir.join(NVE_ABI_HEADER_FILE);
        if !probe.file_exists(&header_path) {
            issues.push(PrerequisiteIssue {
                kind: MissingPrerequisite::AbiHeader,
                summary: format!("generated C ABI header not found at '{}'", header_path.display()),
                remediation: "Run: cargo run --locked -p nve-cli -- codegen abi".into(),
            });
        }
        // Without a module map the `.xcframework` cannot be imported from Swift (`import CNveAbi`).
        let module_map = headers_dir.join(NVE_MODULE_MAP_FILE);
        if !probe.file_exists(&module_map) {
            issues.push(PrerequisiteIssue {
                kind: MissingPrerequisite::ModuleMap,
                summary: format!("Clang module map not found at '{}'", module_map.display()),
                remediation: "Pass --headers pointing at a directory with a `module.modulemap` declaring `CNveAbi` (packages/core/host-cxx/include has one)".into(),
            });
        }
    }
    issues
}

/// Probes all 6 Apple Tier-3 Xcode SDKs (reused by `nve doctor`).
pub fn probe_all_tier3_sdks(probe: &dyn EnvironmentProbe) -> Vec<SdkProbeStatus> {
    AppleTier3Target::ALL
        .iter()
        .map(|&target| SdkProbeStatus {
            target,
            sdk_path: probe.sdk_path(target.spec().sdk_name),
        })
        .collect()
}

/// Formats the Apple Tier-3 SDK & nightly `rust-src` readiness section for `nve doctor`.
pub fn format_tier3_doctor_section(toolchain: &str, probe: &dyn EnvironmentProbe) -> String {
    let mut out = String::from("\n  Apple Tier-3 SDKs (tvOS / visionOS / watchOS):\n");
    for status in probe_all_tier3_sdks(probe) {
        let spec = status.target.spec();
        match status.sdk_path {
            Some(path) => out.push_str(&format!(
                "  [OK]   {:<14} ({:<16} min {}) {}\n",
                spec.cli_name,
                spec.sdk_name,
                spec.min_os_version,
                path.display()
            )),
            None => out.push_str(&format!(
                "  [MISS] {:<14} ({:<16} min {}) xcrun --sdk {} not found\n",
                spec.cli_name, spec.sdk_name, spec.min_os_version, spec.sdk_name
            )),
        }
    }
    let has_tc = probe.has_toolchain(toolchain);
    let has_src = has_tc && probe.has_rust_src(toolchain);
    if has_src {
        out.push_str(&format!(
            "  [OK]   tier3-std      {toolchain} + rust-src (-Z build-std=std,panic_abort ready)\n"
        ));
    } else if has_tc {
        out.push_str(&format!(
            "  [MISS] tier3-std      rust-src missing (fix: rustup component add rust-src --toolchain {toolchain})\n"
        ));
    } else {
        out.push_str(&format!(
            "  [MISS] tier3-std      {toolchain} toolchain missing (fix: rustup toolchain install {toolchain})\n"
        ));
    }
    out
}
