//! `nve doctor` & `nve inspect` subcommands.

use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use nve_abi::{EventPayload, NativeEvent, NativeEventKind};
use nve_backend_headless::HeadlessBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{StoreBackend, Viewport};

use crate::build::probe::{format_tier3_doctor_section, SystemProbe};

pub(crate) fn probe_tool(cmd: &str, args: &[&str]) -> Option<String> {
    let out = ProcessCommand::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next().unwrap_or("ok").trim().to_string();
    Some(if line.is_empty() { "ok".into() } else { line })
}

pub(crate) fn cmd_doctor() {
    println!("Native View Engine — Environment Doctor\n");
    let tools: &[(&str, &[&str])] = &[
        ("rustc", &["--version"]),
        ("cargo", &["--version"]),
        ("kache", &["--version"]),
        ("clang++", &["--version"]),
        ("cmake", &["--version"]),
        ("ninja", &["--version"]),
        ("ccache", &["--version"]),
        ("ld64.lld", &["--version"]),
        ("xcodebuild", &["-version"]),
        ("swift", &["--version"]),
        ("flutter", &["--version"]),
        ("adb", &["version"]),
        ("wasm-bindgen", &["--version"]),
    ];

    for (name, args) in tools {
        match probe_tool(name, args) {
            Some(ver) => println!("  [OK]   {name:<14} {ver}"),
            None => println!("  [MISS] {name:<14} not found in PATH"),
        }
    }

    if let Ok(out) = ProcessCommand::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    {
        let targets: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        println!(
            "\n  Installed Rust targets ({}): {}",
            targets.len(),
            targets.join(", ")
        );
    }

    print!("{}", format_tier3_doctor_section("nightly", &SystemProbe));

    match std::env::var("RN_REACT_COMMON_DIR") {
        Ok(raw) if !raw.trim().is_empty() => {
            let rn_path = PathBuf::from(raw.trim());
            if rn_path.exists() {
                println!("  [OK]   RN Fabric      {}", rn_path.display());
            } else {
                println!(
                    "  [WARN] RN Fabric      RN_REACT_COMMON_DIR does not exist: {}",
                    rn_path.display()
                );
            }
        }
        _ => {
            println!(
                "  [WARN] RN Fabric      RN_REACT_COMMON_DIR not set (optional for standalone builds)"
            );
        }
    }
}

#[component]
pub(crate) fn InspectDemo() -> Element {
    let mut count = use_signal(|| 0i32);
    rsx! {
        view {
            style: "width: 100%; height: 100%; padding: 20px; gap: 12px; background-color: #0f172a;",
            text {
                style: "color: #ffffff; font-size: 24px;",
                "Counter: {count}"
            }
            pressable {
                style: "width: 140px; height: 44px; background-color: #2563eb; border-radius: 10px;",
                onpress: move |_| count += 1,
                text {
                    style: "color: #ffffff; font-size: 16px;",
                    "Tap +1"
                }
            }
        }
    }
}

pub(crate) fn cmd_inspect() {
    let mut surface = DioxusSurface::new(1, InspectDemo);
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 393.0,
            height: 852.0,
            scale_factor: 3.0,
        },
    );

    let (rec_us, s1) = surface.commit_to_timed(1, &mut backend).expect("frame 1");
    let total1 = rec_us + s1.layout_us + s1.diff_us + s1.mount_us;
    println!(
        "=== Frame 1 (initial) | {} ops | total={}us (rec={}us layout={}us diff={}us) ===",
        s1.op_count, total1, rec_us, s1.layout_us, s1.diff_us
    );
    print!("{}", backend.render_ascii_tree(surface.store()));

    let hit = backend
        .hit_test(surface.store(), 60.0, 75.0)
        .expect("expected pressable button hit at (60, 75)");
    println!("\n--> Simulated native Press at (60, 75) hit Node#{hit}");

    let handled = surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: hit,
        timestamp_us: 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 60.0, y: 75.0 },
    }]);
    assert!(handled, "press event should have been handled");

    let (rec2, s2) = surface.commit_to_timed(2, &mut backend).expect("frame 2");
    let total2 = rec2 + s2.layout_us + s2.diff_us + s2.mount_us;
    println!(
        "\n=== Frame 2 (after tap) | {} ops | total={}us (rec={}us layout={}us diff={}us) ===",
        s2.op_count, total2, rec2, s2.layout_us, s2.diff_us
    );
    print!("{}", backend.render_ascii_tree(surface.store()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_tool_and_inspect_demo_work_headlessly() {
        assert!(probe_tool("rustc", &["--version"]).is_some());
        assert!(probe_tool("__definitely_missing_nve_binary__", &["--version"]).is_none());
        assert!(probe_tool("rustc", &["--definitely-invalid-flag-xyz"]).is_none());

        let mut surface = DioxusSurface::new(1, InspectDemo);
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(1, Viewport::default());
        surface.commit_to(1, &mut backend).unwrap();
        assert!(backend.render_ascii_tree(surface.store()).contains("Counter: 0"));

        let hit = backend
            .hit_test(surface.store(), 60.0, 75.0)
            .expect("hit button");
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: hit,
            timestamp_us: 1000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 60.0, y: 75.0 },
        }]));
        surface.commit_to(2, &mut backend).unwrap();
        assert!(backend.render_ascii_tree(surface.store()).contains("Counter: 1"));
    }
}
