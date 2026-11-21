//! Data-driven delivery planning and atomic policy execution.

use std::collections::{BTreeMap, BTreeSet};

use nomos_cipher::{
    CipherError, CipherRef, CipherResult, CipherVault, DeliveryMode, NodeTarget, SecretPolicy,
    SecretPolicyEntry, SyncAction, SyncActionKind, SyncPlan, TargetTransport,
};
use nomos_core::cipher::Secret;
use zeroize::Zeroize;

use crate::vault::blake3_hex;

/// Renders a vault secret according to the entry's [`DeliveryMode`] inside [`Secret`].
pub fn render_entry_payload(
    entry: &SecretPolicyEntry,
    plaintext: &Secret<Vec<u8>>,
) -> CipherResult<Secret<Vec<u8>>> {
    let raw = plaintext.expose();
    match &entry.delivery {
        DeliveryMode::EnvFile { env_var, .. } => {
            let text = std::str::from_utf8(raw).map_err(|_| {
                CipherError::InvalidPolicy(format!(
                    "secret `{}` for EnvFile delivery must be valid UTF-8",
                    entry.key
                ))
            })?;
            let trimmed = text.trim_end_matches(&['\r', '\n'][..]);
            if trimmed.contains('\n') || trimmed.contains('\r') {
                return Err(CipherError::InvalidPolicy(format!(
                    "secret `{}` for EnvFile delivery cannot span multiple lines",
                    entry.key
                )));
            }
            let rendered = format!("{env_var}={trimmed}\n");
            Ok(Secret::new(rendered.into_bytes()))
        }
        DeliveryMode::RawFile { .. } | DeliveryMode::SystemdCred { .. } => {
            Ok(Secret::new(raw.clone()))
        }
    }
}

/// Computes deterministic sync actions over all matching `policy` targets and entries.
///
/// Never mutates remote targets unless `confirm_apply` is `true`, and never
/// places decrypted payloads inside [`SyncPlan`] or [`SyncAction`].
pub fn plan_and_execute_sync(
    vault: &dyn CipherVault,
    policy: &SecretPolicy,
    transport: &dyn TargetTransport,
    target_filter: Option<&str>,
    confirm_apply: bool,
) -> CipherResult<SyncPlan> {
    policy.validate()?;

    let mut targets: Vec<&NodeTarget> = policy
        .targets
        .iter()
        .filter(|t| target_filter.is_none_or(|wanted| wanted == t.name))
        .collect();
    targets.sort_by(|a, b| a.name.cmp(&b.name));

    if let Some(wanted) = target_filter
        && targets.is_empty()
    {
        return Err(CipherError::InvalidPolicy(format!(
            "target filter `{wanted}` matched no target in SecretPolicy"
        )));
    }

    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    let mut pending_restarts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for target in &targets {
        let mut entries = policy.entries_for_target(&target.name);
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        for entry in entries {
            let reference = CipherRef::new(&entry.key)?;
            let plaintext = match vault.get(&reference) {
                Ok(secret) => secret,
                Err(CipherError::NotFound(_)) if !entry.required => {
                    warnings.push(format!(
                        "optional secret `{}` absent from vault; skipping target `{}`",
                        entry.key, target.name
                    ));
                    continue;
                }
                Err(err) => return Err(err),
            };

            let rendered = render_entry_payload(entry, &plaintext)?;
            let mut desired_digest = blake3_hex(rendered.expose());
            let remote = transport.inspect_remote(target, entry)?;

            let kind = if !remote.exists {
                SyncActionKind::Create
            } else if remote.blake3_hex.as_deref() != Some(desired_digest.as_str())
                || remote.mode.as_deref() != Some(entry.mode.as_str())
            {
                SyncActionKind::Update
            } else {
                SyncActionKind::Skip
            };
            desired_digest.zeroize();

            let mut applied = false;
            if confirm_apply && matches!(kind, SyncActionKind::Create | SyncActionKind::Update) {
                transport.write_remote(target, entry, &rendered)?;
                applied = true;
                for unit in &entry.restart_units {
                    pending_restarts
                        .entry(target.name.clone())
                        .or_default()
                        .insert(unit.clone());
                }
            }

            let reason = match kind {
                SyncActionKind::Create => "remote destination absent".to_owned(),
                SyncActionKind::Update => "remote digest or permissions differ".to_owned(),
                SyncActionKind::Skip => "remote digest and permissions match".to_owned(),
            };

            actions.push(SyncAction {
                target: target.name.clone(),
                key: entry.key.clone(),
                destination: entry.delivery.destination_path().to_owned(),
                kind,
                reason,
                restart_units: entry.restart_units.clone(),
                applied,
            });
        }
    }

    if confirm_apply {
        for (target_name, units_set) in pending_restarts {
            if let Some(target) = policy.find_target(&target_name)
                && !units_set.is_empty()
            {
                let units: Vec<String> = units_set.into_iter().collect();
                transport.restart_units(target, &units)?;
            }
        }
    }

    Ok(SyncPlan {
        dry_run: !confirm_apply,
        actions,
        warnings,
    })
}
