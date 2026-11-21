//! Integration tests for `nomos-cipher` and `nomos-cipher-age-vtpm` (ADR 0018, N8).

use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nomos_cipher::{
    CipherError, CipherPort, CipherRef, CipherVault, DeliveryMode, NodeTarget, SecretPolicy,
    SecretPolicyEntry, SyncActionKind, VtpmPosture,
};
use nomos_cipher_age_vtpm::{
    AgeVtpmVault, HybridIdentity, MemoryTargetTransport, MockCredSealer, SshTargetTransport,
    SystemdCredsSealer, VaultCipherMode, VaultKeyStorage, decrypt_hybrid, encrypt_hybrid,
    is_hybrid_ciphertext, render_entry_payload_with_sealer,
};
use nomos_core::cipher::{Secret, SecretScrub, scrub_bytes};

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
fn pq_hybrid_and_classical_round_trip_redact_plaintext() {
    let dir = TempVaultDir::new("pq-roundtrip");
    let sealer = Arc::new(MockCredSealer::default());
    let vault = AgeVtpmVault::new(&dir.0)
        .with_key_storage(VaultKeyStorage::Tpm2Sealed)
        .with_cipher_mode(VaultCipherMode::HybridMlKem768X25519)
        .with_cred_sealer(sealer);

    let init = vault.init().unwrap();
    assert!(init.initialized);
    assert!(
        init.recipient_public
            .as_deref()
            .unwrap()
            .starts_with("nomos1pq1:")
    );

    let needle = "super-sensitive-pq-token-99";
    let key_ref = CipherRef::new("loom_api_token").unwrap();
    let meta = vault
        .set(&key_ref, &Secret::new(needle.as_bytes().to_vec()))
        .unwrap();
    assert_eq!(meta.byte_len, needle.len());

    let raw_ct = fs::read(dir.0.join("entries/loom_api_token.age")).unwrap();
    assert!(is_hybrid_ciphertext(&raw_ct));
    assert!(!raw_ct.windows(needle.len()).any(|w| w == needle.as_bytes()));

    let loaded = vault
        .get(&key_ref.clone().with_digest_hex(&meta.blake3_hex))
        .unwrap();
    assert_eq!(loaded.expose(), needle.as_bytes());

    let wrong_digest = CipherRef::new("loom_api_token")
        .unwrap()
        .with_digest_hex("0".repeat(64));
    let err = vault.get(&wrong_digest).unwrap_err();
    assert!(!err.to_string().contains(needle));

    // Tampering a ciphertext byte must fail AEAD authentication.
    let id = HybridIdentity::generate().unwrap();
    let mut ct = encrypt_hybrid(&id.to_public(), b"quantum-safe-payload").unwrap();
    assert_eq!(decrypt_hybrid(&id, &ct).unwrap(), b"quantum-safe-payload");
    let last = ct.len() - 1;
    ct[last] ^= 0xFF;
    assert!(decrypt_hybrid(&id, &ct).is_err());

    // Classical X25519 dev mode continues to round-trip cleanly.
    let classic_dir = TempVaultDir::new("classic");
    let classic = AgeVtpmVault::new(&classic_dir.0)
        .with_cipher_mode(VaultCipherMode::ClassicalX25519)
        .allow_plaintext_key(true);
    let classic_init = classic.init().unwrap();
    assert!(
        classic_init
            .recipient_public
            .as_deref()
            .unwrap()
            .starts_with("age1")
    );
    let m = classic
        .set(&key_ref, &Secret::new(b"classic-val".to_vec()))
        .unwrap();
    assert_eq!(
        classic
            .get(&key_ref.with_digest_hex(&m.blake3_hex))
            .unwrap()
            .expose(),
        b"classic-val"
    );
}

