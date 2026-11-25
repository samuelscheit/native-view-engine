//! 13-day ECDSA P-256 self-signed TLS dev certificates, SHA-256 DER pinning, and dual-ALPN
//! (`h3` + `nve-wire-2`) QUIC/WSS `rustls` configuration.

use std::sync::{Arc, Once};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use subtle::ConstantTimeEq;

use nve_wire_core::{format_cert_hash_hex, DEFAULT_WIRE_QUIC_ALPN};

use crate::NveWireTransportError;

/// HTTP/3 WebTransport ALPN identifier.
pub const WEBTRANSPORT_ALPN: &[u8] = b"h3";

/// Native raw QUIC ALPN identifier (`"nve-wire-2"`).
pub const NVE_WIRE_QUIC_ALPN_BYTES: &[u8] = DEFAULT_WIRE_QUIC_ALPN.as_bytes();

/// Validity window for browser `WebTransport` `serverCertificateHashes` (`<= 14 days` mandated by
/// Chrome, Firefox, and Safari 26.4+; we use 13 days for safe clock-skew margin).
pub const DEV_CERT_VALIDITY_DAYS: u64 = 13;

static CRYPTO_INIT: Once = Once::new();

/// Ensures the process-wide `rustls` `ring` CryptoProvider is installed once.
pub fn ensure_crypto_provider() -> Arc<CryptoProvider> {
    CRYPTO_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()))
}

/// Generated 13-day ECDSA P-256 TLS certificate with precomputed SHA-256 DER fingerprint.
#[derive(Clone)]
pub struct NveDevCertificate {
    pub cert_der: CertificateDer<'static>,
    pub key_der: Vec<u8>,
    pub sha256: [u8; 32],
}

impl NveDevCertificate {
    /// Generates a fresh 13-day ECDSA P-256 self-signed certificate covering `localhost`,
    /// `127.0.0.1`, `::1`, and any additional `extra_sans` hostnames/IPs.
    pub fn generate(extra_sans: &[String]) -> Result<Self, NveWireTransportError> {
        let provider = ensure_crypto_provider();
        let mut sans = vec![
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "::1".to_owned(),
        ];
        for san in extra_sans {
            let trimmed = san.trim();
            if !trimmed.is_empty()
                && trimmed != "0.0.0.0"
                && trimmed != "::"
                && !sans.iter().any(|s| s == trimmed)
            {
                sans.push(trimmed.to_owned());
            }
        }

        let mut params = CertificateParams::new(sans)
            .map_err(|e| NveWireTransportError::Tls(format!("rcgen params failed: {e}")))?;
        // Backdate slightly (-1h) for clock skew, expire strictly within 13 days (< 14-day W3C cap).
        let now = SystemTime::now();
        params.not_before = (now - Duration::from_secs(3600)).into();
        params.not_after = (now + Duration::from_secs(DEV_CERT_VALIDITY_DAYS * 86_400)).into();

        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
            .map_err(|e| NveWireTransportError::Tls(format!("ECDSA P-256 keygen failed: {e}")))?;
        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| NveWireTransportError::Tls(format!("self-signed cert failed: {e}")))?;

        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = key_pair.serialize_der();
        let digest = web_transport_quinn::crypto::sha256(&provider, &cert_der)
            .map_err(|e| NveWireTransportError::Tls(format!("cert SHA-256 failed: {e}")))?;
        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(digest.as_ref());

        Ok(Self {
            cert_der,
            key_der,
            sha256,
        })
    }

    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()))
    }

    pub fn sha256_hex(&self) -> String {
        format_cert_hash_hex(&self.sha256)
    }

    pub fn sha256_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.sha256)
    }
}

