//! Unit and CLI parsing tests for Apple Tier-3 build recipes (`tvos`, `visionos`, `watchos`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use clap::Parser;

use super::apple_tier3::{
    parse_targets, plan_tier3_build, plan_xcframework, ApplePlatformFamily, AppleTier3Target,
    Tier3BuildRequest, Tier3Error, XcframeworkSlice, NVE_ABI_HEADER_FILE, NVE_INTERFACE_SYMBOL, NVE_MODULE_MAP_FILE,
};
use super::probe::{
    check_prerequisites, format_tier3_doctor_section, probe_all_tier3_sdks, EnvironmentProbe,
    MissingPrerequisite,
};
use super::{build_request_from_options, format_build_plan, run_build_with_probe, BuildOptions};
use crate::{Cli, Commands};

#[derive(Default)]
struct FakeProbe {
    sdks: HashMap<String, PathBuf>,
    toolchains: HashSet<String>,
    rust_src_toolchains: HashSet<String>,
    xcodebuild: bool,
    files: HashSet<PathBuf>,
}

impl FakeProbe {
    fn all_ready(headers_dir: &Path) -> Self {
        let mut p = Self::default();
        for target in AppleTier3Target::ALL {
            let sdk = target.spec().sdk_name;
            p.sdks.insert(
                sdk.into(),
                PathBuf::from(format!("/Applications/Xcode.app/SDKs/{sdk}.sdk")),
            );
        }
        p.toolchains.insert("nightly".into());
        p.rust_src_toolchains.insert("nightly".into());
        p.xcodebuild = true;
        p.files.insert(headers_dir.join(NVE_ABI_HEADER_FILE));
        p.files.insert(headers_dir.join(NVE_MODULE_MAP_FILE));
        p
    }
}

impl EnvironmentProbe for FakeProbe {
    fn sdk_path(&self, sdk_name: &str) -> Option<PathBuf> {
        self.sdks.get(sdk_name).cloned()
    }

    fn has_toolchain(&self, toolchain: &str) -> bool {
        self.toolchains.contains(toolchain)
    }

    fn has_rust_src(&self, toolchain: &str) -> bool {
        self.rust_src_toolchains.contains(toolchain)
    }

    fn has_xcodebuild(&self) -> bool {
        self.xcodebuild
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.files.contains(path)
    }
}

