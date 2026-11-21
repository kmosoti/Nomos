//! Sealing primitives for `vault.key` TPM2 protection and `systemd-creds` delivery.

use std::fmt;
use std::io::Write;
use std::process::{Command, Stdio};

use nomos_cipher::{CipherError, CipherResult, validate_ascii_identifier};
use zeroize::Zeroize;

/// Storage mode governing how `vault.key` is persisted on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VaultKeyStorage {
    /// Production default: seals `vault.key` via `systemd-creds encrypt --with-key=tpm2`
    /// and refuses to load unsealed plaintext keys.
    #[default]
    Tpm2Sealed,
    /// Development/CI opt-in (`--allow-plaintext-key`): stores `vault.key` with
    /// `0o600` permissions without TPM2 sealing.
    PlaintextDev,
}

/// Cryptographic backend mode for vault entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VaultCipherMode {
    /// Post-quantum hybrid `ML-KEM-768 + X25519` X-Wing-style combiner.
    #[default]
    HybridMlKem768X25519,
    /// Classical `age` X25519 recipient/identity format.
    ClassicalX25519,
}

/// Abstraction over `systemd-creds` for vault key sealing and credential encryption.
pub trait CredSealer: Send + Sync + fmt::Debug {
    /// Seals private vault key bytes to TPM2 (`systemd-creds encrypt --with-key=tpm2`).
    fn seal_tpm2(&self, plaintext: &[u8]) -> CipherResult<Vec<u8>>;

    /// Unseals a TPM2-sealed vault key (`systemd-creds decrypt`).
    fn unseal_tpm2(&self, sealed: &[u8]) -> CipherResult<Vec<u8>>;

    /// Encrypts a secret payload for `DeliveryMode::SystemdCred`
    /// (`systemd-creds encrypt --name=<name> --with-key=auto`).
    fn encrypt_credential(&self, name: &str, plaintext: &[u8]) -> CipherResult<Vec<u8>>;
}

/// Production [`CredSealer`] delegating strictly over `stdin`/`stdout` to `systemd-creds`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemdCredsSealer;

impl SystemdCredsSealer {
    /// Returns the exact argv passed to `systemd-creds` when sealing a vault key.
    pub fn seal_tpm2_args() -> &'static [&'static str] {
        &["encrypt", "--with-key=tpm2", "-", "-"]
    }

    /// Returns the exact argv passed to `systemd-creds` when unsealing a vault key.
    pub fn unseal_tpm2_args() -> &'static [&'static str] {
        &["decrypt", "-", "-"]
    }

    /// Returns the exact argv passed to `systemd-creds` for `DeliveryMode::SystemdCred`.
    pub fn encrypt_credential_args(name: &str) -> CipherResult<Vec<String>> {
        validate_ascii_identifier("credential name", name)?;
        Ok(vec![
            "encrypt".to_owned(),
            format!("--name={name}"),
            "--with-key=auto".to_owned(),
            "-".to_owned(),
            "-".to_owned(),
        ])
    }

    fn run_systemd_creds(args: &[String], input: &[u8]) -> CipherResult<Vec<u8>> {
        let mut child = Command::new("systemd-creds")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| CipherError::VtpmUnavailable(format!("spawn systemd-creds: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input).map_err(|e| {
                CipherError::VtpmUnavailable(format!("write systemd-creds stdin: {e}"))
            })?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| CipherError::VtpmUnavailable(format!("wait systemd-creds: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(CipherError::VtpmUnavailable(format!(
                "systemd-creds exited {}: {stderr}",
                out.status
            )));
        }
        Ok(out.stdout)
    }
}

impl CredSealer for SystemdCredsSealer {
    fn seal_tpm2(&self, plaintext: &[u8]) -> CipherResult<Vec<u8>> {
        let args: Vec<String> = Self::seal_tpm2_args()
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        Self::run_systemd_creds(&args, plaintext)
    }

    fn unseal_tpm2(&self, sealed: &[u8]) -> CipherResult<Vec<u8>> {
        let args: Vec<String> = Self::unseal_tpm2_args()
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        Self::run_systemd_creds(&args, sealed)
    }

    fn encrypt_credential(&self, name: &str, plaintext: &[u8]) -> CipherResult<Vec<u8>> {
        let args = Self::encrypt_credential_args(name)?;
        Self::run_systemd_creds(&args, plaintext)
    }
}

const MOCK_TPM2_MAGIC: &[u8; 16] = b"NOMOS-MOCK-TPM2\0";
const MOCK_CRED_MAGIC: &[u8; 16] = b"NOMOS-MOCK-CRED\0";

/// Deterministic in-memory [`CredSealer`] for CI and unit/integration verification
/// where hardware TPM2 is unavailable.
#[derive(Clone, Debug)]
pub struct MockCredSealer {
    key: [u8; 32],
}

impl Default for MockCredSealer {
    fn default() -> Self {
        Self::new(*b"nomos-test-tpm2-mock-sealer-key!")
    }
}

impl MockCredSealer {
    /// Creates a deterministic mock sealer keyed by `key`.
    pub const fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    fn xor_stream(&self, domain: &[u8], data: &[u8]) -> Vec<u8> {
        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        hasher.update(domain);
        let mut xof = hasher.finalize_xof();
        let mut mask = vec![0u8; data.len()];
        xof.fill(&mut mask);
        let out: Vec<u8> = data.iter().zip(mask.iter()).map(|(a, b)| a ^ b).collect();
        mask.zeroize();
        out
    }
}

impl CredSealer for MockCredSealer {
    fn seal_tpm2(&self, plaintext: &[u8]) -> CipherResult<Vec<u8>> {
        let body = self.xor_stream(b"tpm2-seal", plaintext);
        let mut out = Vec::with_capacity(MOCK_TPM2_MAGIC.len() + body.len());
        out.extend_from_slice(MOCK_TPM2_MAGIC);
        out.extend_from_slice(&body);
        Ok(out)
    }

    fn unseal_tpm2(&self, sealed: &[u8]) -> CipherResult<Vec<u8>> {
        let Some(body) = sealed.strip_prefix(MOCK_TPM2_MAGIC) else {
            return Err(CipherError::Crypto(
                "unsealed or corrupted vault.key cannot be unsealed by TPM2 sealer".into(),
            ));
        };
        Ok(self.xor_stream(b"tpm2-seal", body))
    }

    fn encrypt_credential(&self, name: &str, plaintext: &[u8]) -> CipherResult<Vec<u8>> {
        validate_ascii_identifier("credential name", name)?;
        let domain = format!("systemd-cred:{name}");
        let body = self.xor_stream(domain.as_bytes(), plaintext);
        let mut out = Vec::with_capacity(MOCK_CRED_MAGIC.len() + name.len() + 1 + body.len());
        out.extend_from_slice(MOCK_CRED_MAGIC);
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(&body);
        Ok(out)
    }
}
