//! Native View Engine — Skia Desktop Shell CLI (`nve-skia-shell`).
//!
//! Launches any compiled NVE `cdylib` (`nve_get_interface`) inside a live cross-platform
//! `winit` window (direct Metal/Vulkan/D3D12 GPU swapchain when enabled, or zero-C++ `softbuffer`),
//! or renders a headless PPM screenshot with optional Frame-0 `MmapSnapshot` replay & atomic save.
//!
//! Usage:
//!   cargo run -p nve-skia-shell -- \
//!     target/debug/libnve_example_kitchen_sink.dylib \
//!     [--ws ws://127.0.0.1:8089 | --no-ws] \
//!     [--snapshot <path.nvesnap>] [--schema <ver>] \
//!     [--screenshot <out.ppm>] [--width W] [--height H] [--scale S] [--no-hud]

use std::path::PathBuf;

use nve_skia_shell::{
    render_headless_screenshot, render_paired_headless_screenshot,
    run_desktop_preview_with_options, run_paired_desktop_preview, DesktopPreviewOptions,
    PairConnectConfig, PairTransport, DEFAULT_WS_URL,
};

#[derive(Debug, PartialEq)]
enum LaunchTarget {
    LocalDylib(PathBuf),
    Pair(PairConnectConfig),
}

#[derive(Debug, PartialEq)]
struct CliConfig {
    target: LaunchTarget,
    options: DesktopPreviewOptions,
    screenshot_path: Option<PathBuf>,
}

const USAGE: &str = "\
Usage:
  nve-skia-shell <dylib_path> [OPTIONS]
  nve-skia-shell --pair-connect <host:port> --pair-token <token> [--pair-transport <udp|tcp|ws>] [OPTIONS]

