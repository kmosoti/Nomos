//! Data-driven secret delivery policy and node target declarations.
//!
//! Policies describe secret keys, delivery formats, file ownership, and node
//! targets strictly as configuration data so adapters carry zero hardcoded
//! node identifiers or network addresses.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::CipherError;

/// Destination host descriptor supplied by caller topology or policy config.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeTarget {
    /// Logical target identifier matching [`SecretPolicyEntry::target_names`].
    pub name: String,
    /// Reachable network host or IPv6 address (for example `fd00:db8::10`).
    pub address: String,
    /// SSH control port used for remote transport.
    pub ssh_port: u16,
    /// Remote Unix account used for transport staging.
    pub remote_user: String,
    /// Remote staging directory owned by [`Self::remote_user`].
    pub staging_dir: String,
}

impl NodeTarget {
    /// Validates that a target descriptor is non-empty and free of whitespace.
    pub fn validate(&self) -> Result<(), CipherError> {
        if self.name.trim().is_empty()
            || self.name.contains(char::is_whitespace)
            || self.name.contains('/')
            || self.name.contains("..")
        {
            return Err(CipherError::InvalidPolicy(format!(
                "invalid node target name `{}`",
                self.name
            )));
        }
        if self.address.trim().is_empty() || self.address.contains(char::is_whitespace) {
            return Err(CipherError::InvalidPolicy(format!(
                "target `{}` has an invalid network address",
                self.name
            )));
        }
        if self.ssh_port == 0 {
            return Err(CipherError::InvalidPolicy(format!(
                "target `{}` cannot use ssh_port 0",
                self.name
            )));
        }
        if self.remote_user.trim().is_empty() || !self.staging_dir.starts_with('/') {
            return Err(CipherError::InvalidPolicy(format!(
                "target `{}` must specify a remote_user and absolute staging_dir",
                self.name
            )));
        }
        Ok(())
    }
}

/// Format and remote location for delivering a secret onto a node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Writes `KEY=VALUE` inside a root-readable environment file.
    EnvFile {
        /// Absolute file path on the target node.
        path: String,
        /// Environment variable name inside the file.
        env_var: String,
    },
    /// Writes raw secret bytes to an absolute path on the target node.
    RawFile {
        /// Absolute destination file path.
        path: String,
    },
    /// Writes a systemd credential file under a credential directory.
    SystemdCred {
        /// Credential identifier consumed by `LoadCredential=`.
        name: String,
        /// Absolute destination file path.
        path: String,
    },
}

impl DeliveryMode {
    /// Absolute destination path on the target host.
    pub fn destination_path(&self) -> &str {
        match self {
            DeliveryMode::EnvFile { path, .. }
            | DeliveryMode::RawFile { path }
            | DeliveryMode::SystemdCred { path, .. } => path,
        }
    }

    /// Validates that paths and identifiers are well-formed.
    pub fn validate(&self, key: &str) -> Result<(), CipherError> {
        let path = self.destination_path();
        if !path.starts_with('/') || path.contains("..") {
            return Err(CipherError::InvalidPolicy(format!(
                "secret `{key}` has non-absolute or traversing path `{path}`"
            )));
        }
        match self {
            DeliveryMode::EnvFile { env_var, .. } => {
                if env_var.is_empty()
                    || !env_var
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err(CipherError::InvalidPolicy(format!(
                        "secret `{key}` has invalid env_var `{env_var}`"
                    )));
                }
            }
            DeliveryMode::SystemdCred { name, .. } => {
                if name.is_empty() || name.contains('/') || name.contains("..") {
                    return Err(CipherError::InvalidPolicy(format!(
                        "secret `{key}` has invalid credential name `{name}`"
                    )));
                }
            }
            DeliveryMode::RawFile { .. } => {}
        }
        Ok(())
    }
}

/// Policy entry mapping one logical vault key to its delivery targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretPolicyEntry {
    /// Logical vault entry identifier.
    pub key: String,
    /// Target names in [`SecretPolicy::targets`] receiving this secret.
    pub target_names: Vec<String>,
    /// On-disk delivery format and path.
    pub delivery: DeliveryMode,
    /// Target POSIX file owner.
    pub owner: String,
    /// Target POSIX file group.
    pub group: String,
    /// Octal mode string such as `"0400"` or `"0600"`.
    pub mode: String,
    /// Optional systemd units restarted when the delivered digest changes.
    #[serde(default)]
    pub restart_units: Vec<String>,
    /// Marks entries required before declaring workload readiness.
    #[serde(default)]
    pub required: bool,
    /// Human-readable description shown in status metadata.
    #[serde(default)]
    pub description: String,
}

