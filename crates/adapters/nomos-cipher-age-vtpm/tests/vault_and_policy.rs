//! Integration tests for `nomos-cipher` and `nomos-cipher-age-vtpm` (ADR 0018, N8).

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nomos_cipher::{
    CipherError, CipherPort, CipherRef, CipherVault, DeliveryMode, NodeTarget, SecretPolicy,
    SecretPolicyEntry, SyncActionKind, VtpmPosture,
};
use nomos_cipher_age_vtpm::{AgeVtpmVault, MemoryTargetTransport};
use nomos_core::cipher::Secret;

struct TempVaultDir(PathBuf);

impl TempVaultDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "nomos-cipher-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempVaultDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sample_policy() -> SecretPolicy {
    let targets = vec![
        NodeTarget {
            name: "alpha-coord".to_owned(),
            address: "fd00:db8::10".to_owned(),
            ssh_port: 22,
            remote_user: "ops".to_owned(),
            staging_dir: "/run/nomos-stage".to_owned(),
        },
        NodeTarget {
            name: "beta-worker".to_owned(),
            address: "fd00:db8::20".to_owned(),
            ssh_port: 2222,
            remote_user: "ops".to_owned(),
            staging_dir: "/run/nomos-stage".to_owned(),
        },
    ];
    let entries = vec![
        SecretPolicyEntry {
            key: "loom_api_token".to_owned(),
            target_names: vec!["alpha-coord".to_owned()],
            delivery: DeliveryMode::EnvFile {
                path: "/etc/nomos/loom.env".to_owned(),
                env_var: "NOMOS_API_TOKEN".to_owned(),
            },
            owner: "root".to_owned(),
            group: "nomos".to_owned(),
            mode: "0440".to_owned(),
            restart_units: vec!["nomos-loom.service".to_owned()],
            required: true,
            description: "API bearer seed".to_owned(),
        },
        SecretPolicyEntry {
            key: "wg_mesh_key".to_owned(),
            target_names: vec!["alpha-coord".to_owned(), "beta-worker".to_owned()],
            delivery: DeliveryMode::SystemdCred {
                name: "wg-mesh".to_owned(),
                path: "/etc/credstore/wg-mesh".to_owned(),
            },
            owner: "root".to_owned(),
            group: "root".to_owned(),
            mode: "0400".to_owned(),
            restart_units: vec!["systemd-networkd.service".to_owned()],
            required: true,
            description: "Mesh static identity".to_owned(),
        },
    ];
    SecretPolicy::new(targets, entries).unwrap()
}

#[test]
fn vault_round_trip_never_exposes_plaintext_in_metadata_or_errors() {
    let dir = TempVaultDir::new("roundtrip");
    let vault = AgeVtpmVault::new(&dir.0);

    let init = vault.init().unwrap();
    assert!(init.initialized);
    assert!(init.recipient_public.is_some());

    let needle = "super-sensitive-token-value-99";
    let key_ref = CipherRef::new("loom_api_token").unwrap();
    let meta = vault
        .set(&key_ref, &Secret::new(needle.as_bytes().to_vec()))
        .unwrap();
    assert_eq!(meta.byte_len, needle.len());

    let loaded = vault
        .get(&key_ref.clone().with_digest_hex(&meta.blake3_hex))
        .unwrap();
    assert_eq!(loaded.expose(), needle.as_bytes());

    let wrong_digest = CipherRef::new("loom_api_token")
        .unwrap()
        .with_digest_hex("0".repeat(64));
    let err = vault.get(&wrong_digest).unwrap_err();
    let err_str = err.to_string();
    assert!(!err_str.contains(needle));

    let index_disk = fs::read_to_string(dir.0.join("metadata.json")).unwrap();
    assert!(!index_disk.contains(needle));
}

#[test]
fn policy_rejects_path_traversal_and_unknown_targets() {
    assert!(matches!(
        CipherRef::new("../escape"),
        Err(CipherError::InvalidKey(_))
    ));
    let mut bad = sample_policy();
    bad.entries[0].target_names.push("missing-node".to_owned());
    assert!(matches!(bad.validate(), Err(CipherError::InvalidPolicy(_))));
}

#[test]
fn data_driven_sync_is_idempotent_and_redacts_secrets_from_posture() {
    let dir = TempVaultDir::new("sync");
    let vault = AgeVtpmVault::new(&dir.0);
    vault.init().unwrap();

    let api_secret = "tok-rfc4193-example";
    let wg_secret = "wg-private-example-key";
    vault
        .set(
            &CipherRef::new("loom_api_token").unwrap(),
            &Secret::new(api_secret.as_bytes().to_vec()),
        )
        .unwrap();
    vault
        .set(
            &CipherRef::new("wg_mesh_key").unwrap(),
            &Secret::new(wg_secret.as_bytes().to_vec()),
        )
        .unwrap();

    let policy = sample_policy();
    let transport = MemoryTargetTransport::new().with_vtpm_posture(VtpmPosture {
        node: "alpha-coord".to_owned(),
        has_tpm2: true,
        systemd_creds_available: true,
        libtss2_esys: true,
        libtss2_tcti_device: true,
        detail: "ok".to_owned(),
    });

    let dry = vault.sync_policy(&policy, &transport, None, false).unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.pending_mutations(), 3);
    assert!(dry.actions.iter().all(|a| !a.applied));

    let applied = vault.sync_policy(&policy, &transport, None, true).unwrap();
    assert!(!applied.dry_run);
    assert_eq!(applied.pending_mutations(), 3);
    assert!(applied.actions.iter().all(|a| a.applied));
    assert_eq!(transport.restarted_units().len(), 2);

    let second = vault.sync_policy(&policy, &transport, None, true).unwrap();
    assert_eq!(second.pending_mutations(), 0);
    assert!(
        second
            .actions
            .iter()
            .all(|a| a.kind == SyncActionKind::Skip)
    );

    let posture = vault
        .posture(&policy, &transport, policy.find_target("alpha-coord"))
        .unwrap();
    assert!(posture.vault_initialized);
    assert_eq!(posture.encrypted_entries, 2);
    assert!(posture.missing_keys.is_empty());
    assert!(posture.vtpm.as_ref().is_some_and(VtpmPosture::fully_ready));
    let posture_json = serde_json::to_string(&posture).unwrap();
    assert!(!posture_json.contains(api_secret));
    assert!(!posture_json.contains(wg_secret));
}
