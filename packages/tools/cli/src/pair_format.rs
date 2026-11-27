//! URI formatting, token generation, dylib resolution, and telemetry helpers for `nve pair`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;

use nve_abi::{NveMapEntry, NveValue};
use nve_wire::{format_wire_uri, generate_bearer_token, NveWireServerMetrics};

use crate::dev::{dylib_path_for_package_in, resolve_cargo_target_dir};

pub(crate) const TELEMETRY_PUSH_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) fn generate_pairing_token() -> String {
    generate_bearer_token()
}

pub(crate) fn resolve_or_generate_token(token: Option<String>) -> String {
    match token {
        Some(t) if !t.trim().is_empty() => t.trim().to_owned(),
        _ => generate_pairing_token(),
    }
}

pub(crate) fn format_bind_addr(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub(crate) fn display_host_for_uri(host: &str) -> &str {
    if host == "0.0.0.0" || host == "::" || host == "[::]" {
        "127.0.0.1"
    } else {
        host
    }
}

fn format_url_host(host: &str) -> String {
    let disp = display_host_for_uri(host);
    if disp.contains(':') && !disp.starts_with('[') {
        format!("[{disp}]")
    } else {
        disp.to_owned()
    }
}

pub(crate) fn format_ws_url(host: &str, ws_port: u16) -> String {
    format!("ws://{}:{ws_port}", format_url_host(host))
}

pub(crate) fn format_pairing_uri(
    host: &str,
    quic_port: u16,
    wss_port: u16,
    token: &str,
    cert_sha256: Option<&[u8; 32]>,
    width: f32,
    height: f32,
) -> String {
    let h = display_host_for_uri(host);
    format_wire_uri(
        h,
        quic_port,
        wss_port,
        token,
        cert_sha256,
        width.round() as u32,
        height.round() as u32,
    )
}

pub(crate) fn format_browser_pair_query(
    host: &str,
    ws_port: u16,
    token: &str,
    cert_hash_hex: Option<&str>,
) -> String {
    match cert_hash_hex {
        Some(h) if !h.is_empty() => {
            format!(
                "?pair={}&token={token}&cert_hash={h}",
                format_ws_url(host, ws_port)
            )
        }
        _ => format!("?pair={}&token={token}", format_ws_url(host, ws_port)),
    }
}

pub(crate) fn format_pair_ready_line(
    host: &str,
    quic_addr: SocketAddr,
    wss_addr: SocketAddr,
    token: &str,
    cert_hash_hex: &str,
) -> String {
    let ws_url = format_ws_url(host, wss_addr.port());
    format!(
        "NVE_PAIR_READY ws={ws_url} token={token} udp={quic_addr} tcp={wss_addr} cert_hash={cert_hash_hex}"
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn format_pair_banner(
    dylib_path: &Path,
    host: &str,
    quic_addr: SocketAddr,
    wss_addr: SocketAddr,
    token: &str,
    cert_sha256: &[u8; 32],
    cert_hash_hex: &str,
    width: f32,
    height: f32,
) -> String {
    let url_host = format_url_host(host);
    let uri = format_pairing_uri(
        host,
        quic_addr.port(),
        wss_addr.port(),
        token,
        Some(cert_sha256),
        width,
        height,
    );
    let browser_query = format_browser_pair_query(host, wss_addr.port(), token, Some(cert_hash_hex));
    format!(
        "nve pair — Unified QUIC/WebTransport + WSS Device Pairing (NveWireFrame v2)\n  dylib:     {}\n  quic/h3:   https://{url_host}:{}/nve-wire (ALPN: h3, nve-wire-2; bound {quic_addr})\n  wss/ws:    wss://{url_host}:{}/nve-wire (bound {wss_addr})\n  cert_hash: {cert_hash_hex} (13-day ECDSA P-256)\n  token:     {token}\n  uri:       {uri}\n  browser:   {browser_query}\n",
        dylib_path.display(),
        quic_addr.port(),
        wss_addr.port(),
    )
}

pub(crate) fn resolve_dylib_path(
    dylib: Option<PathBuf>,
    package: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(path) = dylib {
        return if path.exists() {
            Ok(path)
        } else {
            Err(format!("specified dylib does not exist: {}", path.display()))
        };
    }
    let pkg = package.unwrap_or("nve-studio-workspace");
    let target_dir = resolve_cargo_target_dir().unwrap_or_else(|_| PathBuf::from("target"));
    let path = dylib_path_for_package_in(&target_dir, pkg);
    if !path.exists() {
        println!("[pair] building {pkg} -> {}...", path.display());
        let status = ProcessCommand::new("cargo")
            .args(["build", "-p", pkg])
            .status()
            .map_err(|e| format!("failed to invoke cargo build -p {pkg}: {e}"))?;
        if !status.success() {
            return Err(format!("cargo build -p {pkg} failed"));
        }
    }
    if path.exists() {
        Ok(path)
    } else {
        Err(format!(
            "expected cdylib at {} after building {pkg}",
            path.display()
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncTelemetrySnapshot {
    pub transport: &'static str,
    pub peers: u16,
    pub kbps_tenths: u32,
    pub compression_tenths: u16,
    pub rtt_ms_tenths: u32,
}

impl SyncTelemetrySnapshot {
    pub(crate) fn from_server(
        has_quic: bool,
        m: &NveWireServerMetrics,
        elapsed: Duration,
    ) -> Self {
        let peers = m.connected_peers.min(u16::MAX as usize) as u16;
        let transport = if has_quic {
            "lan-quic"
        } else if peers > 0 {
            "relay-wss"
        } else {
            "offline"
        };
        let secs = elapsed.as_secs_f64().max(0.05);
        let kbps = ((m.wire_bytes_tx + m.wire_bytes_rx) as f64 / 1024.0) / secs;
        let kbps_tenths = if kbps.is_finite() {
            (kbps * 10.0).round() as u32
        } else {
            0
        };
        Self {
            transport,
            peers,
            kbps_tenths,
            compression_tenths: 10,
            rtt_ms_tenths: 0,
        }
    }

    pub(crate) fn as_prop_entries(&self) -> [NveMapEntry; 5] {
        [
            NveMapEntry::new("syncTransport", NveValue::from_str(self.transport)),
            NveMapEntry::new("syncPeers", NveValue::i64(i64::from(self.peers))),
            NveMapEntry::new("syncKbps", NveValue::f64(f64::from(self.kbps_tenths) / 10.0)),
            NveMapEntry::new(
                "syncCompression",
                NveValue::f64(f64::from(self.compression_tenths) / 10.0),
            ),
            NveMapEntry::new(
                "syncRttMs",
                NveValue::f64(f64::from(self.rtt_ms_tenths) / 10.0),
            ),
        ]
    }
}