impl SecretPolicyEntry {
    /// Validates entry key, ownership, octal mode, and delivery configuration.
    pub fn validate(&self) -> Result<(), CipherError> {
        validate_secret_key(&self.key)?;
        self.delivery.validate(&self.key)?;
        if self.owner.trim().is_empty() || self.group.trim().is_empty() {
            return Err(CipherError::InvalidPolicy(format!(
                "secret `{}` must define non-empty owner and group",
                self.key
            )));
        }
        if self.mode.len() < 3
            || self.mode.len() > 4
            || !self.mode.bytes().all(|b| (b'0'..=b'7').contains(&b))
        {
            return Err(CipherError::InvalidPolicy(format!(
                "secret `{}` has invalid octal mode `{}`",
                self.key, self.mode
            )));
        }
        Ok(())
    }
}

/// Complete data-driven secret policy binding entries to node targets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretPolicy {
    /// All target hosts eligible for sync operations.
    pub targets: Vec<NodeTarget>,
    /// Managed secret definitions and delivery rules.
    pub entries: Vec<SecretPolicyEntry>,
}

impl SecretPolicy {
    /// Constructs a validated policy or rejects inconsistent targets and keys.
    pub fn new(
        targets: Vec<NodeTarget>,
        entries: Vec<SecretPolicyEntry>,
    ) -> Result<Self, CipherError> {
        let policy = Self { targets, entries };
        policy.validate()?;
        Ok(policy)
    }

    /// Validates targets, entry definitions, and target cross-references.
    pub fn validate(&self) -> Result<(), CipherError> {
        for target in &self.targets {
            target.validate()?;
        }
        for entry in &self.entries {
            entry.validate()?;
            for target_name in &entry.target_names {
                if self.find_target(target_name).is_none() {
                    return Err(CipherError::InvalidPolicy(format!(
                        "secret `{}` references unknown target `{target_name}`",
                        entry.key
                    )));
                }
            }
        }
        Ok(())
    }

    /// Looks up a target by logical name.
    pub fn find_target(&self, name: &str) -> Option<&NodeTarget> {
        self.targets.iter().find(|t| t.name == name)
    }

    /// Looks up a policy entry by secret key.
    pub fn find_entry(&self, key: &str) -> Option<&SecretPolicyEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Lists policy entries targeting a given node.
    pub fn entries_for_target(&self, target_name: &str) -> Vec<&SecretPolicyEntry> {
        self.entries
            .iter()
            .filter(|e| e.target_names.iter().any(|name| name == target_name))
            .collect()
    }
}

/// Validates a logical secret key against path-traversal and control characters.
pub fn validate_secret_key(key: &str) -> Result<(), CipherError> {
    if key.is_empty()
        || key.len() > 128
        || key.contains('/')
        || key.contains('\\')
        || key.contains("..")
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(CipherError::InvalidKey(key.to_owned()));
    }
    Ok(())
}

/// Typed reference to a protected secret entry without carrying plaintext.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CipherRef {
    key: String,
    expected_blake3_hex: Option<String>,
}

impl CipherRef {
    /// Creates a validated secret reference.
    pub fn new(key: impl Into<String>) -> Result<Self, CipherError> {
        let key = key.into();
        validate_secret_key(&key)?;
        Ok(Self {
            key,
            expected_blake3_hex: None,
        })
    }

    /// Attaches an expected BLAKE3 hex integrity digest.
    pub fn with_digest_hex(mut self, digest_hex: impl Into<String>) -> Self {
        self.expected_blake3_hex = Some(digest_hex.into());
        self
    }

    /// Logical key name of the secret.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Optional expected BLAKE3 hex integrity digest.
    pub fn expected_blake3_hex(&self) -> Option<&str> {
        self.expected_blake3_hex.as_deref()
    }
}

impl fmt::Debug for CipherRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CipherRef")
            .field("key", &self.key)
            .field("has_digest", &self.expected_blake3_hex.is_some())
            .finish()
    }
}

impl fmt::Display for CipherRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cipher:{}", self.key)
    }
}
