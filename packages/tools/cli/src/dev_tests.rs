use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use nve_engine::HotPatchMsg;

use super::*;

#[test]
fn hot_reload_helpers_cover_platform_paths_and_filters() {
    let dylib = dylib_path_for_package("nve-example-kitchen-sink");
    let s = dylib.to_string_lossy();
    assert!(s.contains("nve_example_kitchen_sink"), "unexpected path: {s}");
    if cfg!(target_os = "macos") {
        assert_eq!(s, "target/debug/libnve_example_kitchen_sink.dylib");
    } else if cfg!(target_os = "windows") {
        assert_eq!(s, "target/debug/nve_example_kitchen_sink.dll");
    } else {
        assert_eq!(s, "target/debug/libnve_example_kitchen_sink.so");
    }

    let custom_dylib = dylib_path_for_package_in(Path::new("/var/cache/cargo"), "nve-studio-workspace");
    assert!(custom_dylib.starts_with("/var/cache/cargo/debug"));

    assert_eq!(ws_url_for_port(8089), "ws://127.0.0.1:8089");
    assert_eq!(ws_bind_addr("0.0.0.0", 8089), "0.0.0.0:8089");
    assert_eq!(
        default_watch_dir_for_package("nve-studio-workspace"),
        PathBuf::from("examples/studio-workspace/src")
    );
    assert_eq!(
        default_watch_dir_for_package("nve-example-kitchen-sink"),
        PathBuf::from("examples/kitchen-sink/src")
    );
    assert!(is_relevant_source_path(Path::new(
        "examples/studio-workspace/src/ui.rs"
    )));
    assert!(!is_relevant_source_path(Path::new(
        "examples/studio-workspace/src/.ui.rs.swp"
    )));

    let msg = make_dylib_reload_msg(3, &dylib, 180);
    assert_eq!(HotPatchMsg::from_bytes(&msg.to_bytes()).unwrap(), msg);

    use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};
    assert!(should_trigger_rebuild(&notify::EventKind::Create(
        CreateKind::File
    )));
    assert!(should_trigger_rebuild(&notify::EventKind::Modify(
        ModifyKind::Any
    )));
    assert!(!should_trigger_rebuild(&notify::EventKind::Access(
        AccessKind::Any
    )));
    assert!(!should_trigger_rebuild(&notify::EventKind::Remove(
        RemoveKind::File
    )));
}

#[test]
fn skia_shell_subprocess_resolution_and_args_reject_missing_override() {
    let err = resolve_explicit_skia_shell_bin(OsStr::new("/nonexistent/path/to/nve-skia-shell"))
        .expect_err("non-existent NVE_SKIA_SHELL_BIN must return explicit error");
    assert!(err.contains("NVE_SKIA_SHELL_BIN"), "unexpected err: {err}");
    assert!(resolve_explicit_skia_shell_bin(OsStr::new("")).is_err());

    let parsed = parse_cargo_target_dir(br#"{"target_directory":"/tmp/custom target"}"#).unwrap();
    assert_eq!(parsed, PathBuf::from("/tmp/custom target"));
    assert!(parse_cargo_target_dir(br#"{"target_directory":""}"#).is_err());
    assert!(parse_cargo_target_dir(b"{}").is_err());

    let args = skia_shell_args(Path::new("target/debug/libapp.dylib"), "ws://127.0.0.1:8089");
    assert_eq!(
        args,
        [
            std::ffi::OsString::from("--dylib"),
            std::ffi::OsString::from("target/debug/libapp.dylib"),
            std::ffi::OsString::from("--ws"),
            std::ffi::OsString::from("ws://127.0.0.1:8089"),
        ]
    );
}

#[test]
fn web_dev_redirect_wires_bare_urls_to_the_daemon_socket_and_respects_explicit_modes() {
    let r = |target, host| web_dev_redirect(target, host, 8089);
    assert_eq!(r("/", Some("127.0.0.1:8088")).as_deref(), Some("/?ws=ws://127.0.0.1:8089"));
    assert_eq!(
        r("/index.html?app=kitchen-sink", Some("192.168.1.20:8088")).as_deref(),
        Some("/index.html?app=kitchen-sink&ws=ws://192.168.1.20:8089"),
        "LAN clients get the host they used; other params survive"
    );
    assert_eq!(r("/", Some("[::1]:8088")).as_deref(), Some("/?ws=ws://[::1]:8089"));
    assert_eq!(r("/", Some("devbox")).as_deref(), Some("/?ws=ws://devbox:8089"));
    assert_eq!(r("/", None).as_deref(), Some("/?ws=ws://127.0.0.1:8089"));
    assert_eq!(r("/", Some("")).as_deref(), Some("/?ws=ws://127.0.0.1:8089"));
    assert_eq!(
        web_dev_redirect("/?app=studio", None, 9001).as_deref(),
        Some("/?app=studio&ws=ws://127.0.0.1:9001")
    );
    for target in [
        "/?ws=off",
        "/?dev",
        "/?dev=1",
        "/?pair=ws://x&token=t",
        "/?bundle=studio",
        "/?app=kitchen-sink&ws=ws://h:1",
    ] {
        assert_eq!(r(target, Some("h:8088")), None, "{target}");
    }
    for target in [
        "/pkg/dev/manifest.json",
        "/pkg/dev/0123456789abcdef/nve_web_shell.js",
        "/pkg/dev/0123456789abcdef/nve_web_shell_bg.wasm",
        "/e2e/run_e2e.mjs",
    ] {
        assert_eq!(r(target, Some("h:8088")), None, "{target}");
    }
    assert_eq!(r("/?wsx=1", None).as_deref(), Some("/?wsx=1&ws=ws://127.0.0.1:8089"));
}
