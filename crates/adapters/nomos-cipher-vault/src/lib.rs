//! Cipher adapter for HashiCorp Vault ([ADR 0002](../../../docs/adr/0002-vault-cipher-adapter.md)).
//!
//! Runtime Vault transport awaits Phase 6 composition; the adapter currently
//! satisfies the [`nomos_cipher`] port contract by failing closed with
//! [`CipherError::UnsupportedBackend`].

use nomos_cipher::{
    CipherError, CipherPort, CipherRef, CipherResult, CipherVault, SecretMetadata, SecretPolicy,
    SecretsPosture, SyncPlan, TargetTransport, VaultStatus,
};
use nomos_core::cipher::Secret;
use nomos_core::resource::Digest;

/// Placeholder HashiCorp Vault cipher adapter wired to [`nomos_cipher`].
#[derive(Clone, Debug, Default)]
pub struct VaultCipherAdapter {
    address: Option<String>,
}

impl VaultCipherAdapter {
    /// Creates an adapter targeting an optional Vault endpoint URL.
    pub fn new(address: Option<String>) -> Self {
        Self { address }
    }

    /// Configured Vault endpoint URL, if any.
    pub fn address(&self) -> Option<&str> {
        self.address.as_deref()
    }

    fn unsupported(&self) -> CipherError {
        let target = self.address.as_deref().unwrap_or("unconfigured");
        CipherError::UnsupportedBackend(format!(
            "HashiCorp Vault transport (`{target}`) is not yet implemented"
        ))
    }
}

impl CipherVault for VaultCipherAdapter {
    fn init(&self) -> CipherResult<VaultStatus> {
        Err(self.unsupported())
    }

    fn status(&self, _policy: Option<&SecretPolicy>) -> CipherResult<VaultStatus> {
        Err(self.unsupported())
    }

    fn get(&self, _reference: &CipherRef) -> CipherResult<Secret<Vec<u8>>> {
        Err(self.unsupported())
    }

    fn set(
        &self,
        _reference: &CipherRef,
        _plaintext: &Secret<Vec<u8>>,
    ) -> CipherResult<SecretMetadata> {
        Err(self.unsupported())
    }

    fn delete(&self, _reference: &CipherRef) -> CipherResult<()> {
        Err(self.unsupported())
    }

    fn digest_secret(&self, plaintext: &Secret<Vec<u8>>) -> Digest {
        let _ = plaintext.expose();
        Digest::from_bytes([0u8; 32])
    }
}

impl CipherPort for VaultCipherAdapter {
    fn sync_policy(
        &self,
        _policy: &SecretPolicy,
        _transport: &dyn TargetTransport,
        _target_filter: Option<&str>,
        _confirm_apply: bool,
    ) -> CipherResult<SyncPlan> {
        Err(self.unsupported())
    }

    fn posture(
        &self,
        _policy: &SecretPolicy,
        _transport: &dyn TargetTransport,
        _target: Option<&nomos_cipher::NodeTarget>,
    ) -> CipherResult<SecretsPosture> {
        Err(self.unsupported())
    }
}
