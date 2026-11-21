//! Metadata-only vault status, sync plans, and vTPM readiness posture.
//!
//! No structure in this module stores, derives, or serializes plaintext secret
//! bytes or private cryptographic material (invariant N8).

use serde::{Deserialize, Serialize};

/// Non-secret metadata summarizing one stored encrypted vault entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMetadata {
    /// Logical key identifying the entry.
    pub key: String,
    /// BLAKE3 hex digest of the decrypted secret bytes.
    pub blake3_hex: String,
    /// Length in bytes of the plaintext payload.
    pub byte_len: usize,
    /// Last modification timestamp in seconds since the Unix epoch.
    pub updated_unix_secs: u64,
}

/// Summary of a cipher vault's state and non-secret metadata index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultStatus {
    /// Filesystem or backend locator for the vault.
    pub vault_path: String,
    /// Whether the vault has been initialized with an identity key.
    pub initialized: bool,
    /// Public recipient identifier (never a private identity).
    pub recipient_public: Option<String>,
    /// Number of stored encrypted entries.
    pub entry_count: usize,
    /// Stored metadata records sorted by key.
    pub entries: Vec<SecretMetadata>,
    /// Required policy keys missing from the vault.
    pub policy_missing_keys: Vec<String>,
}

/// Observed state of a single remote secret destination without reading secret bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSecretState {
    /// Whether a file already exists at the destination path.
    pub exists: bool,
    /// Remote BLAKE3 digest of the currently deployed file or value.
    pub blake3_hex: Option<String>,
    /// Current octal permissions string on the target if present.
    pub mode: Option<String>,
}

/// Operation kind planned by [`SyncPlan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncActionKind {
    /// Target file is missing and needs initial creation.
    Create,
    /// Target file digest or mode differs from the vault/policy state.
    Update,
    /// Target file matches the desired digest and permissions.
    Skip,
}

/// Planned operation for delivering one policy entry to one node target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncAction {
    /// Logical target name from [`crate::NodeTarget`].
    pub target: String,
    /// Logical secret key.
    pub key: String,
    /// Destination path on the target host.
    pub destination: String,
    /// Action classification.
    pub kind: SyncActionKind,
    /// Non-secret reason explaining the planned action.
    pub reason: String,
    /// Systemd units to restart after delivering this action.
    pub restart_units: Vec<String>,
    /// Whether this action mutated target state when executed.
    pub applied: bool,
}

/// Deterministic sync plan across all selected targets and policy entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPlan {
    /// True when planned without mutating targets.
    pub dry_run: bool,
    /// Ordered actions computed from policy and observed remote digests.
    pub actions: Vec<SyncAction>,
    /// Non-fatal warnings such as missing optional vault entries.
    pub warnings: Vec<String>,
}

impl SyncPlan {
    /// Number of actions requiring target writes (`Create` or `Update`).
    pub fn pending_mutations(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a.kind, SyncActionKind::Create | SyncActionKind::Update))
            .count()
    }
}

/// Metadata-only vTPM 2.0 capability probe for one host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VtpmPosture {
    /// Logical host label from caller target metadata.
    pub node: String,
    /// True if `systemd-creds has-tpm2` exited cleanly with TPM2 support.
    pub has_tpm2: bool,
    /// True if `systemd-creds` executable is present on the host.
    pub systemd_creds_available: bool,
    /// True if `libtss2-esys.so.0` was detected on the host library paths.
    pub libtss2_esys: bool,
    /// True if `libtss2-tcti-device.so.0` was detected on the host library paths.
    pub libtss2_tcti_device: bool,
    /// Human-readable posture summary without any key or secret material.
    pub detail: String,
}

impl VtpmPosture {
    /// True when TPM2 and both required TSS2 shared libraries are present.
    pub fn fully_ready(&self) -> bool {
        self.has_tpm2
            && self.systemd_creds_available
            && self.libtss2_esys
            && self.libtss2_tcti_device
    }
}

/// Aggregate metadata-only secrets posture exposed to telemetry and governance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsPosture {
    /// Whether the backing vault is initialized and readable.
    pub vault_initialized: bool,
    /// Count of encrypted entries present in the vault.
    pub encrypted_entries: usize,
    /// Count of entries declared by active policy.
    pub policy_entries: usize,
    /// Policy keys missing from the vault (names only, zero plaintext).
    pub missing_keys: Vec<String>,
    /// Optional local or remote vTPM 2.0 capability summary.
    pub vtpm: Option<VtpmPosture>,
}

impl SecretsPosture {
    /// Builds metadata-only posture from vault status and optional vTPM probe.
    pub fn from_status(
        status: &VaultStatus,
        policy_entries: usize,
        vtpm: Option<VtpmPosture>,
    ) -> Self {
        Self {
            vault_initialized: status.initialized,
            encrypted_entries: status.entry_count,
            policy_entries,
            missing_keys: status.policy_missing_keys.clone(),
            vtpm,
        }
    }
}