#[test]
fn tpm2_production_mode_refuses_plaintext_key_and_encrypts_systemd_cred() {
    let dir = TempVaultDir::new("tpm2-refuse");
    let dev_vault = AgeVtpmVault::new(&dir.0).allow_plaintext_key(true);
    dev_vault.init().unwrap();
    let key_ref = CipherRef::new("wg_mesh_key").unwrap();
    dev_vault
        .set(&key_ref, &Secret::new(b"plain-dev-secret".to_vec()))
        .unwrap();

    // Production Tpm2Sealed vault MUST reject unsealed plaintext vault.key.
    let prod_vault =
        AgeVtpmVault::new(&dir.0).with_cred_sealer(Arc::new(MockCredSealer::default()));
    let err = prod_vault.get(&key_ref).unwrap_err();
    assert!(matches!(err, CipherError::Crypto(_)));
    assert!(err.to_string().contains("Tpm2Sealed"));

    // Verify SystemdCredsSealer argv contract and SystemdCred delivery encryption.
    assert_eq!(
        SystemdCredsSealer::seal_tpm2_args(),
        &["encrypt", "--with-key=tpm2", "-", "-"]
    );
    assert_eq!(
        SystemdCredsSealer::encrypt_credential_args("wg-mesh").unwrap(),
        vec!["encrypt", "--name=wg-mesh", "--with-key=auto", "-", "-"]
    );

    let policy = sample_policy();
    let cred_entry = policy.find_entry("wg_mesh_key").unwrap();
    let raw_secret = b"never-deliver-this-raw-plaintext";
    let rendered = render_entry_payload_with_sealer(
        cred_entry,
        &Secret::new(raw_secret.to_vec()),
        &MockCredSealer::default(),
    )
    .unwrap();
    assert_ne!(rendered.expose().as_slice(), raw_secret);
    assert!(
        !rendered
            .expose()
            .windows(raw_secret.len())
            .any(|w| w == raw_secret)
    );
}

#[test]
fn ssh_policy_rejects_metacharacters_and_enforces_pinned_strict_quoting() {
    assert!(matches!(
        CipherRef::new("../escape"),
        Err(CipherError::InvalidKey(_))
    ));
    let base = sample_policy();

    let mut bad_owner = base.clone();
    bad_owner.entries[0].owner = "root;id".to_owned();
    assert!(matches!(
        bad_owner.validate(),
        Err(CipherError::InvalidPolicy(_))
    ));

    let mut bad_group = base.clone();
    bad_group.entries[0].group = "$(whoami)".to_owned();
    assert!(matches!(
        bad_group.validate(),
        Err(CipherError::InvalidPolicy(_))
    ));

    let mut bad_unit = base.clone();
    bad_unit.entries[0].restart_units = vec!["nomos.service`reboot`".to_owned()];
    assert!(matches!(
        bad_unit.validate(),
        Err(CipherError::InvalidPolicy(_))
    ));

    let mut bad_path = base.clone();
    bad_path.entries[0].delivery = DeliveryMode::RawFile {
        path: "/etc/nomos/$(touch /tmp/pwn)".to_owned(),
    };
    assert!(matches!(
        bad_path.validate(),
        Err(CipherError::InvalidPolicy(_))
    ));

    let mut bad_space = base.clone();
    bad_space.targets[0].staging_dir = "/run/nomos stage".to_owned();
    assert!(matches!(
        bad_space.validate(),
        Err(CipherError::InvalidPolicy(_))
    ));

    // SSH transport must refuse unpinned known_hosts and enforce strict quoting + umask 077.
    let unpinned = SshTargetTransport::default();
    assert!(unpinned.build_ssh_args(&base.targets[0]).is_err());

    let pinned = SshTargetTransport::new("/etc/nomos/known_hosts");
    let args = pinned.build_ssh_args(&base.targets[0]).unwrap();
    assert!(args.contains(&"StrictHostKeyChecking=yes".to_owned()));
    assert!(args.contains(&"UserKnownHostsFile=/etc/nomos/known_hosts".to_owned()));

    let script = SshTargetTransport::build_write_remote_script(
        &base.targets[0],
        &base.entries[0],
        "deadbeef01234567",
    )
    .unwrap();
    assert!(script.starts_with("umask 077 &&"));
    assert!(script.contains("'/run/nomos-stage/loom_api_token.deadbeef01234567.stage'"));
    assert!(script.contains("'/etc/nomos/loom.env'"));
}

#[test]
fn data_driven_sync_is_idempotent_and_secret_drop_scrubs() {
    let dir = TempVaultDir::new("sync");
    let vault = AgeVtpmVault::new(&dir.0)
        .with_key_storage(VaultKeyStorage::Tpm2Sealed)
        .with_cred_sealer(Arc::new(MockCredSealer::default()));
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

    let applied = vault.sync_policy(&policy, &transport, None, true).unwrap();
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
    let posture_json = serde_json::to_string(&posture).unwrap();
    assert!(!posture_json.contains(api_secret));
    assert!(!posture_json.contains(wg_secret));

    // Verify Secret<T> invokes SecretScrub on Drop.
    struct Spy<'a>(&'a Cell<bool>, [u8; 4]);
    impl SecretScrub for Spy<'_> {
        fn scrub(&mut self) {
            scrub_bytes(&mut self.1);
            assert_eq!(self.1, [0; 4]);
            self.0.set(true);
        }
    }
    let flag = Cell::new(false);
    {
        let s = Secret::new(Spy(&flag, *b"abcd"));
        assert_eq!(&s.expose().1, b"abcd");
    }
    assert!(flag.get());
}
