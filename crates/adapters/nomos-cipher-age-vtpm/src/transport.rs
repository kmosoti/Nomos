//! Local vTPM 2.0 posture probing and deterministic in-memory target transport.
//!
//! No target names, network addresses, or SSH identities are hardcoded here;
//! every destination is driven by caller-supplied [`NodeTarget`] structs.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use nomos_cipher::{
    CipherError, CipherResult, NodeTarget, RemoteSecretState, SecretPolicyEntry, TargetTransport,
    VtpmPosture,
};
use nomos_core::cipher::Secret;

use crate::vault::blake3_hex;

const TSS2_SEARCH_DIRS: &[&str] = &[
    "/usr/lib",
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/lib/x86_64-linux-gnu",
    "/run/current-system/sw/lib",
    "/run/booted-system/sw/lib",
];

/// Checks whether a shared library soname exists across standard Linux/Nix library paths.
pub fn has_shared_library(soname: &str) -> bool {
    if let Ok(ld_paths) = std::env::var("LD_LIBRARY_PATH") {
        for dir in ld_paths.split(':').filter(|s| !s.is_empty()) {
            if Path::new(dir).join(soname).exists() {
                return true;
            }
        }
    }
    TSS2_SEARCH_DIRS
        .iter()
        .any(|dir| Path::new(dir).join(soname).exists())
}

/// Probes the local host for `systemd-creds has-tpm2` and TSS2 device/ESYS libraries.
///
/// Returns metadata-only readiness without touching or logging secret values.
pub fn probe_local_vtpm(node_label: &str) -> VtpmPosture {
    let libtss2_esys = has_shared_library("libtss2-esys.so.0");
    let libtss2_tcti_device = has_shared_library("libtss2-tcti-device.so.0");

    let (systemd_creds_available, has_tpm2, cred_detail) = match Command::new("systemd-creds")
        .arg("has-tpm2")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            let msg = if !stdout.is_empty() {
                stdout
            } else if !stderr.is_empty() {
                stderr
            } else {
                format!("exit {}", out.status.code().unwrap_or(-1))
            };
            (true, out.status.success(), msg)
        }
        Err(err) => (false, false, format!("systemd-creds unavailable: {err}")),
    };

    VtpmPosture {
        node: node_label.to_owned(),
        has_tpm2,
        systemd_creds_available,
        libtss2_esys,
        libtss2_tcti_device,
        detail: cred_detail,
    }
}

/// Stored entry inside [`MemoryTargetTransport`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredRemoteFile {
    blake3_hex: String,
    mode: String,
    byte_len: usize,
}

type RemoteFileMap = Arc<Mutex<BTreeMap<(String, String), StoredRemoteFile>>>;
type RestartLog = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// Deterministic in-memory transport for dry-run verification and integration tests.
///
/// Stores non-secret BLAKE3 digests and modes rather than retaining plaintext
/// secret copies in test inspection state.
#[derive(Clone, Debug, Default)]
pub struct MemoryTargetTransport {
    files: RemoteFileMap,
    restarted: RestartLog,
    vtpm_override: Option<VtpmPosture>,
}

impl MemoryTargetTransport {
    /// Creates an empty in-memory transport using local host vTPM probing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures a deterministic [`VtpmPosture`] override for test scenarios.
    pub fn with_vtpm_posture(mut self, posture: VtpmPosture) -> Self {
        self.vtpm_override = Some(posture);
        self
    }

    /// Returns recorded `(target_name, restarted_units)` invocations.
    pub fn restarted_units(&self) -> Vec<(String, Vec<String>)> {
        self.restarted
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Seeds a remote file digest and mode directly for drift simulation.
    pub fn seed_remote_digest(
        &self,
        target: &str,
        destination: &str,
        blake3_hex: impl Into<String>,
        mode: impl Into<String>,
        byte_len: usize,
    ) {
        if let Ok(mut guard) = self.files.lock() {
            guard.insert(
                (target.to_owned(), destination.to_owned()),
                StoredRemoteFile {
                    blake3_hex: blake3_hex.into(),
                    mode: mode.into(),
                    byte_len,
                },
            );
        }
    }

    /// Inspects recorded metadata size without exposing secret bytes.
    pub fn recorded_len(&self, target: &str, destination: &str) -> Option<usize> {
        self.files.lock().ok().and_then(|g| {
            g.get(&(target.to_owned(), destination.to_owned()))
                .map(|f| f.byte_len)
        })
    }
}

impl TargetTransport for MemoryTargetTransport {
    fn inspect_remote(
        &self,
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
    ) -> CipherResult<RemoteSecretState> {
        let guard = self
            .files
            .lock()
            .map_err(|_| CipherError::Transport("memory transport lock poisoned".into()))?;
        let key = (
            target.name.clone(),
            entry.delivery.destination_path().to_owned(),
        );
        Ok(match guard.get(&key) {
            Some(file) => RemoteSecretState {
                exists: true,
                blake3_hex: Some(file.blake3_hex.clone()),
                mode: Some(file.mode.clone()),
            },
            None => RemoteSecretState {
                exists: false,
                blake3_hex: None,
                mode: None,
            },
        })
    }

    fn write_remote(
        &self,
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
        payload: &Secret<Vec<u8>>,
    ) -> CipherResult<()> {
        let mut guard = self
            .files
            .lock()
            .map_err(|_| CipherError::Transport("memory transport lock poisoned".into()))?;
        let key = (
            target.name.clone(),
            entry.delivery.destination_path().to_owned(),
        );
        guard.insert(
            key,
            StoredRemoteFile {
                blake3_hex: blake3_hex(payload.expose()),
                mode: entry.mode.clone(),
                byte_len: payload.expose().len(),
            },
        );
        Ok(())
    }

    fn restart_units(&self, target: &NodeTarget, units: &[String]) -> CipherResult<()> {
        let mut guard = self
            .restarted
            .lock()
            .map_err(|_| CipherError::Transport("memory transport lock poisoned".into()))?;
        guard.push((target.name.clone(), units.to_vec()));
        Ok(())
    }

    fn probe_vtpm(&self, target: Option<&NodeTarget>) -> CipherResult<VtpmPosture> {
        if let Some(posture) = &self.vtpm_override {
            return Ok(posture.clone());
        }
        let label = target.map_or("local", |t| t.name.as_str());
        Ok(probe_local_vtpm(label))
    }
}