#[test]
fn all_six_tier3_targets_match_swiftui_kit_specs_and_cargo_invocations() {
    let headers = PathBuf::from("/repo/packages/core/host-cxx/include");
    let target_dir = PathBuf::from("/repo/target");
    let probe = FakeProbe::all_ready(&headers);

    let expected = [
        (
            AppleTier3Target::Tvos,
            "tvos",
            "aarch64-apple-tvos",
            "appletvos",
            "16.0",
            "TVOS_DEPLOYMENT_TARGET",
            false,
        ),
        (
            AppleTier3Target::TvosSim,
            "tvos-sim",
            "aarch64-apple-tvos-sim",
            "appletvsimulator",
            "16.0",
            "TVOS_DEPLOYMENT_TARGET",
            true,
        ),
        (
            AppleTier3Target::VisionOs,
            "visionos",
            "aarch64-apple-visionos",
            "xros",
            "1.0",
            "XROS_DEPLOYMENT_TARGET",
            false,
        ),
        (
            AppleTier3Target::VisionOsSim,
            "visionos-sim",
            "aarch64-apple-visionos-sim",
            "xrsimulator",
            "1.0",
            "XROS_DEPLOYMENT_TARGET",
            true,
        ),
        (
            AppleTier3Target::WatchOs,
            "watchos",
            "aarch64-apple-watchos",
            "watchos",
            "9.0",
            "WATCHOS_DEPLOYMENT_TARGET",
            false,
        ),
        (
            AppleTier3Target::WatchOsSim,
            "watchos-sim",
            "aarch64-apple-watchos-sim",
            "watchsimulator",
            "9.0",
            "WATCHOS_DEPLOYMENT_TARGET",
            true,
        ),
    ];

    for (target, cli_name, triple, sdk, min_os, env_key, is_sim) in expected {
        let spec = target.spec();
        assert_eq!(spec.cli_name, cli_name);
        assert_eq!(spec.rust_triple, triple);
        assert_eq!(spec.sdk_name, sdk);
        assert_eq!(spec.min_os_version, min_os);
        assert_eq!(spec.deployment_target_env, env_key);
        assert_eq!(spec.is_simulator, is_sim);
        assert_eq!(AppleTier3Target::parse(cli_name).unwrap(), target);
        assert_eq!(AppleTier3Target::parse(triple).unwrap(), target);

        let req = Tier3BuildRequest {
            targets: vec![target],
            package: "nve-studio-workspace".into(),
            lib_name: "nve_studio_workspace".into(),
            release: true,
            toolchain: "nightly".into(),
            target_dir: target_dir.clone(),
            headers_dir: headers.clone(),
            xcframework_out: None,
        };
        let plan = plan_tier3_build(&req, &probe).expect("valid plan");
        assert_eq!(plan.cargo_steps.len(), 1);
        let step = &plan.cargo_steps[0];
        assert_eq!(step.program, "cargo");
        assert_eq!(
            step.args,
            vec![
                "+nightly",
                "rustc",
                "-Z",
                "build-std=std,panic_abort",
                "--target",
                triple,
                "-p",
                "nve-studio-workspace",
                "--lib",
                "--crate-type",
                "staticlib",
                "--config",
                "profile.release.panic=\"abort\"",
                "--release",
            ]
        );
        assert_eq!(
            step.env.get("SDKROOT").map(String::as_str),
            Some(format!("/Applications/Xcode.app/SDKs/{sdk}.sdk").as_str())
        );
        assert_eq!(step.env.get(env_key).map(String::as_str), Some(min_os));
        assert_eq!(
            step.env.get("CARGO_TARGET_DIR").map(String::as_str),
            Some("/repo/target")
        );
        assert_eq!(
            step.output_staticlib,
            PathBuf::from(format!("/repo/target/{triple}/release/libnve_studio_workspace.a"))
        );
        assert_eq!(step.linkage.crate_type, "staticlib");
        assert_eq!(step.linkage.panic_strategy, "abort");
        assert_eq!(step.linkage.interface_symbol, NVE_INTERFACE_SYMBOL);
        assert!(!step.linkage.dlopen_allowed);
        assert!(step.linkage.swift_registration.contains("NveStaticInterfaceTable"));
    }
}

#[test]
fn debug_profile_uses_dev_panic_abort_and_omits_release_flag() {
    let headers = PathBuf::from("/repo/packages/core/host-cxx/include");
    let probe = FakeProbe::all_ready(&headers);
    let req = Tier3BuildRequest {
        targets: vec![AppleTier3Target::WatchOsSim],
        package: "nve-example-kitchen-sink".into(),
        lib_name: "nve_example_kitchen_sink".into(),
        release: false,
        toolchain: "nightly-2026-10-20".into(),
        target_dir: PathBuf::from("/repo/target"),
        headers_dir: headers,
        xcframework_out: None,
    };
    let plan = plan_tier3_build(&req, &probe).unwrap();
    let step = &plan.cargo_steps[0];
    assert!(step.args.contains(&"+nightly-2026-10-20".to_owned()));
    assert!(step.args.contains(&"profile.dev.panic=\"abort\"".to_owned()));
    assert!(!step.args.contains(&"--release".to_owned()));
    assert_eq!(
        step.output_staticlib,
        PathBuf::from("/repo/target/aarch64-apple-watchos-sim/debug/libnve_example_kitchen_sink.a")
    );
}

