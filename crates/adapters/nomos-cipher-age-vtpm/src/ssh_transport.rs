//! SSH target delivery transport piping secrets exclusively via `stdin`.
//!
//! Enforces strict host-key pinning (`StrictHostKeyChecking=yes` +
//! `UserKnownHostsFile=<pinned>`), POSIX single-quoting on all remote arguments,
//! `umask 077`, CSPRNG staging suffixes, bounded pipe reads, and process deadlines.

use std::path::Path;
use std::process::Command;

use nomos_cipher::{
    CipherError, CipherResult, NodeTarget, RemoteSecretState, SecretPolicyEntry, TargetTransport,
    VtpmPosture, validate_ascii_identifier, validate_safe_path,
};
use nomos_core::cipher::Secret;

pub use crate::bounded_proc::{
    BoundedCommandOutput, DEFAULT_SSH_TIMEOUT, MAX_SSH_OUTPUT_BYTES, run_command_bounded,
};
use crate::transport::probe_local_vtpm;
use crate::vault_io::csprng_hex_suffix;

/// Wraps `arg` in POSIX single quotes, escaping embedded single quotes safely.
pub fn posix_single_quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// SSH transport piping secret payloads strictly over `stdin` to caller [`NodeTarget`]s.
#[derive(Clone, Debug, Default)]
pub struct SshTargetTransport {
    /// Optional SSH identity key path supplied by the operator.
    pub identity_file: Option<String>,
    /// Required pinned `known_hosts` file path for `StrictHostKeyChecking=yes`.
    pub known_hosts_file: Option<String>,
}

impl SshTargetTransport {
    /// Creates an SSH transport pinned to `known_hosts_file`.
    pub fn new(known_hosts_file: impl Into<String>) -> Self {
        Self {
            identity_file: None,
            known_hosts_file: Some(known_hosts_file.into()),
        }
    }

    /// Configures an explicit SSH client identity key path.
    pub fn with_identity_file(mut self, identity_file: impl Into<String>) -> Self {
        self.identity_file = Some(identity_file.into());
        self
    }

    /// Builds validated SSH CLI arguments enforcing `StrictHostKeyChecking=yes`,
    /// keepalive dead-peer detection, and a pinned `UserKnownHostsFile`.
    pub fn build_ssh_args(&self, target: &NodeTarget) -> CipherResult<Vec<String>> {
        target.validate()?;
        let known_hosts = self.known_hosts_file.as_deref().ok_or_else(|| {
            CipherError::InvalidPolicy(
                "SshTargetTransport requires a pinned known_hosts_file".into(),
            )
        })?;
        validate_safe_path("known_hosts_file", known_hosts)?;

        let mut args = vec![
            "-o".to_owned(),
            "BatchMode=yes".to_owned(),
            "-o".to_owned(),
            "ConnectTimeout=8".to_owned(),
            "-o".to_owned(),
            "ServerAliveInterval=10".to_owned(),
            "-o".to_owned(),
            "ServerAliveCountMax=2".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=yes".to_owned(),
            "-o".to_owned(),
            format!("UserKnownHostsFile={known_hosts}"),
            "-p".to_owned(),
            target.ssh_port.to_string(),
        ];
        if let Some(id_file) = &self.identity_file {
            validate_safe_path("identity_file", id_file)?;
            args.push("-i".to_owned());
            args.push(id_file.clone());
        }
        args.push(format!("{}@{}", target.remote_user, target.address));
        Ok(args)
    }

    /// Builds the POSIX-quoted `umask 077` remote staging and atomic install script.
    pub fn build_write_remote_script(
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
        stage_nonce_hex: &str,
    ) -> CipherResult<String> {
        target.validate()?;
        entry.validate()?;
        validate_ascii_identifier("stage_nonce_hex", stage_nonce_hex)?;

        let dest = entry.delivery.destination_path();
        let stage = format!(
            "{}/{}.{}.stage",
            target.staging_dir, entry.key, stage_nonce_hex
        );
        validate_safe_path("remote stage path", &stage)?;
        let parent = Path::new(dest)
            .parent()
            .and_then(|p| p.to_str())
            .filter(|p| !p.is_empty())
            .unwrap_or("/");

        let q_stage_dir = posix_single_quote(&target.staging_dir);
        let q_stage = posix_single_quote(&stage);
        let q_parent = posix_single_quote(parent);
        let q_owner = posix_single_quote(&entry.owner);
        let q_group = posix_single_quote(&entry.group);
        let q_mode = posix_single_quote(&entry.mode);
        let q_dest = posix_single_quote(dest);

        Ok(format!(
            "umask 077 && mkdir -p {q_stage_dir} && chmod 0700 {q_stage_dir} && trap 'rm -f {q_stage}' EXIT && cat > {q_stage} && sudo install -d -m 0755 {q_parent} && sudo install -o {q_owner} -g {q_group} -m {q_mode} {q_stage} {q_dest} && rm -f {q_stage}"
        ))
    }

