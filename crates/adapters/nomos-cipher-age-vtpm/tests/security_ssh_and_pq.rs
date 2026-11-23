//! Security tests for bounded SSH subprocesses, failure isolation, and Zeroizing PQ keys (#15, #17).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nomos_cipher::{
    CipherError, CipherRef, CipherVault, DeliveryMode, NodeTarget, RemoteSecretState, SecretPolicy,
    SecretPolicyEntry, TargetTransport, VtpmPosture,
};
use nomos_cipher_age_vtpm::bounded_proc::run_command_bounded;
use nomos_cipher_age_vtpm::{
    AgeVtpmVault, HybridIdentity, MockCredSealer, SshTargetTransport, VaultCipherMode,
    VaultKeyStorage, plan_and_execute_sync_with_sealer,
};
use nomos_core::cipher::Secret;
use zeroize::Zeroizing;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "nomos-cipher-sec-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ssh_args_include_keepalives_and_bounded_runner_kills_hangs_and_floods() {
    let transport = SshTargetTransport::new("/etc/nomos/known_hosts");
    let target = NodeTarget {
        name: "alpha".to_owned(),
        address: "fd00:db8::10".to_owned(),
        ssh_port: 22,
        remote_user: "ops".to_owned(),
        staging_dir: "/run/nomos".to_owned(),
    };
    let args = transport.build_ssh_args(&target).unwrap();
    assert!(args.contains(&"ServerAliveInterval=10".to_owned()));
    assert!(args.contains(&"ServerAliveCountMax=2".to_owned()));

    // Timeout enforcement kills hanging child cleanly.
    let mut hang = Command::new("sh");
    hang.arg("-c").arg("sleep 5");
    let err =
        run_command_bounded(hang, None, "hang-test", Duration::from_millis(60), 4096).unwrap_err();
    assert!(matches!(err, CipherError::Transport(ref m) if m.contains("timed out")));

    // Output byte limit kills flooding child cleanly.
    let mut flood = Command::new("sh");
    flood
        .arg("-c")
        .arg("dd if=/dev/zero bs=4096 count=10 2>/dev/null");
    let overflow =
        run_command_bounded(flood, None, "flood-test", Duration::from_secs(2), 1024).unwrap_err();
    assert!(matches!(overflow, CipherError::Transport(ref m) if m.contains("exceeded")));

    // Normal short execution succeeds.
    let mut ok = Command::new("sh");
    ok.arg("-c").arg("cat");
    let res =
        run_command_bounded(ok, Some(b"hello"), "ok-test", Duration::from_secs(2), 1024).unwrap();
    assert!(res.status.success());
    assert_eq!(res.stdout, b"hello");
}

#[test]
fn sync_isolates_failing_target_from_healthy_targets_and_pq_encode_returns_zeroizing() {
    // #17: encode_secret yields Zeroizing<String> cleanly round-tripped by parse_secret.
    let id = HybridIdentity::generate().unwrap();
    let encoded: Zeroizing<String> = id.encode_secret();
    let parsed = HybridIdentity::parse_secret(&encoded).unwrap();
    assert_eq!(
        id.to_public().fingerprint_hex(),
        parsed.to_public().fingerprint_hex()
    );

    // #15: Failure on alpha must NOT prevent delivery & unit restart on beta.
    struct FailingFirstTransport {
        writes: Mutex<BTreeMap<String, usize>>,
        restarts: Mutex<BTreeMap<String, usize>>,
    }

    impl TargetTransport for FailingFirstTransport {
        fn inspect_remote(
            &self,
            target: &NodeTarget,
            _entry: &SecretPolicyEntry,
        ) -> nomos_cipher::CipherResult<RemoteSecretState> {
            if target.name == "alpha" {
                return Err(CipherError::Transport("alpha offline".into()));
            }
            Ok(RemoteSecretState {
                exists: false,
                blake3_hex: None,
                mode: None,
            })
        }

        fn write_remote(
            &self,
            target: &NodeTarget,
            _entry: &SecretPolicyEntry,
            _payload: &Secret<Vec<u8>>,
        ) -> nomos_cipher::CipherResult<()> {
            *self
                .writes
                .lock()
                .unwrap()
                .entry(target.name.clone())
                .or_default() += 1;
            Ok(())
        }

        fn restart_units(
            &self,
            target: &NodeTarget,
            _units: &[String],
        ) -> nomos_cipher::CipherResult<()> {
            *self
                .restarts
                .lock()
                .unwrap()
                .entry(target.name.clone())
                .or_default() += 1;
            Ok(())
        }

        fn probe_vtpm(
            &self,
            target: Option<&NodeTarget>,
        ) -> nomos_cipher::CipherResult<VtpmPosture> {
            Ok(VtpmPosture {
                node: target.map_or_else(|| "local".to_owned(), |t| t.name.clone()),
                systemd_creds_available: true,
                has_tpm2: true,
                libtss2_esys: true,
                libtss2_tcti_device: true,
                detail: "ok".to_owned(),
            })
        }
    }

    let tmp = TempDir::new("sync-iso");
    let vault = AgeVtpmVault::new(&tmp.0)
        .with_key_storage(VaultKeyStorage::Tpm2Sealed)
        .with_cipher_mode(VaultCipherMode::HybridMlKem768X25519)
        .with_cred_sealer(Arc::new(MockCredSealer::default()));
    vault.init().unwrap();
    vault
        .set(
            &CipherRef::new("shared_tok").unwrap(),
            &Secret::new(b"secret-value".to_vec()),
        )
        .unwrap();

    let policy = SecretPolicy::new(
        vec![
            NodeTarget {
                name: "alpha".to_owned(),
                address: "fd00:db8::1".to_owned(),
                ssh_port: 22,
                remote_user: "ops".to_owned(),
                staging_dir: "/run/stage".to_owned(),
            },
            NodeTarget {
                name: "beta".to_owned(),
                address: "fd00:db8::2".to_owned(),
                ssh_port: 22,
                remote_user: "ops".to_owned(),
                staging_dir: "/run/stage".to_owned(),
            },
        ],
        vec![SecretPolicyEntry {
            key: "shared_tok".to_owned(),
            target_names: vec!["alpha".to_owned(), "beta".to_owned()],
            delivery: DeliveryMode::RawFile {
                path: "/etc/nomos/shared.tok".to_owned(),
            },
            owner: "root".to_owned(),
            group: "root".to_owned(),
            mode: "0400".to_owned(),
            restart_units: vec!["nomos-cell.service".to_owned()],
            required: true,
            description: "shared".to_owned(),
        }],
    )
    .unwrap();

    let transport = FailingFirstTransport {
        writes: Mutex::new(BTreeMap::new()),
        restarts: Mutex::new(BTreeMap::new()),
    };
    let err = plan_and_execute_sync_with_sealer(
        &vault,
        &policy,
        &transport,
        None,
        true,
        &MockCredSealer::default(),
    )
    .unwrap_err();
    assert!(matches!(err, CipherError::Transport(ref m) if m.contains("alpha offline")));
    assert_eq!(transport.writes.lock().unwrap().get("beta"), Some(&1));
    assert_eq!(transport.restarts.lock().unwrap().get("beta"), Some(&1));
}
