//! Native View Engine — Skia Desktop Shell CLI (`nve-skia-shell`).
//!
//! Launches any compiled NVE `cdylib` (`nve_get_interface`) inside a live cross-platform
//! `winit` window (direct Metal/Vulkan/D3D12 GPU swapchain when enabled, or zero-C++ `softbuffer`),
//! or renders a headless PPM screenshot with optional Frame-0 `MmapSnapshot` replay & atomic save.
//!
//! Usage:
//!   cargo run -p nve-skia-shell -- \
//!     target/debug/libnve_example_counter.dylib \
//!     [--ws ws://127.0.0.1:8089 | --no-ws] \
//!     [--snapshot <path.nvesnap>] [--schema <ver>] \
//!     [--screenshot <out.ppm>] [--width W] [--height H] [--scale S] [--no-hud]

use std::path::PathBuf;

use nve_skia_shell::{
    render_headless_screenshot, run_desktop_preview_with_options, DesktopPreviewOptions,
    DEFAULT_WS_URL,
};

#[derive(Debug, PartialEq)]
struct CliConfig {
    dylib_path: PathBuf,
    options: DesktopPreviewOptions,
    screenshot_path: Option<PathBuf>,
}

const USAGE: &str = "\
Usage: nve-skia-shell <dylib_path> [OPTIONS]

Options:
  --dylib <path>            Path to the compiled NVE app cdylib (alternative to positional arg)
  --ws <url>                Connect to nve dev WebSocket hot-reload server (default: ws://127.0.0.1:8089)
  --no-ws                   Disable WebSocket hot-reload client
  --snapshot <path.nvesnap> Replay Frame-0 MmapSnapshot before dylib load & atomically save on exit
  --schema <ver>            Expected snapshot app schema version (default: 1)
  --screenshot <out.ppm>    Render headlessly to a P6 PPM file and exit (no window opened)
  --width <px>              Logical window/viewport width (default: 420)
  --height <px>             Logical window/viewport height (default: 780)
  --scale <factor>          HiDPI scale factor for headless rendering (default: 1.0)
  --no-hud                  Hide the top-left hot-reload HUD banner
  -h, --help                Show this help message";

fn parse_args<I>(args: I) -> Result<Option<CliConfig>, String>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    let mut dylib_path: Option<PathBuf> = None;
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
            "--ws" => {
                let url = iter
                    .next()
                    .ok_or_else(|| "--ws requires a WebSocket URL argument".to_string())?;
                options.ws_url = Some(url);
            }
            "--no-ws" => {
                options.ws_url = None;
            }
            "--snapshot" => {
                let path = iter
                    .next()
                    .ok_or_else(|| "--snapshot requires a file path argument".to_string())?;
                options.snapshot_path = Some(PathBuf::from(path));
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
                    .ok_or_else(|| "--screenshot requires an output PPM path".to_string())?;
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

    let Some(dylib_path) = dylib_path else {
        return Err("missing <dylib_path> argument".to_string());
    };

    Ok(Some(CliConfig {
        dylib_path,
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

    let result = if let Some(screenshot_path) = config.screenshot_path {
        render_headless_screenshot(config.dylib_path, config.options, screenshot_path)
    } else {
        run_desktop_preview_with_options(config.dylib_path, config.options)
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

        assert_eq!(cfg.dylib_path, PathBuf::from("target/debug/libapp.dylib"));
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
    fn parses_help_and_rejects_invalid_args() {
        assert_eq!(parse_args(s(&["--help"])).unwrap(), None);
        assert_eq!(parse_args(s(&["-h"])).unwrap(), None);
        assert!(parse_args(s(&[])).is_err());
        assert!(parse_args(s(&["libapp.dylib", "--schema", "abc"])).is_err());
        assert!(parse_args(s(&["libapp.dylib", "--width", "0"])).is_err());
        assert!(parse_args(s(&["libapp.dylib", "--bogus"])).is_err());
    }
}
