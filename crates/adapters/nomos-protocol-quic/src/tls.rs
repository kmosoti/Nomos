//! Strict SHA-256 fingerprint-pinning `rustls` verifiers and QUIC configs.

use std::sync::Arc;

use nomos_protocol::{ProtocolError, ProtocolResult};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme};

use crate::cert::{PinnedIdentity, sha256_der_bytes};

/// Constant-time 32-byte equality check without unsafe code or external deps.
fn ct_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// `rustls` server certificate verifier rejecting any cert whose SHA-256 digest differs from `expected`.
#[derive(Debug)]
pub struct PinnedServerVerifier {
    expected_sha256: [u8; 32],
    supported: WebPkiSupportedAlgorithms,
}

impl PinnedServerVerifier {
    /// Creates a server verifier pinning `expected_sha256`.
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
        let actual = sha256_der_bytes(end_entity.as_ref());
        if ct_eq_32(&actual, &self.expected_sha256) {
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
    /// Creates a client certificate verifier pinned to `allowed_sha256` digests.
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
        let actual = sha256_der_bytes(end_entity.as_ref());
        if self.allowed_sha256.iter().any(|pin| ct_eq_32(&actual, pin)) {
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

/// Builds a Quinn server configuration presenting `identity` and optionally pinning client certs.
pub fn build_server_config(
    identity: &PinnedIdentity,
    allowed_client_pins: Option<Vec<[u8; 32]>>,
) -> ProtocolResult<quinn::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ProtocolError::Certificate(format!("TLS protocol config: {e}")))?;

    let crypto = if let Some(pins) = allowed_client_pins {
        builder
            .with_client_cert_verifier(Arc::new(PinnedClientVerifier::new(pins)))
            .with_single_cert(vec![identity.certificate_der()], identity.private_key_der())
    } else {
        builder
            .with_no_client_auth()
            .with_single_cert(vec![identity.certificate_der()], identity.private_key_der())
    }
    .map_err(|e| ProtocolError::Certificate(format!("server TLS cert: {e}")))?;

    let quic_crypto = QuicServerConfig::try_from(crypto)
        .map_err(|e| ProtocolError::Certificate(format!("QUIC server TLS config: {e}")))?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(quic_crypto)))
}

/// Builds a Quinn client configuration pinning the peer server certificate to `expected_sha256`.
pub fn build_client_config(
    client_identity: Option<&PinnedIdentity>,
    expected_sha256: [u8; 32],
) -> ProtocolResult<quinn::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ProtocolError::Certificate(format!("TLS protocol config: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerVerifier::new(expected_sha256)));

    let crypto = if let Some(id) = client_identity {
        builder
            .with_client_auth_cert(vec![id.certificate_der()], id.private_key_der())
            .map_err(|e| ProtocolError::Certificate(format!("client TLS cert: {e}")))?
    } else {
        builder.with_no_client_auth()
    };

    let quic_crypto = QuicClientConfig::try_from(crypto)
        .map_err(|e| ProtocolError::Certificate(format!("QUIC client TLS config: {e}")))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic_crypto)))
}
