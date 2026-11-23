//! Constant-time SPKI & certificate SHA-256 pinning `rustls` verifiers and mandatory mTLS configs.

use std::sync::Arc;

use nomos_protocol::{ProtocolError, ProtocolResult};
use quinn::IdleTimeout;
use quinn::VarInt;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme};

use crate::cert::{PinnedIdentity, sha256_der_bytes, spki_sha256_bytes};
use crate::limits::{DEFAULT_IDLE_TIMEOUT, DEFAULT_QUIC_MAX_BIDI_STREAMS};

/// Constant-time 32-byte equality check resistant to short-circuit timing leaks.
fn ct_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= core::hint::black_box(x ^ y);
    }
    core::hint::black_box(diff) == 0
}

/// Constant-time match checking both SPKI SHA-256 and full certificate DER SHA-256
/// without short-circuit timing leaks.
fn ct_matches_pin(cert_der: &[u8], expected_pin: &[u8; 32]) -> bool {
    let cert_digest = sha256_der_bytes(cert_der);
    let cert_ok = u8::from(ct_eq_32(&cert_digest, expected_pin));
    let spki_ok = match spki_sha256_bytes(cert_der) {
        Ok(spki_digest) => u8::from(ct_eq_32(&spki_digest, expected_pin)),
        Err(_) => 0,
    };
    (cert_ok | spki_ok) != 0
}

/// `rustls` server verifier rejecting any peer whose SPKI/cert SHA-256 differs from `expected`.
#[derive(Debug)]
pub struct PinnedServerVerifier {
    expected_sha256: [u8; 32],
    supported: WebPkiSupportedAlgorithms,
}

impl PinnedServerVerifier {
    /// Creates a server verifier pinning `expected_sha256` (SPKI or full-cert SHA-256).
    pub fn new(expected_sha256: [u8; 32]) -> Self {
        Self {
            expected_sha256,
            supported: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if ct_matches_pin(end_entity.as_ref(), &self.expected_sha256) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

/// `rustls` mutual-TLS client verifier accepting only client certs matching `allowed_sha256`.
#[derive(Debug)]
pub struct PinnedClientVerifier {
    allowed_sha256: Vec<[u8; 32]>,
    supported: WebPkiSupportedAlgorithms,
}

impl PinnedClientVerifier {
    /// Creates a client certificate verifier pinned to `allowed_sha256` (SPKI or cert SHA-256).
    pub fn new(allowed_sha256: Vec<[u8; 32]>) -> Self {
        Self {
            allowed_sha256,
            supported: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }
    }
}

impl ClientCertVerifier for PinnedClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let mut matched = 0u8;
        for pin in &self.allowed_sha256 {
            matched |= u8::from(ct_matches_pin(end_entity.as_ref(), pin));
        }
        if matched != 0 {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

/// Builds a mandatory-mTLS Quinn server configuration presenting `identity` and
/// requiring every peer client certificate to match `allowed_client_pins`.
pub fn build_server_config(
    identity: &PinnedIdentity,
    allowed_client_pins: Vec<[u8; 32]>,
) -> ProtocolResult<quinn::ServerConfig> {
    if allowed_client_pins.is_empty() {
        return Err(ProtocolError::Certificate(
            "mandatory mTLS server requires at least one allowed_client_pin".into(),
        ));
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let crypto = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ProtocolError::Certificate(format!("TLS protocol config: {e}")))?
        .with_client_cert_verifier(Arc::new(PinnedClientVerifier::new(allowed_client_pins)))
        .with_single_cert(vec![identity.certificate_der()], identity.private_key_der())
        .map_err(|e| ProtocolError::Certificate(format!("server TLS cert: {e}")))?;

    let quic_crypto = QuicServerConfig::try_from(crypto)
        .map_err(|e| ProtocolError::Certificate(format!("QUIC server TLS config: {e}")))?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    let mut transport = quinn::TransportConfig::default();
    if let Ok(idle) = IdleTimeout::try_from(DEFAULT_IDLE_TIMEOUT) {
        transport.max_idle_timeout(Some(idle));
    }
    transport.max_concurrent_bidi_streams(VarInt::from_u32(DEFAULT_QUIC_MAX_BIDI_STREAMS));
    transport.max_concurrent_uni_streams(VarInt::from_u32(0));
    server_config.transport_config(Arc::new(transport));
    Ok(server_config)
}

/// Builds a mutual-TLS Quinn client configuration presenting `client_identity` and
/// pinning the peer server certificate/SPKI to `expected_sha256`.
pub fn build_client_config(
    client_identity: &PinnedIdentity,
    expected_sha256: [u8; 32],
) -> ProtocolResult<quinn::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let crypto = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ProtocolError::Certificate(format!("TLS protocol config: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerVerifier::new(expected_sha256)))
        .with_client_auth_cert(
            vec![client_identity.certificate_der()],
            client_identity.private_key_der(),
        )
        .map_err(|e| ProtocolError::Certificate(format!("client TLS cert: {e}")))?;

    let quic_crypto = QuicClientConfig::try_from(crypto)
        .map_err(|e| ProtocolError::Certificate(format!("QUIC client TLS config: {e}")))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic_crypto)))
}