Options:
  --dylib <path>            Path to the compiled NVE app cdylib (alternative to positional arg)
  --pair-connect <addr>     Connect as a thin client to a live `nve pair` host (host:port or ws://.../ws/nvd)
  --pair-token <token>      Pairing token printed by `nve pair` (required with --pair-connect)
  --pair-transport <mode>   Live pairing transport: udp, tcp, or ws (default: ws, or inferred from URL scheme)
  --ws <url>                Connect to nve dev WebSocket hot-reload server (default: ws://127.0.0.1:8089)
  --no-ws                   Disable WebSocket hot-reload client
  --snapshot <path.nvereloc>       Replay Frame-0 NVERELOC before dylib load & atomically save on exit
  --instant-boot-dir <dir>         Per-user NativeInstantBootStore dir (default: auto per-app cache dir)
  --default-frame0 <path.nvereloc> Bundled cold-start frame0.nvereloc for first launch with --instant-boot-dir
  --no-snapshot                    Disable default instant-boot Frame-0 replay & state persistence
  --reset-state                    Clear cached snapshot/state before launching fresh
  --schema <ver>                   Expected snapshot app schema version (default: 1)
  --screenshot <out.png|ppm> Render headlessly to a PNG or P6 PPM file and exit (no window opened)
  --width <px>              Logical window/viewport width (default: 420)
  --height <px>             Logical window/viewport height (default: 780)
  --scale <factor>          HiDPI scale factor for headless rendering (default: 1.0)
  --no-hud                  Hide the top-left hot-reload HUD banner
  -h, --help                Show this help message";

fn infer_transport_from_addr(addr: &str) -> Option<PairTransport> {
    let lower = addr.trim().to_ascii_lowercase();
    if lower.starts_with("udp://") {
        Some(PairTransport::Udp)
    } else if lower.starts_with("tcp://") {
        Some(PairTransport::Tcp)
    } else if lower.starts_with("ws://") || lower.starts_with("wss://") {
        Some(PairTransport::Ws)
    } else {
        None
    }
}

fn parse_args<I>(args: I) -> Result<Option<CliConfig>, String>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    let mut dylib_path: Option<PathBuf> = None;
    let mut pair_connect: Option<String> = None;
    let mut pair_token: Option<String> = None;
    let mut pair_transport: Option<PairTransport> = None;
    let mut explicit_ws = false;
    let mut options = DesktopPreviewOptions {
        ws_url: Some(DEFAULT_WS_URL.to_string()),
        ..Default::default()
    };
    let mut screenshot_path: Option<PathBuf> = None;

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--dylib" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--dylib requires a path argument".to_string())?;
                dylib_path = Some(PathBuf::from(val));
            }
            "--pair-connect" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--pair-connect requires a host:port or URL argument".to_string())?;
                pair_connect = Some(val);
            }
            "--pair-token" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "--pair-token requires a pairing token argument".to_string())?;
                pair_token = Some(val);
            }
            "--pair-transport" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--pair-transport requires one of: udp, tcp, ws".to_string())?;
                pair_transport = Some(PairTransport::parse(&raw)?);
            }
            "--ws" => {
                let url = iter
                    .next()
                    .ok_or_else(|| "--ws requires a WebSocket URL argument".to_string())?;
                options.ws_url = Some(url);
                explicit_ws = true;
            }
            "--no-ws" => {
                options.ws_url = None;
                explicit_ws = true;
            }
            "--snapshot" => {
                let path = iter
                    .next()
                    .ok_or_else(|| "--snapshot requires a file path argument".to_string())?;
                options.snapshot_path = Some(PathBuf::from(path));
            }
            "--instant-boot-dir" => {
                let path = iter
                    .next()
                    .ok_or_else(|| "--instant-boot-dir requires a directory path argument".to_string())?;
                options.instant_boot_dir = Some(PathBuf::from(path));
            }
            "--default-frame0" => {
                let path = iter
                    .next()
                    .ok_or_else(|| "--default-frame0 requires a file path argument".to_string())?;
                options.default_frame0_path = Some(PathBuf::from(path));
            }
            "--no-snapshot" => {
                options.disable_instant_boot = true;
                options.snapshot_path = None;
                options.instant_boot_dir = None;
            }
            "--reset-state" => {
                options.reset_state = true;
            }
            "--schema" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--schema requires a u32 version argument".to_string())?;
                options.schema_version = raw
                    .parse::<u32>()
                    .map_err(|_| format!("invalid --schema version: '{raw}'"))?;
            }
            "--screenshot" => {
                let path = iter
                    .next()
                    .ok_or_else(|| "--screenshot requires an output PNG or PPM path".to_string())?;
                screenshot_path = Some(PathBuf::from(path));
            }
            "--width" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--width requires a positive number".to_string())?;
                let val = raw
                    .parse::<f32>()
                    .map_err(|_| format!("invalid --width value: '{raw}'"))?;
                if !val.is_finite() || val <= 0.0 {
                    return Err(format!("--width must be positive, got '{raw}'"));
                }
                options.width = val;
            }
            "--height" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--height requires a positive number".to_string())?;
                let val = raw
                    .parse::<f32>()
                    .map_err(|_| format!("invalid --height value: '{raw}'"))?;
                if !val.is_finite() || val <= 0.0 {
                    return Err(format!("--height must be positive, got '{raw}'"));
                }
                options.height = val;
            }
            "--scale" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--scale requires a positive number".to_string())?;
                let val = raw
                    .parse::<f32>()
                    .map_err(|_| format!("invalid --scale value: '{raw}'"))?;
                if !val.is_finite() || val <= 0.0 {
                    return Err(format!("--scale must be positive, got '{raw}'"));
                }
                options.scale_factor = val;
            }
            "--no-hud" => {
                options.show_hud = false;
            }
            other if !other.starts_with('-') && dylib_path.is_none() => {
                dylib_path = Some(PathBuf::from(other));
            }
            other => {
                return Err(format!("unknown argument: '{other}'"));
            }
        }
    }

    let target = if let Some(addr) = pair_connect {
        let token = pair_token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "--pair-token is required when using --pair-connect".to_string())?;
        let transport = pair_transport
            .or_else(|| infer_transport_from_addr(&addr))
            .unwrap_or(PairTransport::Ws);
        if !explicit_ws {
            options.ws_url = None;
        }
        LaunchTarget::Pair(PairConnectConfig::new(addr, token, transport))
    } else if let Some(dylib_path) = dylib_path {
        LaunchTarget::LocalDylib(dylib_path)
    } else {
        return Err("missing <dylib_path> or --pair-connect <host:port> argument".to_string());
    };

    Ok(Some(CliConfig {
        target,
        options,
        screenshot_path,
    }))
}