#[test]
fn xcframework_pairs_device_and_simulator_slices_with_generated_headers() {
    let headers = PathBuf::from("/repo/packages/core/host-cxx/include");
    let probe = FakeProbe::all_ready(&headers);

    for family in [
        ApplePlatformFamily::Tvos,
        ApplePlatformFamily::VisionOs,
        ApplePlatformFamily::WatchOs,
    ] {
        let [device, sim] = family.pair();
        assert_eq!(device.counterpart(), sim);
        assert_eq!(sim.counterpart(), device);

        let req = Tier3BuildRequest {
            targets: vec![device, sim],
            package: "nve-studio-workspace".into(),
            lib_name: "nve_studio_workspace".into(),
            release: true,
            toolchain: "nightly".into(),
            target_dir: PathBuf::from("/repo/target"),
            headers_dir: headers.clone(),
            xcframework_out: Some(PathBuf::from("/repo/dist/NveStudio.xcframework")),
        };
        let plan = plan_tier3_build(&req, &probe).unwrap();
        let xcf = plan.xcframework.expect("xcframework invocation");
        assert_eq!(xcf.program, "xcodebuild");
        assert_eq!(xcf.slices.len(), 2);
        let expected_device_lib = req
            .target_dir
            .join(device.spec().rust_triple)
            .join("release")
            .join("libnve_studio_workspace.a")
            .display()
            .to_string();
        let expected_sim_lib = req
            .target_dir
            .join(sim.spec().rust_triple)
            .join("release")
            .join("libnve_studio_workspace.a")
            .display()
            .to_string();
        assert_eq!(
            xcf.args,
            vec![
                "-create-xcframework",
                "-library",
                expected_device_lib.as_str(),
                "-headers",
                "/repo/packages/core/host-cxx/include",
                "-library",
                expected_sim_lib.as_str(),
                "-headers",
                "/repo/packages/core/host-cxx/include",
                "-output",
                "/repo/dist/NveStudio.xcframework",
            ]
        );
    }
}

#[test]
fn rejects_empty_invalid_duplicate_targets_and_bad_xcframework_extension() {
    assert_eq!(parse_targets(&[]), Err(Tier3Error::EmptyTargets));
    assert_eq!(
        parse_targets(&["tvos".into(), "ios".into()]),
        Err(Tier3Error::InvalidTarget("ios".into()))
    );
    assert_eq!(
        parse_targets(&["visionos".into(), "VISIONOS".into()]),
        Err(Tier3Error::DuplicateTarget(AppleTier3Target::VisionOs))
    );
    assert_eq!(
        parse_targets(&["watchos".into(), "aarch64-apple-watchos".into()]),
        Err(Tier3Error::DuplicateTarget(AppleTier3Target::WatchOs))
    );

    let dup_slices = vec![
        XcframeworkSlice {
            target: AppleTier3Target::Tvos,
            staticlib_path: PathBuf::from("a.a"),
        },
        XcframeworkSlice {
            target: AppleTier3Target::Tvos,
            staticlib_path: PathBuf::from("b.a"),
        },
    ];
    assert_eq!(
        plan_xcframework(
            dup_slices,
            PathBuf::from("/headers"),
            PathBuf::from("/out/App.xcframework")
        ),
        Err(Tier3Error::DuplicateTarget(AppleTier3Target::Tvos))
    );

    let bad_ext = plan_xcframework(
        vec![XcframeworkSlice {
            target: AppleTier3Target::Tvos,
            staticlib_path: PathBuf::from("a.a"),
        }],
        PathBuf::from("/headers"),
        PathBuf::from("/out/App.framework"),
    );
    assert_eq!(
        bad_ext,
        Err(Tier3Error::InvalidXcframeworkOutput(PathBuf::from(
            "/out/App.framework"
        )))
    );
}