    fn ssh_command(&self, target: &NodeTarget) -> CipherResult<Command> {
        let args = self.build_ssh_args(target)?;
        let mut cmd = Command::new("ssh");
        cmd.args(args);
        Ok(cmd)
    }
}

impl TargetTransport for SshTargetTransport {
    fn inspect_remote(
        &self,
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
    ) -> CipherResult<RemoteSecretState> {
        entry.validate()?;
        let dest = entry.delivery.destination_path();
        let q_dest = posix_single_quote(dest);
        let mut cmd = self.ssh_command(target)?;
        cmd.arg(format!(
            "if test -e {q_dest}; then stat -c '%a' {q_dest} && b3sum {q_dest} | cut -d' ' -f1; else echo ABSENT; fi"
        ));
        let out = run_command_bounded(
            cmd,
            None,
            &format!("ssh inspect `{}`", target.name),
            DEFAULT_SSH_TIMEOUT,
            MAX_SSH_OUTPUT_BYTES,
        )?;
        if !out.status.success() {
            return Err(CipherError::Transport(format!(
                "ssh inspect `{}` exited {}: {}",
                target.name,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
        match lines.next() {
            Some("ABSENT") | None => Ok(RemoteSecretState {
                exists: false,
                blake3_hex: None,
                mode: None,
            }),
            Some(mode) => {
                let normalized_mode = if mode.len() == 3 {
                    format!("0{mode}")
                } else {
                    mode.to_owned()
                };
                Ok(RemoteSecretState {
                    exists: true,
                    blake3_hex: lines.next().map(str::to_owned),
                    mode: Some(normalized_mode),
                })
            }
        }
    }

    fn write_remote(
        &self,
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
        payload: &Secret<Vec<u8>>,
    ) -> CipherResult<()> {
        let nonce = csprng_hex_suffix()?;
        let script = Self::build_write_remote_script(target, entry, &nonce)?;
        let mut cmd = self.ssh_command(target)?;
        cmd.arg(script);
        let out = run_command_bounded(
            cmd,
            Some(payload.expose()),
            &format!("ssh write `{}`", target.name),
            DEFAULT_SSH_TIMEOUT,
            MAX_SSH_OUTPUT_BYTES,
        )?;
        if !out.status.success() {
            return Err(CipherError::Transport(format!(
                "ssh write `{}` exited {}: {}",
                target.name,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    fn restart_units(&self, target: &NodeTarget, units: &[String]) -> CipherResult<()> {
        if units.is_empty() {
            return Ok(());
        }
        let mut quoted_units = Vec::with_capacity(units.len());
        for unit in units {
            validate_ascii_identifier("restart_unit", unit)?;
            quoted_units.push(posix_single_quote(unit));
        }
        let mut cmd = self.ssh_command(target)?;
        cmd.arg(format!("sudo systemctl restart {}", quoted_units.join(" ")));
        let out = run_command_bounded(
            cmd,
            None,
            &format!("ssh restart `{}`", target.name),
            DEFAULT_SSH_TIMEOUT,
            MAX_SSH_OUTPUT_BYTES,
        )?;
        if !out.status.success() {
            return Err(CipherError::Transport(format!(
                "ssh restart `{}` failed: {}",
                target.name,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    fn probe_vtpm(&self, target: Option<&NodeTarget>) -> CipherResult<VtpmPosture> {
        let Some(target) = target else {
            return Ok(probe_local_vtpm("local"));
        };
        let mut cmd = self.ssh_command(target)?;
        cmd.arg(
            "command -v systemd-creds >/dev/null 2>&1 && echo CREDS=1 || echo CREDS=0; systemd-creds has-tpm2 >/dev/null 2>&1 && echo TPM2=1 || echo TPM2=0; (ldconfig -p 2>/dev/null | grep -q libtss2-esys.so.0 || test -e /usr/lib/libtss2-esys.so.0) && echo ESYS=1 || echo ESYS=0; (ldconfig -p 2>/dev/null | grep -q libtss2-tcti-device.so.0 || test -e /usr/lib/libtss2-tcti-device.so.0) && echo TCTI=1 || echo TCTI=0",
        );
        let out = run_command_bounded(
            cmd,
            None,
            &format!("ssh vtpm probe `{}`", target.name),
            DEFAULT_SSH_TIMEOUT,
            MAX_SSH_OUTPUT_BYTES,
        )?;
        if !out.status.success() {
            return Err(CipherError::Transport(format!(
                "ssh vtpm probe `{}` exited {}",
                target.name, out.status
            )));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let flag = |k: &str| text.lines().any(|l| l.trim() == format!("{k}=1"));
        Ok(VtpmPosture {
            node: target.name.clone(),
            systemd_creds_available: flag("CREDS"),
            has_tpm2: flag("TPM2"),
            libtss2_esys: flag("ESYS"),
            libtss2_tcti_device: flag("TCTI"),
            detail: "remote ssh probe completed".to_owned(),
        })
    }
}
