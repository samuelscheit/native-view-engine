//! `nve` — Best-in-class developer CLI for `native-view-engine`.
//!
//! Subcommands:
//! - `nve doctor`  — verifies Rust targets, caches (kache/ccache), CMake/Ninja, Xcode/Android/Flutter/Swift SDKs, and RN checkout.
//! - `nve inspect` — runs a Dioxus `rsx!` app headlessly, simulates native press events, and dumps ShadowTrees + Flexbox rects.
//! - `nve bench`   — runs 1,000-node initial + incremental multi-backend benchmarks and exports Perfetto/Chrome JSON traces.
//! - `nve dev`     — starts the 2-tier hot-reload daemon (`--skia` desktop preview, `--pair` live `NVD1` device pairing).
//! - `nve pair`    — streams live `NVD1` binary deltas over UDP + TCP/WS and ingests remote `InputEvents`.
//! - `nve codegen` — bi-directional RN <-> NVE + Flutter/SwiftUI/Compose/Web component code generation.

mod bench;
mod codegen;
mod dev;
mod doctor;
mod pair;
mod rsx_diff;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::codegen::CodegenAction;
use crate::dev::DevOptions;
use crate::pair::PairOptions;

#[derive(Parser)]
#[command(name = "nve", about = "Native View Engine developer CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Check local toolchain, mobile SDKs, compilation caches, and RN Fabric sources.
    Doctor,
    /// Render a Dioxus rsx! surface headlessly, simulate native taps, and print the layout tree.
    Inspect,
    /// Benchmark 1,000-node initial commit + incremental updates across Headless, Skia, and Web backends.
    Bench {
        /// Number of rows to benchmark in the virtual tree.
        #[arg(long, default_value_t = 1000)]
        rows: usize,
        /// Number of incremental update frames to run.
        #[arg(long, default_value_t = 50)]
        frames: usize,
        /// Optional path to write a Perfetto/Chrome trace JSON file.
        #[arg(long)]
        trace: Option<PathBuf>,
    },
    /// Run the 2-tier hot-reload daemon (WebSocket server + incremental cdylib rebuild on save).
    Dev {
        /// Crate/package name of the cdylib example to watch and hot-rebuild.
        #[arg(long, default_value = "nve-studio-workspace")]
        package: String,
        /// Directory to watch for Rust source changes (defaults to the package's `src/` directory).
        #[arg(long)]
        watch: Option<PathBuf>,
        /// Bind address for the hot-reload WebSocket server (`0.0.0.0` allows LAN/phone clients).
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// WebSocket port for Tier-1/Tier-2 hot-reload clients.
        #[arg(long, default_value_t = 8089)]
        port: u16,
        /// Rebuild the Web WASM bundle (`hosts/web-shell/build.sh --dev`) on save.
        #[arg(long, default_value_t = false)]
        web: bool,
        /// Skip rebuilding the Web WASM bundle (`hosts/web-shell/build.sh --dev`) on save.
        #[arg(long, default_value_t = false)]
        no_web: bool,
        /// Launch the live cross-platform desktop preview window (`winit` + `softbuffer`) connected to this daemon.
        #[arg(long, default_value_t = false)]
        skia: bool,
        /// Enable live `NVD1` device pairing (UDP + TCP/WebSocket delta streaming + remote input backchannel).
        #[arg(long, default_value_t = false)]
        pair: bool,
        /// UDP port for live `NVD1` delta pairing when `--pair` is enabled.
        #[arg(long, default_value_t = 8091)]
        pair_udp_port: u16,
        /// TCP/WebSocket port for live `NVD1` delta pairing when `--pair` is enabled.
        #[arg(long, default_value_t = 8092)]
        pair_ws_port: u16,
        /// Optional pairing token secret for `--pair` (auto-generated when omitted).
        #[arg(long)]
        pair_token: Option<String>,
    },
    /// Stream live `NVD1` binary deltas over UDP + TCP/WebSocket and ingest remote `InputEvents`.
    Pair {
        /// Direct path to an already-built app cdylib exporting `nve_get_interface`.
        #[arg(long)]
        dylib: Option<PathBuf>,
        /// Workspace crate/package name of the cdylib to load (builds automatically if missing).
        #[arg(long)]
        package: Option<String>,
        /// Bind address (`0.0.0.0` allows LAN/phone clients).
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// UDP port for low-latency MTU-safe `NVD1` datagrams.
        #[arg(long, default_value_t = 8091)]
        udp_port: u16,
        /// TCP / RFC-6455 binary WebSocket port for framed `NVD1` streams.
        #[arg(long, default_value_t = 8092)]
        ws_port: u16,
        /// Optional pairing token secret (auto-generated when omitted).
        #[arg(long)]
        token: Option<String>,
        /// Initial logical viewport width in points.
        #[arg(long, default_value_t = 393.0)]
        width: f32,
        /// Initial logical viewport height in points.
        #[arg(long, default_value_t = 852.0)]
        height: f32,
        /// Target frame/poll cadence in Hz.
        #[arg(long, default_value_t = 60)]
        fps: u32,
    },
    /// Bi-directional React Native <-> NVE and multi-framework (Flutter, SwiftUI, Compose, Web) codegen.
    Codegen {
        #[command(subcommand)]
        action: CodegenAction,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => doctor::cmd_doctor(),
        Commands::Inspect => doctor::cmd_inspect(),
        Commands::Bench {
            rows,
            frames,
            trace,
        } => bench::cmd_bench(rows, frames, trace),
        Commands::Dev {
            package,
            watch,
            host,
            port,
            web,
            no_web,
            skia,
            pair,
            pair_udp_port,
            pair_ws_port,
            pair_token,
        } => dev::cmd_dev(DevOptions {
            package,
            watch,
            host,
            port,
            web,
            no_web,
            skia,
            pair,
            pair_udp_port,
            pair_ws_port,
            pair_token,
        }),
        Commands::Pair {
            dylib,
            package,
            host,
            udp_port,
            ws_port,
            token,
            width,
            height,
            fps,
        } => pair::cmd_pair(PairOptions {
            dylib,
            package,
            host,
            udp_port,
            ws_port,
            token,
            width,
            height,
            fps,
        }),
        Commands::Codegen { action } => codegen::cmd_codegen(action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_all_subcommands_defaults_and_flags() {
        assert!(matches!(
            Cli::try_parse_from(["nve", "doctor"]).unwrap().command,
            Commands::Doctor
        ));
        assert!(matches!(
            Cli::try_parse_from(["nve", "inspect"]).unwrap().command,
            Commands::Inspect
        ));

        match Cli::try_parse_from(["nve", "bench"]).unwrap().command {
            Commands::Bench {
                rows,
                frames,
                trace,
            } => {
                assert_eq!(rows, 1000);
                assert_eq!(frames, 50);
                assert!(trace.is_none());
            }
            _ => panic!("expected Bench"),
        }

        match Cli::try_parse_from([
            "nve",
            "bench",
            "--rows",
            "250",
            "--frames",
            "12",
            "--trace",
            "out.trace.json",
        ])
        .unwrap()
        .command
        {
            Commands::Bench {
                rows,
                frames,
                trace,
            } => {
                assert_eq!(rows, 250);
                assert_eq!(frames, 12);
                assert_eq!(trace, Some(PathBuf::from("out.trace.json")));
            }
            _ => panic!("expected Bench"),
        }

        match Cli::try_parse_from([
            "nve",
            "dev",
            "--package",
            "my-app",
            "--watch",
            "src",
            "--port",
            "9001",
            "--skia",
            "--pair",
            "--pair-udp-port",
            "9101",
            "--pair-ws-port",
            "9102",
            "--pair-token",
            "secret-dev-token",
        ])
        .unwrap()
        .command
        {
            Commands::Dev {
                package,
                watch,
                host,
                port,
                web,
                no_web,
                skia,
                pair,
                pair_udp_port,
                pair_ws_port,
                pair_token,
            } => {
                assert_eq!(package, "my-app");
                assert_eq!(watch, Some(PathBuf::from("src")));
                assert_eq!(host, "0.0.0.0");
                assert_eq!(port, 9001);
                assert!(!web);
                assert!(!no_web);
                assert!(skia);
                assert!(pair);
                assert_eq!(pair_udp_port, 9101);
                assert_eq!(pair_ws_port, 9102);
                assert_eq!(pair_token.as_deref(), Some("secret-dev-token"));
            }
            _ => panic!("expected Dev"),
        }

        match Cli::try_parse_from(["nve", "pair"]).unwrap().command {
            Commands::Pair {
                dylib,
                package,
                host,
                udp_port,
                ws_port,
                token,
                width,
                height,
                fps,
            } => {
                assert!(dylib.is_none());
                assert!(package.is_none());
                assert_eq!(host, "0.0.0.0");
                assert_eq!(udp_port, 8091);
                assert_eq!(ws_port, 8092);
                assert!(token.is_none());
                assert_eq!(width, 393.0);
                assert_eq!(height, 852.0);
                assert_eq!(fps, 60);
            }
            _ => panic!("expected Pair defaults"),
        }

        match Cli::try_parse_from([
            "nve",
            "pair",
            "--dylib",
            "target/debug/libnve_example_counter.dylib",
            "--host",
            "127.0.0.1",
            "--udp-port",
            "9091",
            "--ws-port",
            "9092",
            "--token",
            "pair-secret",
            "--width",
            "430",
            "--height",
            "932",
            "--fps",
            "120",
        ])
        .unwrap()
        .command
        {
            Commands::Pair {
                dylib,
                package,
                host,
                udp_port,
                ws_port,
                token,
                width,
                height,
                fps,
            } => {
                assert_eq!(
                    dylib,
                    Some(PathBuf::from("target/debug/libnve_example_counter.dylib"))
                );
                assert!(package.is_none());
                assert_eq!(host, "127.0.0.1");
                assert_eq!(udp_port, 9091);
                assert_eq!(ws_port, 9092);
                assert_eq!(token.as_deref(), Some("pair-secret"));
                assert_eq!(width, 430.0);
                assert_eq!(height, 932.0);
                assert_eq!(fps, 120);
            }
            _ => panic!("expected Pair custom flags"),
        }

        match Cli::try_parse_from(["nve", "codegen", "abi", "--check", "--out", "/tmp/abi"])
            .unwrap()
            .command
        {
            Commands::Codegen {
                action: CodegenAction::Abi { check, out },
            } => {
                assert!(check);
                assert_eq!(out, PathBuf::from("/tmp/abi"));
            }
            _ => panic!("expected Codegen Abi --check"),
        }

        match Cli::try_parse_from(["nve", "codegen", "hybrid", "specs/crypto.nve.rs"])
            .unwrap()
            .command
        {
            Commands::Codegen {
                action: CodegenAction::Hybrid { spec, target, out },
            } => {
                assert_eq!(spec, PathBuf::from("specs/crypto.nve.rs"));
                assert_eq!(target, "all");
                assert_eq!(out, PathBuf::from("generated/hybrid"));
            }
            _ => panic!("expected Codegen Hybrid defaults"),
        }

        // Invalid invocations must be rejected cleanly.
        assert!(Cli::try_parse_from(["nve"]).is_err());
        assert!(Cli::try_parse_from(["nve", "unknown"]).is_err());
        assert!(Cli::try_parse_from(["nve", "bench", "--rows", "not_a_num"]).is_err());
        assert!(Cli::try_parse_from(["nve", "dev", "--port", "99999"]).is_err());
        assert!(Cli::try_parse_from(["nve", "pair", "--udp-port", "99999"]).is_err());
        assert!(Cli::try_parse_from(["nve", "codegen", "import-rn"]).is_err());
        assert!(Cli::try_parse_from(["nve", "codegen", "hybrid"]).is_err());
    }
}
