//! Ephemeral and persisted Ed25519 certificate generation, SPKI extraction, and SHA-256 pinning.

use nomos_protocol::{ProtocolError, ProtocolResult};
use rcgen::{CertificateParams, KeyPair, PKCS_ED25519};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest as _, Sha256};

/// Self-signed Ed25519 certificate and PKCS#8 private key bytes with cached cert & SPKI SHA-256.
#[derive(Clone)]
pub struct PinnedIdentity {
    cert_der: Vec<u8>,
    key_pkcs8_der: Vec<u8>,
    sha256_bytes: [u8; 32],
    fingerprint_hex: String,
    spki_sha256_bytes: [u8; 32],
    spki_fingerprint_hex: String,
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
            .field("spki_fingerprint_hex", &self.spki_fingerprint_hex)
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
        let spki_sha256_bytes = spki_sha256_bytes(&cert_der)?;
        let spki_fingerprint_hex = hex::encode(spki_sha256_bytes);

        Ok(Self {
            cert_der,
            key_pkcs8_der,
            sha256_bytes,
            fingerprint_hex,
            spki_sha256_bytes,
            spki_fingerprint_hex,
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

    /// Raw 32-byte SHA-256 full-certificate fingerprint.
    pub const fn sha256_bytes(&self) -> [u8; 32] {
        self.sha256_bytes
    }

    /// Lowercase 64-character hex SHA-256 certificate fingerprint.
    pub fn fingerprint_hex(&self) -> &str {
        &self.fingerprint_hex
    }

    /// Raw 32-byte SHA-256 SubjectPublicKeyInfo (SPKI) fingerprint.
    pub const fn spki_sha256_bytes(&self) -> [u8; 32] {
        self.spki_sha256_bytes
    }

    /// Lowercase 64-character hex SHA-256 SPKI fingerprint.
    pub fn spki_fingerprint_hex(&self) -> &str {
        &self.spki_fingerprint_hex
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

/// Extracts the DER-encoded `SubjectPublicKeyInfo` (SPKI) TLV from an X.509 certificate.
pub fn extract_spki_der(cert_der: &[u8]) -> ProtocolResult<&[u8]> {
    let (_, cert_seq, _) = read_der_tlv(cert_der, 0x30)?;
    let (_, mut tbs, _) = read_der_tlv(cert_seq, 0x30)?;
    // Optional explicit [0] version tag (0xA0)
    if tbs.first() == Some(&0xA0) {
        let (_, _, rest) = read_der_tlv(tbs, 0xA0)?;
        tbs = rest;
    }
    // Skip serialNumber (0x02), signature (0x30), issuer (0x30), validity (0x30), subject (0x30).
    for expected_tag in [0x02u8, 0x30, 0x30, 0x30, 0x30] {
        let (_, _, rest) = read_der_tlv(tbs, expected_tag)?;
        tbs = rest;
    }
    let (spki_tlv, _, _) = read_der_tlv(tbs, 0x30)?;
    Ok(spki_tlv)
}

/// Computes the 32-byte SHA-256 digest of the certificate's `SubjectPublicKeyInfo`.
pub fn spki_sha256_bytes(cert_der: &[u8]) -> ProtocolResult<[u8; 32]> {
    let spki = extract_spki_der(cert_der)?;
    Ok(sha256_der_bytes(spki))
}

/// Computes lowercase hex SHA-256 SPKI fingerprint of a DER-encoded X.509 certificate.
pub fn spki_sha256_hex(cert_der: &[u8]) -> ProtocolResult<String> {
    Ok(hex::encode(spki_sha256_bytes(cert_der)?))
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

fn read_der_tlv(input: &[u8], expected_tag: u8) -> ProtocolResult<(&[u8], &[u8], &[u8])> {
    if input.len() < 2 || input[0] != expected_tag {
        return Err(ProtocolError::Certificate(format!(
            "unexpected ASN.1 DER tag (expected 0x{expected_tag:02x})"
        )));
    }
    let (len, hdr_len) = if (input[1] & 0x80) == 0 {
        (input[1] as usize, 2usize)
    } else {
        let n_octets = (input[1] & 0x7f) as usize;
        if n_octets == 0 || n_octets > 4 || input.len() < 2 + n_octets {
            return Err(ProtocolError::Certificate(
                "invalid ASN.1 DER length".into(),
            ));
        }
        let mut len = 0usize;
        for &b in &input[2..2 + n_octets] {
            len = (len << 8) | (b as usize);
        }
        (len, 2 + n_octets)
    };
    let total = hdr_len
        .checked_add(len)
        .ok_or_else(|| ProtocolError::Certificate("ASN.1 DER length overflow".into()))?;
    if total > input.len() {
        return Err(ProtocolError::Certificate(
            "truncated ASN.1 DER element".into(),
        ));
    }
    Ok((&input[..total], &input[hdr_len..total], &input[total..]))
}
