//! Live desktop preview runner for any compiled NVE `cdylib` (`nve_get_interface`).
//!
//! Usage:
//!   cargo run -p nve-backend-skia --features desktop --example preview -- \
//!     target/debug/libnve_example_counter.dylib [--ws ws://127.0.0.1:8089 | --no-ws]

use std::path::PathBuf;

use nve_backend_skia::{run_desktop_preview, DEFAULT_WS_URL};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dylib_path: Option<PathBuf> = None;
    let mut ws_url: Option<String> = Some(DEFAULT_WS_URL.to_string());

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--ws" => {
                let url = args.next().unwrap_or_else(|| {
                    eprintln!("--ws requires a WebSocket URL argument");
                    std::process::exit(2);
                });
                ws_url = Some(url);
            }
            "--no-ws" => {
                ws_url = None;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: preview <path/to/libapp.dylib> [--ws ws://127.0.0.1:8089 | --no-ws]"
                );
                return;
            }
            other if dylib_path.is_none() => {
                dylib_path = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let Some(path) = dylib_path else {
        eprintln!("Usage: preview <path/to/libapp.dylib> [--ws ws://127.0.0.1:8089 | --no-ws]");
        std::process::exit(2);
    };

    if let Err(err) = run_desktop_preview(path, ws_url) {
        eprintln!("Desktop preview exited with error: {err}");
        std::process::exit(1);
    }
}
