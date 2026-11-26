//! First-class Apple Tier-3 build recipes (`tvos`, `tvos-sim`, `visionos`, `visionos-sim`,
//! `watchos`, `watchos-sim`) using `-Z build-std=std,panic_abort`, staticlib archives,
//! `NveInterfaceResolver` static C linkage (zero `dlopen`), and `xcodebuild -create-xcframework`.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::PathBuf;

use super::probe::{EnvironmentProbe, PrerequisiteIssue};

pub const NVE_ABI_HEADER_FILE: &str = "nve_abi.generated.h";
pub const NVE_INTERFACE_SYMBOL: &str = "nve_get_interface";
pub const DEFAULT_HEADERS_REL_PATH: &str = "hosts/swiftui-kit/Sources/CNveAbi/include";

/// Apple Tier-3 platform family (tvOS, visionOS, watchOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplePlatformFamily {
    Tvos,
    VisionOs,
    WatchOs,
}

impl ApplePlatformFamily {
    /// Device + simulator target pair for building a fat `.xcframework` per Apple platform.
    pub const fn pair(self) -> [AppleTier3Target; 2] {
        match self {
            Self::Tvos => [AppleTier3Target::Tvos, AppleTier3Target::TvosSim],
            Self::VisionOs => [AppleTier3Target::VisionOs, AppleTier3Target::VisionOsSim],
            Self::WatchOs => [AppleTier3Target::WatchOs, AppleTier3Target::WatchOsSim],
        }
    }

    pub const fn xcode_download_name(self) -> &'static str {
        match self {
            Self::Tvos => "tvOS",
            Self::VisionOs => "visionOS",
            Self::WatchOs => "watchOS",
        }
    }
}

/// Supported Apple Tier-3 Rust targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppleTier3Target {
    Tvos,
    TvosSim,
    VisionOs,
    VisionOsSim,
    WatchOs,
    WatchOsSim,
}

/// Static specification of an Apple Tier-3 target matching `hosts/swiftui-kit/Package.swift`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppleTargetSpec {
    pub cli_name: &'static str,
    pub rust_triple: &'static str,
    pub sdk_name: &'static str,
    pub min_os_version: &'static str,
    pub deployment_target_env: &'static str,
    pub family: ApplePlatformFamily,
    pub is_simulator: bool,
}

impl AppleTargetSpec {
    const fn new(
        cli_name: &'static str,
        rust_triple: &'static str,
        sdk_name: &'static str,
        min_os_version: &'static str,
        deployment_target_env: &'static str,
        family: ApplePlatformFamily,
        is_simulator: bool,
    ) -> Self {
        Self {
            cli_name,
            rust_triple,
            sdk_name,
            min_os_version,
            deployment_target_env,
            family,
            is_simulator,
        }
    }
}

/// Staticlib linkage contract for `hosts/swiftui-kit/Sources/NveSwiftUI/NveInterfaceResolver.swift`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticLinkageSpec {
    pub crate_type: &'static str,
    pub panic_strategy: &'static str,
    pub interface_symbol: &'static str,
    pub dlopen_allowed: bool,
    pub swift_registration: &'static str,
}

impl StaticLinkageSpec {
    pub const fn apple_tier3_static() -> Self {
        Self {
            crate_type: "staticlib",
            panic_strategy: "abort",
            interface_symbol: NVE_INTERFACE_SYMBOL,
            dlopen_allowed: false,
            swift_registration: "NveStaticInterfaceTable.shared.register(symbol: \"nve_get_interface\", cFunction: nve_get_interface)",
        }
    }
}

impl AppleTier3Target {
    pub const ALL: [Self; 6] = [
        Self::Tvos,
        Self::TvosSim,
        Self::VisionOs,
        Self::VisionOsSim,
        Self::WatchOs,
        Self::WatchOsSim,
    ];

