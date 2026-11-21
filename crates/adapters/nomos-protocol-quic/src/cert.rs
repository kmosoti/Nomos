//! Ephemeral and persisted Ed25519 certificate generation and SHA-256 fingerprinting.

use nomos_protocol::{ProtocolError, ProtocolResult};
use rcgen::{CertificateParams, KeyPair, PKCS_ED25519};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest as _, Sha256};

/// Self-signed Ed25519 certificate and PKCS#8 private key bytes with cached SHA-256 digest.
#[derive(Clone)]
pub struct PinnedIdentity {
    cert_der: Vec<u8>,
    key_pkcs8_der: Vec<u8>,
    sha256_bytes: [u8; 32],
    fingerprint_hex: String,
}

impl Drop for PinnedIdentity {
    fn drop(&mut self) {
        self.key_pkcs8_der.fill(0);
    }
}

impl core::fmt::Debug for PinnedIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PinnedIdentity")
            .field("fingerprint_hex", &self.fingerprint_hex)
            .finish_non_exhaustive()
    }
}

impl PinnedIdentity {
    /// Generates a fresh Ed25519 self-signed certificate covering `subject_alt_names`.
    pub fn generate(subject_alt_names: &[&str]) -> ProtocolResult<Self> {
        let sans: Vec<String> = subject_alt_names.iter().map(|s| (*s).to_owned()).collect();
        let params = CertificateParams::new(sans)
            .map_err(|e| ProtocolError::Certificate(format!("certificate params: {e}")))?;
        let key_pair = KeyPair::generate_for(&PKCS_ED25519)
            .map_err(|e| ProtocolError::Certificate(format!("Ed25519 keygen: {e}")))?;
        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| ProtocolError::Certificate(format!("self-sign cert: {e}")))?;

        let cert_der = cert.der().as_ref().to_vec();
        let key_pkcs8_der = key_pair.serialize_der();
        let sha256_bytes = sha256_der_bytes(&cert_der);
        let fingerprint_hex = hex::encode(sha256_bytes);

        Ok(Self {
            cert_der,
            key_pkcs8_der,
            sha256_bytes,
            fingerprint_hex,
        })
    }

    /// DER-encoded X.509 certificate for TLS presentation.
    pub fn certificate_der(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    /// PKCS#8 private key handle for `rustls`.
    pub fn private_key_der(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_pkcs8_der.clone()))
    }

    /// Raw 32-byte SHA-256 certificate fingerprint.
    pub const fn sha256_bytes(&self) -> [u8; 32] {
        self.sha256_bytes
    }

    /// Lowercase 64-character hex SHA-256 fingerprint.
    pub fn fingerprint_hex(&self) -> &str {
        &self.fingerprint_hex
    }
}

/// Computes raw 32-byte SHA-256 digest of a DER-encoded X.509 certificate.
pub fn sha256_der_bytes(cert_der: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(cert_der);
    hasher.finalize().into()
}

/// Computes lowercase hex SHA-256 fingerprint of a DER-encoded X.509 certificate.
pub fn sha256_der_hex(cert_der: &[u8]) -> String {
    hex::encode(sha256_der_bytes(cert_der))
}

/// Parses a 64-character hex SHA-256 fingerprint (ignoring optional colons).
pub fn parse_sha256_hex(text: &str) -> ProtocolResult<[u8; 32]> {
    let cleaned: String = text.chars().filter(|c| *c != ':').collect();
    let decoded = hex::decode(cleaned.trim())
        .map_err(|e| ProtocolError::Certificate(format!("invalid fingerprint hex: {e}")))?;
    decoded
        .try_into()
        .map_err(|_| ProtocolError::Certificate("SHA-256 fingerprint must be 32 bytes".into()))
}
