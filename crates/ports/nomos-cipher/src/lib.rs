//! Cipher port: resolution, policy, and posture contracts for protected secrets.
//!
//! Plaintext secret material crosses this boundary strictly inside
//! [`nomos_core::cipher::Secret`] (invariant N8). Non-secret metadata uses the
//! types in [`policy`] and [`posture`].

pub mod policy;
pub mod posture;

use core::fmt;

use nomos_core::cipher::Secret;
use nomos_core::resource::Digest;

pub use policy::{
    CipherRef, DeliveryMode, NodeTarget, SecretPolicy, SecretPolicyEntry,
    validate_ascii_identifier, validate_host_address, validate_safe_path, validate_secret_key,
};
pub use posture::{
    RemoteSecretState, SecretMetadata, SecretsPosture, SyncAction, SyncActionKind, SyncPlan,
    VaultStatus, VtpmPosture,
};

/// Result alias for cipher port operations.
pub type CipherResult<T> = Result<T, CipherError>;

/// Errors returned by cipher vault, policy, and transport operations.
///
/// Error values carry identifiers and diagnostics only and never embed secret
/// plaintext or private key material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CipherError {
    /// Vault directory or identity file has not been initialized.
    NotInitialized(String),
    /// Vault directory is already initialized and would be overwritten.
    AlreadyInitialized(String),
    /// Named secret entry does not exist in the vault.
    NotFound(String),
    /// Secret key failed path-traversal or character allowlist validation.
    InvalidKey(String),
    /// Secret policy or target specification failed validation.
    InvalidPolicy(String),
    /// Cryptographic seal, unseal, or key parsing failure.
    Crypto(String),
    /// Vault filesystem or staging I/O failure.
    Io(String),
    /// Remote delivery transport failure.
    Transport(String),
    /// Local or remote vTPM capability check reported unavailable or failed.
    VtpmUnavailable(String),
    /// Requested backend technology is not implemented by this adapter.
    UnsupportedBackend(String),
}

impl fmt::Display for CipherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CipherError::NotInitialized(msg) => write!(f, "cipher vault not initialized: {msg}"),
            CipherError::AlreadyInitialized(msg) => {
                write!(f, "cipher vault already initialized: {msg}")
            }
            CipherError::NotFound(key) => write!(f, "secret `{key}` not found"),
            CipherError::InvalidKey(key) => write!(f, "invalid secret key `{key}`"),
            CipherError::InvalidPolicy(msg) => write!(f, "invalid secret policy: {msg}"),
            CipherError::Crypto(msg) => write!(f, "cipher cryptographic error: {msg}"),
            CipherError::Io(msg) => write!(f, "cipher I/O error: {msg}"),
            CipherError::Transport(msg) => write!(f, "cipher transport error: {msg}"),
            CipherError::VtpmUnavailable(msg) => write!(f, "vTPM unavailable: {msg}"),
            CipherError::UnsupportedBackend(msg) => write!(f, "unsupported cipher backend: {msg}"),
        }
    }
}

impl std::error::Error for CipherError {}

/// Port contract for local or external secret vault backends.
pub trait CipherVault {
    /// Initializes a new vault or validates an existing vault without overwriting keys.
    fn init(&self) -> CipherResult<VaultStatus>;

    /// Reads vault status and metadata entries against optional policy expectations.
    fn status(&self, policy: Option<&SecretPolicy>) -> CipherResult<VaultStatus>;

    /// Resolves and decrypts a named secret inside a redacted [`Secret`] wrapper.
    fn get(&self, reference: &CipherRef) -> CipherResult<Secret<Vec<u8>>>;

    /// Encrypts and stores plaintext bytes under `reference`, returning stored metadata.
    fn set(
        &self,
        reference: &CipherRef,
        plaintext: &Secret<Vec<u8>>,
    ) -> CipherResult<SecretMetadata>;

    /// Removes a stored entry by reference.
    fn delete(&self, reference: &CipherRef) -> CipherResult<()>;

    /// Computes a [`nomos_core::resource::Digest`] for arbitrary secret bytes using the backend digest.
    fn digest_secret(&self, plaintext: &Secret<Vec<u8>>) -> Digest;
}

/// Transport contract abstracting inspection and atomic delivery to configured [`NodeTarget`]s.
pub trait TargetTransport {
    /// Inspects remote metadata digest and mode without transferring plaintext.
    fn inspect_remote(
        &self,
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
    ) -> CipherResult<RemoteSecretState>;

    /// Atomically installs a rendered secret payload with owner, group, and mode on `target`.
    fn write_remote(
        &self,
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
        payload: &Secret<Vec<u8>>,
    ) -> CipherResult<()>;

    /// Restarts configured systemd units after a secret update.
    fn restart_units(&self, target: &NodeTarget, units: &[String]) -> CipherResult<()>;

    /// Probes host vTPM readiness (`systemd-creds has-tpm2` and TSS2 shared libraries).
    fn probe_vtpm(&self, target: Option<&NodeTarget>) -> CipherResult<VtpmPosture>;
}

/// Combined port capability for resolving posture and driving policy sync.
pub trait CipherPort: CipherVault {
    /// Plans and optionally applies policy sync across targets using `transport`.
    fn sync_policy(
        &self,
        policy: &SecretPolicy,
        transport: &dyn TargetTransport,
        target_filter: Option<&str>,
        confirm_apply: bool,
    ) -> CipherResult<SyncPlan>;

    /// Builds a metadata-only [`SecretsPosture`] combining vault state and vTPM readiness.
    fn posture(
        &self,
        policy: &SecretPolicy,
        transport: &dyn TargetTransport,
        target: Option<&NodeTarget>,
    ) -> CipherResult<SecretsPosture>;
}