fn main() {
    let config = match parse_args(std::env::args().skip(1)) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            println!("{USAGE}");
            return;
        }
        Err(err) => {
            eprintln!("error: {err}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let result = match (config.target, config.screenshot_path) {
        (LaunchTarget::LocalDylib(dylib_path), Some(screenshot_path)) => {
            render_headless_screenshot(dylib_path, config.options, screenshot_path)
        }
        (LaunchTarget::LocalDylib(dylib_path), None) => {
            run_desktop_preview_with_options(dylib_path, config.options)
        }
        (LaunchTarget::Pair(pair_cfg), Some(screenshot_path)) => {
            render_paired_headless_screenshot(pair_cfg, config.options, screenshot_path)
        }
        (LaunchTarget::Pair(pair_cfg), None) => {
            run_paired_desktop_preview(pair_cfg, config.options)
        }
    };

    if let Err(err) = result {
        eprintln!("nve-skia-shell exited with error: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn parses_positional_dylib_and_all_flags() {
        let cfg = parse_args(s(&[
            "target/debug/libapp.dylib",
            "--ws",
            "ws://127.0.0.1:9000",
            "--snapshot",
            "/tmp/app.nvesnap",
            "--schema",
            "4",
            "--screenshot",
            "/tmp/out.ppm",
            "--width",
            "960",
            "--height",
            "640",
            "--scale",
            "2.0",
            "--no-hud",
        ]))
        .unwrap()
        .expect("parsed config");

        assert_eq!(
            cfg.target,
            LaunchTarget::LocalDylib(PathBuf::from("target/debug/libapp.dylib"))
        );
        assert_eq!(cfg.options.ws_url.as_deref(), Some("ws://127.0.0.1:9000"));
        assert_eq!(
            cfg.options.snapshot_path,
            Some(PathBuf::from("/tmp/app.nvesnap"))
        );
        assert_eq!(cfg.options.schema_version, 4);
        assert_eq!(cfg.screenshot_path, Some(PathBuf::from("/tmp/out.ppm")));
        assert_eq!(cfg.options.width, 960.0);
        assert_eq!(cfg.options.height, 640.0);
        assert_eq!(cfg.options.scale_factor, 2.0);
        assert!(!cfg.options.show_hud);
    }

    #[test]
    fn parses_pair_connect_flags_without_local_dylib() {
        let cfg = parse_args(s(&[
            "--pair-connect",
            "127.0.0.1:8091",
            "--pair-token",
            "nve-deadbeef-12345678",
            "--pair-transport",
            "udp",
            "--screenshot",
            "/tmp/paired.png",
            "--width",
            "393",
            "--height",
            "852",
        ]))
        .unwrap()
        .expect("parsed pair config");

        assert_eq!(
            cfg.target,
            LaunchTarget::Pair(PairConnectConfig::new(
                "127.0.0.1:8091",
                "nve-deadbeef-12345678",
                PairTransport::Udp,
            ))
        );
        assert_eq!(cfg.options.ws_url, None);
        assert_eq!(cfg.screenshot_path, Some(PathBuf::from("/tmp/paired.png")));
        assert_eq!(cfg.options.width, 393.0);
        assert_eq!(cfg.options.height, 852.0);

        // Default transport is WS when omitted on bare host:port, and inferred from tcp:// scheme.
        let ws_default = parse_args(s(&[
            "--pair-connect",
            "127.0.0.1:8092",
            "--pair-token",
            "tok",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            ws_default.target,
            LaunchTarget::Pair(PairConnectConfig::new(
                "127.0.0.1:8092",
                "tok",
                PairTransport::Ws,
            ))
        );

        let tcp_inferred = parse_args(s(&[
            "--pair-connect",
            "tcp://127.0.0.1:8092",
            "--pair-token",
            "tok",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            tcp_inferred.target,
            LaunchTarget::Pair(PairConnectConfig::new(
                "tcp://127.0.0.1:8092",
                "tok",
                PairTransport::Tcp,
            ))
        );
    }

    #[test]
    fn parses_help_and_rejects_invalid_args() {
        assert_eq!(parse_args(s(&["--help"])).unwrap(), None);
        assert_eq!(parse_args(s(&["-h"])).unwrap(), None);
        assert!(parse_args(s(&[])).is_err());
        assert!(parse_args(s(&["--pair-connect", "127.0.0.1:8092"])).is_err());
        assert!(parse_args(s(&[
            "--pair-connect",
            "127.0.0.1:8092",
            "--pair-token",
            "tok",
            "--pair-transport",
            "quic"
        ]))
        .is_err());
        assert!(parse_args(s(&["libapp.dylib", "--schema", "abc"])).is_err());
        assert!(parse_args(s(&["libapp.dylib", "--width", "0"])).is_err());
        assert!(parse_args(s(&["libapp.dylib", "--bogus"])).is_err());
    }

    #[test]
    fn parses_no_snapshot_and_reset_state_flags() {
        let disabled = parse_args(s(&["libapp.dylib", "--no-snapshot"]))
            .unwrap()
            .unwrap();
        assert!(disabled.options.disable_instant_boot);
        assert!(!disabled.options.reset_state);

        let reset = parse_args(s(&["libapp.dylib", "--reset-state"]))
            .unwrap()
            .unwrap();
        assert!(!reset.options.disable_instant_boot);
        assert!(reset.options.reset_state);
    }
}