/// Builds a QUIC `TransportConfig` with RFC 9221 unreliable datagrams enabled.
pub fn build_quic_transport_config() -> Arc<quinn::TransportConfig> {
    let mut cfg = quinn::TransportConfig::default();
    cfg.datagram_receive_buffer_size(Some(65_536));
    cfg.datagram_send_buffer_size(65_536);
    cfg.keep_alive_interval(Some(Duration::from_secs(5)));
    if let Ok(idle) = Duration::from_secs(30).try_into() {
        cfg.max_idle_timeout(Some(idle));
    }
    Arc::new(cfg)
}

/// Builds a dual-ALPN (`h3` WebTransport + `nve-wire-2` raw QUIC) `quinn::ServerConfig`.
pub fn build_dual_alpn_quic_server_config(
    cert: &NveDevCertificate,
) -> Result<quinn::ServerConfig, NveWireTransportError> {
    let provider = ensure_crypto_provider();
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NveWireTransportError::Tls(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert_der.clone()], cert.private_key())
        .map_err(|e| NveWireTransportError::Tls(e.to_string()))?;
    tls.alpn_protocols = vec![
        WEBTRANSPORT_ALPN.to_vec(),
        NVE_WIRE_QUIC_ALPN_BYTES.to_vec(),
    ];

    let quic_crypto = web_transport_quinn::crypto::QuicServerConfig::try_from(tls)
        .map_err(|e| NveWireTransportError::Tls(format!("QUIC server crypto failed: {e}")))?;
    let mut server_cfg = web_transport_quinn::crypto::server_config(Arc::new(quic_crypto))
        .map_err(|e| NveWireTransportError::Tls(format!("QUIC server config failed: {e}")))?;
    server_cfg.transport_config(build_quic_transport_config());
    Ok(server_cfg)
}

/// Builds a TLS 1.3 `rustls::ServerConfig` for `wss://` WebSocket fallback.
pub fn build_wss_server_config(
    cert: &NveDevCertificate,
) -> Result<Arc<rustls::ServerConfig>, NveWireTransportError> {
    let provider = ensure_crypto_provider();
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| NveWireTransportError::Tls(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert_der.clone()], cert.private_key())
        .map_err(|e| NveWireTransportError::Tls(e.to_string()))?;
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(tls))
}

/// Builds a `rustls::ClientConfig` pinned to a specific leaf certificate SHA-256 digest.
pub fn build_pinned_client_tls_config(
    expected_sha256: [u8; 32],
    alpn_protocols: Vec<Vec<u8>>,
) -> Result<rustls::ClientConfig, NveWireTransportError> {
    let provider = ensure_crypto_provider();
    let verifier = Arc::new(PinnedCertSha256Verifier {
        provider: provider.clone(),
        expected_sha256,
    });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NveWireTransportError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    tls.alpn_protocols = alpn_protocols;
    Ok(tls)
}

/// Builds a raw QUIC (`nve-wire-2`) `quinn::ClientConfig` pinned to `expected_sha256`.
pub fn build_quic_client_config(
    expected_sha256: [u8; 32],
) -> Result<quinn::ClientConfig, NveWireTransportError> {
    let tls = build_pinned_client_tls_config(
        expected_sha256,
        vec![NVE_WIRE_QUIC_ALPN_BYTES.to_vec()],
    )?;
    let quic_crypto = web_transport_quinn::crypto::QuicClientConfig::try_from(tls)
        .map_err(|e| NveWireTransportError::Tls(format!("QUIC client crypto failed: {e}")))?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(quic_crypto));
    cfg.transport_config(build_quic_transport_config());
    Ok(cfg)
}

#[derive(Debug)]
struct PinnedCertSha256Verifier {
    provider: Arc<CryptoProvider>,
    expected_sha256: [u8; 32],
}

impl ServerCertVerifier for PinnedCertSha256Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let digest = web_transport_quinn::crypto::sha256(&self.provider, end_entity)?;
        if bool::from(digest.as_ref().ct_eq(&self.expected_sha256)) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(RustlsError::InvalidCertificate(
                rustls::CertificateError::BadSignature,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