#[test]
fn reports_actionable_diagnostics_for_missing_nightly_rust_src_sdks_and_headers() {
    let headers = PathBuf::from("/repo/packages/core/host-cxx/include");

    // 1. Missing nightly toolchain + missing SDKs + missing xcodebuild + missing header.
    let empty_probe = FakeProbe::default();
    let issues = check_prerequisites(
        &[AppleTier3Target::Tvos, AppleTier3Target::VisionOsSim],
        "nightly",
        true,
        &headers,
        &empty_probe,
    );
    assert!(issues.iter().any(|i| {
        i.kind == MissingPrerequisite::Toolchain
            && i.remediation.contains("rustup toolchain install nightly")
    }));
    assert!(issues.iter().any(|i| {
        i.kind == MissingPrerequisite::Sdk(AppleTier3Target::Tvos)
            && i.summary.contains("appletvos")
            && i.remediation.contains("xcodebuild -downloadPlatform tvOS")
    }));
    assert!(issues.iter().any(|i| {
        i.kind == MissingPrerequisite::Sdk(AppleTier3Target::VisionOsSim)
            && i.summary.contains("xrsimulator")
            && i.remediation.contains("xcodebuild -downloadPlatform visionOS")
    }));
    assert!(issues
        .iter()
        .any(|i| i.kind == MissingPrerequisite::Xcodebuild));
    assert!(issues.iter().any(|i| {
        i.kind == MissingPrerequisite::AbiHeader
            && i.summary.contains(NVE_ABI_HEADER_FILE)
            && i.remediation.contains("codegen abi")
    }));
    assert!(issues.iter().any(|i| {
        i.kind == MissingPrerequisite::ModuleMap && i.summary.contains(NVE_MODULE_MAP_FILE)
    }));

    // 2. Nightly installed, but rust-src missing and watchos SDK missing.
    let mut partial = FakeProbe::default();
    partial.toolchains.insert("nightly".into());
    let issues2 = check_prerequisites(
        &[AppleTier3Target::WatchOs],
        "nightly",
        false,
        &headers,
        &partial,
    );
    assert_eq!(issues2.len(), 2);
    assert_eq!(issues2[0].kind, MissingPrerequisite::RustSrc);
    assert!(issues2[0]
        .remediation
        .contains("rustup component add rust-src --toolchain nightly"));
    assert_eq!(
        issues2[1].kind,
        MissingPrerequisite::Sdk(AppleTier3Target::WatchOs)
    );
    assert!(issues2[1]
        .remediation
        .contains("xcodebuild -downloadPlatform watchOS"));

    // 3. Dry-run succeeds with warning summary even when prerequisites are missing,
    // while non-dry-run fails with the actionable error text.
    let dry_out = run_build_with_probe(
        BuildOptions {
            targets: vec!["visionos".into(), "visionos-sim".into()],
            package: "nve-studio-workspace".into(),
            lib_name: None,
            release: true,
            toolchain: "nightly".into(),
            xcframework: Some(PathBuf::from("")),
            headers: Some(headers.clone()),
            target_dir: Some(PathBuf::from("/repo/target")),
            dry_run: true,
        },
        Path::new("/repo"),
        &empty_probe,
    )
    .expect("dry-run should always emit a plan");
    assert!(dry_out.contains("build-std=std,panic_abort"));
    assert!(dry_out.contains("<SDKROOT:xros>"));
    assert!(dry_out.contains("XROS_DEPLOYMENT_TARGET=1.0"));
    assert!(dry_out.contains("xcodebuild -create-xcframework"));
    assert!(dry_out.contains("Prerequisite warnings:"));

    let real_err = run_build_with_probe(
        BuildOptions {
            targets: vec!["tvos".into()],
            package: "nve-studio-workspace".into(),
            lib_name: None,
            release: false,
            toolchain: "nightly".into(),
            xcframework: None,
            headers: Some(headers),
            target_dir: Some(PathBuf::from("/repo/target")),
            dry_run: false,
        },
        Path::new("/repo"),
        &empty_probe,
    )
    .expect_err("non-dry-run must fail when prerequisites are missing");
    assert!(real_err.contains("rustup toolchain install nightly"));
    assert!(real_err.contains("xcodebuild -downloadPlatform tvOS"));
}

