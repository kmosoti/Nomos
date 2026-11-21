//! SSH target delivery transport piping secrets exclusively via `stdin`.
//!
//! Targets and network addresses come strictly from caller-supplied
//! [`NodeTarget`] structs with zero hardcoded infrastructure constants.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use nomos_cipher::{
    CipherError, CipherResult, NodeTarget, RemoteSecretState, SecretPolicyEntry, TargetTransport,
    VtpmPosture,
};
use nomos_core::cipher::Secret;

use crate::transport::probe_local_vtpm;

/// SSH transport piping secret payloads strictly over `stdin` to caller [`NodeTarget`]s.
#[derive(Clone, Debug, Default)]
pub struct SshTargetTransport {
    /// Optional SSH identity key path supplied by the operator.
    pub identity_file: Option<String>,
}

impl SshTargetTransport {
    fn ssh_command(&self, target: &NodeTarget) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=8")
            .arg("-p")
            .arg(target.ssh_port.to_string());
        if let Some(id_file) = &self.identity_file {
            cmd.arg("-i").arg(id_file);
        }
        cmd.arg(format!("{}@{}", target.remote_user, target.address));
        cmd
    }
}

impl TargetTransport for SshTargetTransport {
    fn inspect_remote(
        &self,
        target: &NodeTarget,
        entry: &SecretPolicyEntry,
    ) -> CipherResult<RemoteSecretState> {
        let dest = entry.delivery.destination_path();
        let mut cmd = self.ssh_command(target);
        cmd.arg(format!(
            "if test -e {dest:?}; then stat -c '%a' {dest:?} && b3sum {dest:?} | cut -d' ' -f1; else echo ABSENT; fi"
        ));
        let out = cmd
            .output()
            .map_err(|e| CipherError::Transport(format!("ssh inspect `{}`: {e}", target.name)))?;
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
        let dest = entry.delivery.destination_path();
        let stage = format!("{}/{}.stage", target.staging_dir, entry.key);
        let parent = Path::new(dest)
            .parent()
            .map_or("/", |p| p.to_str().unwrap_or("/"));

        let mut cmd = self.ssh_command(target);
        cmd.arg(format!(
            "mkdir -p {stage_dir:?} && cat > {stage:?} && sudo install -d -m 0755 {parent:?} && sudo install -o {owner:?} -g {group:?} -m {mode} {stage:?} {dest:?} && rm -f {stage:?}",
            stage_dir = target.staging_dir,
            owner = entry.owner,
            group = entry.group,
            mode = entry.mode,
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| CipherError::Transport(format!("ssh spawn `{}`: {e}", target.name)))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(payload.expose()).map_err(|e| {
                CipherError::Transport(format!("ssh write payload `{}`: {e}", target.name))
            })?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| CipherError::Transport(format!("ssh wait `{}`: {e}", target.name)))?;
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
        let mut cmd = self.ssh_command(target);
        cmd.arg("sudo").arg("systemctl").arg("restart");
        for unit in units {
            cmd.arg(unit);
        }
        let out = cmd
            .output()
            .map_err(|e| CipherError::Transport(format!("ssh restart `{}`: {e}", target.name)))?;
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
        let mut cmd = self.ssh_command(target);
        cmd.arg(
            "command -v systemd-creds >/dev/null 2>&1 && echo CREDS=1 || echo CREDS=0; systemd-creds has-tpm2 >/dev/null 2>&1 && echo TPM2=1 || echo TPM2=0; (ldconfig -p 2>/dev/null | grep -q libtss2-esys.so.0 || test -e /usr/lib/libtss2-esys.so.0) && echo ESYS=1 || echo ESYS=0; (ldconfig -p 2>/dev/null | grep -q libtss2-tcti-device.so.0 || test -e /usr/lib/libtss2-tcti-device.so.0) && echo TCTI=1 || echo TCTI=0",
        );
        let out = cmd.output().map_err(|e| {
            CipherError::Transport(format!("ssh vtpm probe `{}`: {e}", target.name))
        })?;
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
