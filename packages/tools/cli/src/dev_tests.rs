use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use nve_engine::HotPatchMsg;

use super::*;

#[test]
fn hot_reload_helpers_cover_platform_paths_and_filters() {
    let dylib = dylib_path_for_package_in(Path::new("target"), "nve-example-kitchen-sink");
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
fn child_process_guard_drop_kills_and_reaps_running_child() {
    use std::io::Read as _;
    use std::process::Stdio;

    let mut cmd = if cfg!(windows) {
        let mut c = ProcessCommand::new("ping");
        c.args(["-n", "30", "127.0.0.1"]);
        c
    } else {
        let mut c = ProcessCommand::new("sleep");
        c.arg("30");
        c
    };
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sleep child");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let guard = ChildProcessGuard(Some(child));

    let t0 = Instant::now();
    drop(guard);

    let mut buf = Vec::new();
    stdout.read_to_end(&mut buf).expect("stdout pipe closes on child exit");
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "ChildProcessGuard drop must terminate child promptly (took {:?})",
        t0.elapsed()
    );
}

#[test]
fn deferred_skia_launch_waits_for_first_successful_rebuild() {
    let (tx, rx) = bounded::<()>(1);
    assert!(wait_for_skia_launch_ready(true, &rx).is_ok());

    let handle = std::thread::spawn(move || wait_for_skia_launch_ready(false, &rx));
    tx.try_send(()).expect("signal first successful rebuild");
    assert!(handle.join().expect("join waiter").is_ok());

    let (tx_drop, rx_drop) = bounded::<()>(1);
    drop(tx_drop);
    assert!(wait_for_skia_launch_ready(false, &rx_drop).is_err());
}

#[test]
fn watch_classifier_forces_tier2_rebuild_until_first_build_succeeds() {
    let dir = std::env::temp_dir().join(format!(
        "nve-dev-watch-guard-{}",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("sample.rs");
    std::fs::write(&file, "fn view() { let _ = \"hello\"; }\n").expect("write temp rs");

    let mut cache = rsx_diff::RsxWatchCache::new();
    let paths = vec![file.clone()];
    // Before the first build succeeds, even a save must force a Tier-2 rebuild (never NoOp/Tier1).
    let (patches, reason) = classify_watch_changes(false, &paths, &mut cache, 2);
    assert!(patches.is_empty());
    assert_eq!(reason.as_deref(), Some("awaiting first successful build"));

    // Once built and seeded, an unchanged file is classified as a no-op.
    cache.seed_dir(&dir);
    let (patches_after, reason_after) = classify_watch_changes(true, &paths, &mut cache, 3);
    assert!(patches_after.is_empty());
    assert!(reason_after.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn watch_classifier_forwards_one_message_per_changed_element_for_typed_edits() {
    let dir = std::env::temp_dir().join(format!("nve-dev-watch-typed-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("ui.rs");
    let base = "fn ui() -> Element {\n    rsx! {\n        view { test_id: \"a\", padding: 8.0 }\n        view { test_id: \"b\", opacity: 0.5 }\n    }\n}\n";
    std::fs::write(&file, base).expect("write temp rs");
    let mut cache = rsx_diff::RsxWatchCache::new();
    cache.seed_dir(&dir);

    let edited = base.replace("padding: 8.0", "padding: 12.5").replace("0.5", "0.75");
    std::fs::write(&file, &edited).expect("rewrite temp rs");
    let (patches, reason) = classify_watch_changes(true, std::slice::from_ref(&file), &mut cache, 5);
    assert_eq!(reason, None);
    let [(path, messages)] = patches.as_slice() else {
        panic!("expected one file's messages: {patches:?}");
    };
    assert_eq!(path, &file);
    let updates: Vec<_> = messages
        .iter()
        .map(|msg| match msg {
            HotPatchMsg::TemplatePatch { attr_updates, target_test_id, .. } => {
                (target_test_id.clone(), attr_updates.clone())
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        updates,
        vec![
            (Some("a".to_owned()), vec![("padding".to_owned(), "12.5px".to_owned())]),
            (Some("b".to_owned()), vec![("opacity".to_owned(), "0.75".to_owned())]),
        ]
    );

    // A non-literal value in the next save forces Tier-2 and keeps the typed baseline.
    std::fs::write(&file, edited.replace("12.5", "gap")).expect("rewrite temp rs");
    let (patches, reason) = classify_watch_changes(true, std::slice::from_ref(&file), &mut cache, 6);
    assert!(patches.is_empty());
    assert!(reason.is_some_and(|r| r.contains("not a Tier-1 constant")));

    let _ = std::fs::remove_dir_all(&dir);
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