#[test]
fn doctor_tier3_report_lists_all_six_sdks_and_std_readiness() {
    let headers = PathBuf::from("/repo/packages/core/host-cxx/include");
    let full = FakeProbe::all_ready(&headers);
    assert_eq!(probe_all_tier3_sdks(&full).len(), 6);
    let ok_text = format_tier3_doctor_section("nightly", &full);
    for sdk in [
        "appletvos",
        "appletvsimulator",
        "xros",
        "xrsimulator",
        "watchos",
        "watchsimulator",
    ] {
        assert!(ok_text.contains(sdk), "missing {sdk} in doctor report");
    }
    assert!(ok_text.contains("[OK]   tier3-std"));

    let empty = FakeProbe::default();
    let miss_text = format_tier3_doctor_section("nightly", &empty);
    assert!(miss_text.contains("[MISS] tvos"));
    assert!(miss_text.contains("[MISS] visionos"));
    assert!(miss_text.contains("[MISS] watchos"));
    assert!(miss_text.contains("rustup toolchain install nightly"));
}

#[test]
fn cli_parses_nve_build_flags_and_rejects_missing_target() {
    match Cli::try_parse_from([
        "nve",
        "build",
        "--target",
        "tvos,tvos-sim",
        "--package",
        "nve-example-kitchen-sink",
        "--release",
        "--xcframework",
        "dist/KitchenSink.xcframework",
        "--dry-run",
    ])
    .unwrap()
    .command
    {
        Commands::Build {
            targets,
            package,
            lib_name,
            release,
            toolchain,
            xcframework,
            headers,
            target_dir,
            dry_run,
        } => {
            assert_eq!(targets, vec!["tvos", "tvos-sim"]);
            assert_eq!(package, "nve-example-kitchen-sink");
            assert!(lib_name.is_none());
            assert!(release);
            assert_eq!(toolchain, "nightly");
            assert_eq!(
                xcframework,
                Some(PathBuf::from("dist/KitchenSink.xcframework"))
            );
            assert!(headers.is_none());
            assert!(target_dir.is_none());
            assert!(dry_run);
        }
        _ => panic!("expected Build command"),
    }

    // Bare `--xcframework` flag resolves to default `<target_dir>/xcframework/<lib>.xcframework`.
    match Cli::try_parse_from([
        "nve",
        "build",
        "--target",
        "watchos",
        "--target",
        "watchos-sim",
        "--xcframework",
        "--dry-run",
    ])
    .unwrap()
    .command
    {
        Commands::Build {
            targets,
            package,
            lib_name,
            release,
            toolchain,
            xcframework,
            headers,
            target_dir,
            dry_run,
        } => {
            let req = build_request_from_options(
                &BuildOptions {
                    targets,
                    package,
                    lib_name,
                    release,
                    toolchain,
                    xcframework,
                    headers,
                    target_dir,
                    dry_run,
                },
                Path::new("/repo"),
            )
            .unwrap();
            assert_eq!(
                req.targets,
                vec![AppleTier3Target::WatchOs, AppleTier3Target::WatchOsSim]
            );
            assert_eq!(
                req.xcframework_out,
                Some(PathBuf::from(
                    "/repo/target/xcframework/nve_studio_workspace.xcframework"
                ))
            );
            let plan = plan_tier3_build(&req, &FakeProbe::all_ready(&req.headers_dir)).unwrap();
            let rendered = format_build_plan(&plan, &[]);
            assert!(rendered.contains("NveStaticInterfaceTable"));
        }
        _ => panic!("expected Build command"),
    }

    // Missing `--target` must fail CLI parsing.
    assert!(Cli::try_parse_from(["nve", "build"]).is_err());

    // Single-slice xcframework emits a pairing tip for its counterpart slice.
    let single_dry = run_build_with_probe(
        BuildOptions {
            targets: vec!["tvos".into()],
            package: "nve-studio-workspace".into(),
            lib_name: None,
            release: false,
            toolchain: "nightly".into(),
            xcframework: Some(PathBuf::from("auto")),
            headers: Some(PathBuf::from("/repo/packages/core/host-cxx/include")),
            target_dir: Some(PathBuf::from("/repo/target")),
            dry_run: true,
        },
        Path::new("/repo"),
        &FakeProbe::all_ready(Path::new("/repo/packages/core/host-cxx/include")),
    )
    .expect("single-slice dry-run plan");
    assert!(single_dry.contains("--target tvos,tvos-sim"));
}