    pub const fn spec(self) -> AppleTargetSpec {
        use ApplePlatformFamily as F;
        match self {
            Self::Tvos => AppleTargetSpec::new("tvos", "aarch64-apple-tvos", "appletvos", "16.0", "TVOS_DEPLOYMENT_TARGET", F::Tvos, false),
            Self::TvosSim => AppleTargetSpec::new("tvos-sim", "aarch64-apple-tvos-sim", "appletvsimulator", "16.0", "TVOS_DEPLOYMENT_TARGET", F::Tvos, true),
            Self::VisionOs => AppleTargetSpec::new("visionos", "aarch64-apple-visionos", "xros", "1.0", "XROS_DEPLOYMENT_TARGET", F::VisionOs, false),
            Self::VisionOsSim => AppleTargetSpec::new("visionos-sim", "aarch64-apple-visionos-sim", "xrsimulator", "1.0", "XROS_DEPLOYMENT_TARGET", F::VisionOs, true),
            Self::WatchOs => AppleTargetSpec::new("watchos", "aarch64-apple-watchos", "watchos", "9.0", "WATCHOS_DEPLOYMENT_TARGET", F::WatchOs, false),
            Self::WatchOsSim => AppleTargetSpec::new("watchos-sim", "aarch64-apple-watchos-sim", "watchsimulator", "9.0", "WATCHOS_DEPLOYMENT_TARGET", F::WatchOs, true),
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Tier3Error> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("tvos") || s.eq_ignore_ascii_case("aarch64-apple-tvos") {
            Ok(Self::Tvos)
        } else if s.eq_ignore_ascii_case("tvos-sim")
            || s.eq_ignore_ascii_case("aarch64-apple-tvos-sim")
        {
            Ok(Self::TvosSim)
        } else if s.eq_ignore_ascii_case("visionos")
            || s.eq_ignore_ascii_case("aarch64-apple-visionos")
        {
            Ok(Self::VisionOs)
        } else if s.eq_ignore_ascii_case("visionos-sim")
            || s.eq_ignore_ascii_case("aarch64-apple-visionos-sim")
        {
            Ok(Self::VisionOsSim)
        } else if s.eq_ignore_ascii_case("watchos")
            || s.eq_ignore_ascii_case("aarch64-apple-watchos")
        {
            Ok(Self::WatchOs)
        } else if s.eq_ignore_ascii_case("watchos-sim")
            || s.eq_ignore_ascii_case("aarch64-apple-watchos-sim")
        {
            Ok(Self::WatchOsSim)
        } else {
            Err(Tier3Error::InvalidTarget(raw.to_owned()))
        }
    }

    pub const fn counterpart(self) -> Self {
        match self {
            Self::Tvos => Self::TvosSim,
            Self::TvosSim => Self::Tvos,
            Self::VisionOs => Self::VisionOsSim,
            Self::VisionOsSim => Self::VisionOs,
            Self::WatchOs => Self::WatchOsSim,
            Self::WatchOsSim => Self::WatchOs,
        }
    }

    pub const fn valid_names_csv() -> &'static str {
        "tvos, tvos-sim, visionos, visionos-sim, watchos, watchos-sim"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier3Error {
    EmptyTargets,
    InvalidTarget(String),
    DuplicateTarget(AppleTier3Target),
    InvalidXcframeworkOutput(PathBuf),
    MissingPrerequisites(Vec<PrerequisiteIssue>),
}

impl fmt::Display for Tier3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTargets => write!(
                f,
                "at least one target is required (valid: {})",
                AppleTier3Target::valid_names_csv()
            ),
            Self::InvalidTarget(raw) => write!(
                f,
                "unsupported Apple Tier-3 target '{raw}'; expected one of: {}",
                AppleTier3Target::valid_names_csv()
            ),
            Self::DuplicateTarget(t) => write!(
                f,
                "duplicate Apple Tier-3 target '{}' ({})",
                t.spec().cli_name,
                t.spec().rust_triple
            ),
            Self::InvalidXcframeworkOutput(path) => write!(
                f,
                "xcframework output path '{}' must end with '.xcframework'",
                path.display()
            ),
            Self::MissingPrerequisites(issues) => {
                writeln!(f, "Apple Tier-3 build prerequisites are missing:")?;
                for issue in issues {
                    writeln!(f, "  - {}\n    fix: {}", issue.summary, issue.remediation)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Tier3Error {}

#[derive(Debug, Clone)]
pub struct Tier3BuildRequest {
    pub targets: Vec<AppleTier3Target>,
    pub package: String,
    pub lib_name: String,
    pub release: bool,
    pub toolchain: String,
    pub target_dir: PathBuf,
    pub headers_dir: PathBuf,
    pub xcframework_out: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoInvocation {
    pub target: AppleTier3Target,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub output_staticlib: PathBuf,
    pub linkage: StaticLinkageSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XcframeworkSlice {
    pub target: AppleTier3Target,
    pub staticlib_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XcframeworkInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub slices: Vec<XcframeworkSlice>,
    pub headers_dir: PathBuf,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppleTier3BuildPlan {
    pub package: String,
    pub lib_name: String,
    pub release: bool,
    pub toolchain: String,
    pub cargo_steps: Vec<CargoInvocation>,
    pub xcframework: Option<XcframeworkInvocation>,
}

/// Parses and deduplicates a list of CLI target strings, rejecting unknown or duplicate entries.
pub fn parse_targets(raw: &[String]) -> Result<Vec<AppleTier3Target>, Tier3Error> {
    if raw.is_empty() {
        return Err(Tier3Error::EmptyTargets);
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for item in raw {
        let target = AppleTier3Target::parse(item)?;
        if !seen.insert(target) {
            return Err(Tier3Error::DuplicateTarget(target));
        }
        out.push(target);
    }
    Ok(out)
}

/// Plans `xcodebuild -create-xcframework` for a unique set of `.a` slices + `nve_abi.generated.h`.
pub fn plan_xcframework(
    slices: Vec<XcframeworkSlice>,
    headers_dir: PathBuf,
    output_path: PathBuf,
) -> Result<XcframeworkInvocation, Tier3Error> {
    if slices.is_empty() {
        return Err(Tier3Error::EmptyTargets);
    }
    if output_path.extension().and_then(|e| e.to_str()) != Some("xcframework") {
        return Err(Tier3Error::InvalidXcframeworkOutput(output_path));
    }
    let mut seen = HashSet::new();
    let mut args = vec!["-create-xcframework".to_owned()];
    for slice in &slices {
        if !seen.insert(slice.target) {
            return Err(Tier3Error::DuplicateTarget(slice.target));
        }
        args.extend([
            "-library".into(),
            slice.staticlib_path.to_string_lossy().into_owned(),
            "-headers".into(),
            headers_dir.to_string_lossy().into_owned(),
        ]);
    }
    args.extend(["-output".into(), output_path.to_string_lossy().into_owned()]);
    Ok(XcframeworkInvocation {
        program: "xcodebuild".into(),
        args,
        slices,
        headers_dir,
        output_path,
    })
}

/// Builds the complete Cargo + optional `xcodebuild -create-xcframework` plan.
pub fn plan_tier3_build(
    req: &Tier3BuildRequest,
    probe: &dyn EnvironmentProbe,
) -> Result<AppleTier3BuildPlan, Tier3Error> {
    if req.targets.is_empty() {
        return Err(Tier3Error::EmptyTargets);
    }
    let mut seen = HashSet::new();
    let (profile_dir, profile_cfg) = if req.release { ("release", "release") } else { ("debug", "dev") };
    let mut cargo_steps = Vec::with_capacity(req.targets.len());
    let mut slices = Vec::with_capacity(req.targets.len());

    for &target in &req.targets {
        if !seen.insert(target) {
            return Err(Tier3Error::DuplicateTarget(target));
        }
        let spec = target.spec();
        let sdk_root = probe
            .sdk_path(spec.sdk_name)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("<SDKROOT:{}>", spec.sdk_name));

        let mut args = Vec::new();
        if !req.toolchain.trim().is_empty() {
            args.push(format!("+{}", req.toolchain.trim()));
        }
        args.extend([
            "rustc".into(), "-Z".into(), "build-std=std,panic_abort".into(),
            "--target".into(), spec.rust_triple.into(),
            "-p".into(), req.package.clone(),
            "--lib".into(), "--crate-type".into(), "staticlib".into(),
            "--config".into(), format!("profile.{profile_cfg}.panic=\"abort\""),
        ]);
        if req.release {
            args.push("--release".into());
        }

        let mut env = BTreeMap::new();
        env.insert("CARGO_TARGET_DIR".into(), req.target_dir.to_string_lossy().into_owned());
        env.insert("SDKROOT".into(), sdk_root);
        env.insert(spec.deployment_target_env.into(), spec.min_os_version.into());

        let output_staticlib = req
            .target_dir
            .join(spec.rust_triple)
            .join(profile_dir)
            .join(format!("lib{}.a", req.lib_name));
        slices.push(XcframeworkSlice { target, staticlib_path: output_staticlib.clone() });
        cargo_steps.push(CargoInvocation {
            target,
            program: "cargo".into(),
            args,
            env,
            output_staticlib,
            linkage: StaticLinkageSpec::apple_tier3_static(),
        });
    }

    let xcframework = match &req.xcframework_out {
        Some(out) => Some(plan_xcframework(slices, req.headers_dir.clone(), out.clone())?),
        None => None,
    };
    Ok(AppleTier3BuildPlan {
        package: req.package.clone(),
        lib_name: req.lib_name.clone(),
        release: req.release,
        toolchain: req.toolchain.clone(),
        cargo_steps,
        xcframework,
    })
}
