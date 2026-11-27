//! Native View Engine — Skia Desktop Shell CLI (`nve-skia-shell`).
//!
//! Launches any compiled NVE `cdylib` (`nve_get_interface`) inside a live cross-platform
//! `winit` window (direct Metal/Vulkan/D3D12 GPU swapchain when enabled, or zero-C++ `softbuffer`),
//! or renders a headless PPM screenshot with optional Frame-0 `frame0.nve2` replay & save.
//!
//! Usage:
//!   cargo run -p nve-skia-shell -- \
//!     target/debug/libnve_example_kitchen_sink.dylib \
//!     [--ws ws://127.0.0.1:8089 | --no-ws] \
//!     [--frame0 <path.nve2>] \
//!     [--screenshot <out.ppm>] [--width W] [--height H] [--scale S] [--no-hud]

mod host_services;
mod window;

use std::path::PathBuf;

use window::{
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
  --frame0 <path.nve2>      Pre-warm Frame-0 NVE2 keyframe before dylib load & atomically refresh on exit
  --screenshot <out.png|ppm> Render headlessly to a PNG or P6 PPM file and exit (no window opened)
  --width <px>              Logical window/viewport width (default: 420)
  --height <px>             Logical window/viewport height (default: 780)
  --scale <factor>          HiDPI scale factor for headless rendering (default: 1.0)
  --no-hud                  Hide the top-left hot-reload HUD banner
  -h, --help                Show this help message";

fn infer_transport_from_addr(addr: &str) -> Option<PairTransport> {
    let trimmed = addr.trim();
    let has_prefix = |p: &str| {
        trimmed.len() >= p.len()
            && trimmed.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes())
    };
    if has_prefix("quic://") || has_prefix("udp://") {
        Some(PairTransport::Quic)
    } else if has_prefix("https://") || has_prefix("wt://") {
        Some(PairTransport::WebTransport)
    } else if has_prefix("wss://") {
        Some(PairTransport::Wss)
    } else if has_prefix("ws://") || has_prefix("tcp://") {
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
            "--frame0" => {
                let path = iter
                    .next()
                    .ok_or_else(|| "--frame0 requires a file path argument".to_string())?;
                options.frame0_path = Some(PathBuf::from(path));
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
            .unwrap_or(PairTransport::Auto);
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
            "--frame0",
            "/tmp/frame0.nve2",
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
            cfg.options.frame0_path,
            Some(PathBuf::from("/tmp/frame0.nve2"))
        );
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
                PairTransport::Quic,
            ))
        );
        assert_eq!(cfg.options.ws_url, None);
        assert_eq!(cfg.screenshot_path, Some(PathBuf::from("/tmp/paired.png")));
        assert_eq!(cfg.options.width, 393.0);
        assert_eq!(cfg.options.height, 852.0);

        // Default transport is Auto (Happy-Eyeballs) when omitted on bare host:port.
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
                PairTransport::Auto,
            ))
        );

        let quic_inferred = parse_args(s(&[
            "--pair-connect",
            "quic://127.0.0.1:8091",
            "--pair-token",
            "tok",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            quic_inferred.target,
            LaunchTarget::Pair(PairConnectConfig::new(
                "quic://127.0.0.1:8091",
                "tok",
                PairTransport::Quic,
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
            "bogus-proto"
        ]))
        .is_err());
        assert!(parse_args(s(&["libapp.dylib", "--frame0"])).is_err());
        assert!(parse_args(s(&["libapp.dylib", "--width", "0"])).is_err());
        assert!(parse_args(s(&["libapp.dylib", "--bogus"])).is_err());
    }
}
