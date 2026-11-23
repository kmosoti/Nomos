//! SQLite WAL generic credential and workload lease broker + `$CREDENTIALS_DIRECTORY` loader.
//!
//! Stores strictly BLAKE3 token hashes in SQLite WAL tables; raw tokens are
//! wrapped in [`Secret<String>`] and surfaced once upon issuance (invariant N8).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use nomos_core::cipher::Secret;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// Closed allowlist of least-privilege lease scopes permitted by `LeaseBroker`.
///
/// The privileged `"admin"` authority identity is reserved exclusively for the
/// root operator control token and cannot be delegated via child leases.
pub const ALLOWED_LEASE_SCOPES: &[&str] =
    &["batch", "control", "governor", "credential", "observer"];

/// Maximum allowed length for a lease subject identifier.
const MAX_SUBJECT_LEN: usize = 128;

/// Lease issuance request payload for `POST /v1/leases`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IssueLeaseRequest {
    /// Logical subject requesting the lease (for example `worker-alpha`).
    pub subject: String,
    /// Requested lease scope (`batch`, `control`, `credential`).
    pub scope: String,
    /// Requested lifetime in seconds (`1..=86400`).
    pub ttl_secs: u64,
}

/// Non-secret metadata stored per active/revoked lease (never embeds tokens or hashes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    /// Monotonic SQLite row identifier.
    pub id: i64,
    /// Subject name.
    pub subject: String,
    /// Scope label.
    pub scope: String,
    /// Issuance Unix timestamp (seconds).
    pub issued_at: u64,
    /// Expiration Unix timestamp (seconds).
    pub expires_at: u64,
    /// True if explicitly revoked prior to expiration.
    pub revoked: bool,
}

/// One-time response returned to lease callers containing metadata and redacted raw token.
#[derive(Debug)]
pub struct IssuedLease {
    /// Persisted lease metadata.
    pub record: LeaseRecord,
    /// One-time bearer token wrapped inside [`Secret<String>`].
    pub token: Secret<String>,
}

/// Thread-safe SQLite WAL lease store.
#[derive(Clone, Debug)]
pub struct LeaseBroker {
    conn: Arc<Mutex<Connection>>,
    max_active_leases: usize,
}

impl LeaseBroker {
    /// Opens or initializes a SQLite WAL database at `db_path`.
    pub fn open(db_path: impl AsRef<Path>, max_active_leases: usize) -> Result<Self, String> {
        let path = db_path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create lease db dir `{}`: {e}", parent.display()))?;
        }
        let conn =
            Connection::open(path).map_err(|e| format!("open sqlite `{}`: {e}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS leases (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 subject TEXT NOT NULL,
                 scope TEXT NOT NULL,
                 token_blake3 TEXT NOT NULL UNIQUE,
                 issued_at INTEGER NOT NULL,
                 expires_at INTEGER NOT NULL,
                 revoked INTEGER NOT NULL DEFAULT 0
             );",
        )
        .map_err(|e| format!("init lease schema: {e}"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            max_active_leases: max_active_leases.max(1),
        })
    }

    /// Returns total persisted rows in the `leases` table (used to verify bounded DB growth).
    pub fn total_row_count(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|_| "lease db mutex poisoned")?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM leases", [], |r| r.get(0))
            .map_err(|e| format!("count total lease rows: {e}"))?;
        Ok(count.max(0) as usize)
    }

    /// Lists non-revoked, non-expired leases (metadata only, zero hashes/tokens).
    pub fn list_active(&self) -> Result<Vec<LeaseRecord>, String> {
        let now = unix_now_secs();
        let conn = self.conn.lock().map_err(|_| "lease db mutex poisoned")?;
        let mut stmt = conn
            .prepare(
                "SELECT id, subject, scope, issued_at, expires_at, revoked
                 FROM leases
                 WHERE revoked = 0 AND expires_at > ?1
                 ORDER BY id ASC",
            )
            .map_err(|e| format!("prepare list leases: {e}"))?;
        let rows = stmt
            .query_map(params![now as i64], |r| {
                Ok(LeaseRecord {
                    id: r.get(0)?,
                    subject: r.get(1)?,
                    scope: r.get(2)?,
                    issued_at: r.get::<_, i64>(3)? as u64,
                    expires_at: r.get::<_, i64>(4)? as u64,
                    revoked: r.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(|e| format!("query active leases: {e}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read lease row: {e}"))
    }

    /// Issues a time-bounded lease enforcing `max_active_leases`, scope allowlist,
    /// transactional expired/revoked pruning, and WAL checkpointing.
    pub fn issue(&self, req: &IssueLeaseRequest) -> Result<IssuedLease, String> {
        validate_subject(&req.subject)?;
        if !ALLOWED_LEASE_SCOPES.contains(&req.scope.as_str()) {
            return Err(format!(
                "invalid or non-delegable lease scope `{}`",
                req.scope
            ));
        }
        if req.ttl_secs == 0 || req.ttl_secs > 86_400 {
            return Err("ttl_secs must be within 1..=86400".to_owned());
        }

        let now = unix_now_secs();
        let expires = now.saturating_add(req.ttl_secs);
        let mut raw = [0u8; 32];
        SystemRandom::new()
            .fill(&mut raw)
            .map_err(|_| "CSPRNG failure generating lease token".to_owned())?;
        let token_plain = hex::encode(raw);
        raw.fill(0);
        let token_hash = blake3::hash(token_plain.as_bytes()).to_hex().to_string();

        let mut conn = self.conn.lock().map_err(|_| "lease db mutex poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("begin lease tx: {e}"))?;
        let pruned = tx
            .execute(
                "DELETE FROM leases WHERE revoked = 1 OR expires_at <= ?1",
                params![now as i64],
            )
            .map_err(|e| format!("prune expired leases: {e}"))?;

        let active_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM leases WHERE revoked = 0 AND expires_at > ?1",
                params![now as i64],
                |r| r.get(0),
            )
            .map_err(|e| format!("count active leases: {e}"))?;
        if (active_count as usize) >= self.max_active_leases {
            let _ = tx.commit();
            if pruned > 0 {
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            }
            return Err(format!(
                "active lease budget exhausted ({}/{})",
                active_count, self.max_active_leases
            ));
        }

        tx.execute(
            "INSERT INTO leases (subject, scope, token_blake3, issued_at, expires_at, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![
                req.subject,
                req.scope,
                token_hash,
                now as i64,
                expires as i64
            ],
        )
        .map_err(|e| format!("insert lease: {e}"))?;
        let id = tx.last_insert_rowid();
        tx.commit().map_err(|e| format!("commit lease tx: {e}"))?;

        if pruned > 0 {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }

        Ok(IssuedLease {
            record: LeaseRecord {
                id,
                subject: req.subject.clone(),
                scope: req.scope.clone(),
                issued_at: now,
                expires_at: expires,
                revoked: false,
            },
            token: Secret::new(token_plain),
        })
    }

    /// Verifies a raw bearer token by hashing with BLAKE3 and checking expiry/revocation.
    pub fn verify_token(&self, token: &Secret<String>) -> Result<Option<LeaseRecord>, String> {
        let now = unix_now_secs();
        let hash = blake3::hash(token.expose().as_bytes()).to_hex().to_string();
        let conn = self.conn.lock().map_err(|_| "lease db mutex poisoned")?;
        conn.query_row(
            "SELECT id, subject, scope, issued_at, expires_at, revoked
             FROM leases
             WHERE token_blake3 = ?1 AND revoked = 0 AND expires_at > ?2",
            params![hash, now as i64],
            |r| {
                Ok(LeaseRecord {
                    id: r.get(0)?,
                    subject: r.get(1)?,
                    scope: r.get(2)?,
                    issued_at: r.get::<_, i64>(3)? as u64,
                    expires_at: r.get::<_, i64>(4)? as u64,
                    revoked: r.get::<_, i64>(5)? != 0,
                })
            },
        )
        .optional()
        .map_err(|e| format!("verify lease token: {e}"))
    }

    /// Revokes a lease by ID, deleting it and pruning any expired rows.
    pub fn revoke(&self, id: i64) -> Result<bool, String> {
        let now = unix_now_secs();
        let mut conn = self.conn.lock().map_err(|_| "lease db mutex poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("begin revoke tx: {e}"))?;
        let changed = tx
            .execute("DELETE FROM leases WHERE id = ?1", params![id])
            .map_err(|e| format!("revoke lease {id}: {e}"))?;
        let pruned = tx
            .execute(
                "DELETE FROM leases WHERE revoked = 1 OR expires_at <= ?1",
                params![now as i64],
            )
            .map_err(|e| format!("prune leases on revoke: {e}"))?;
        tx.commit().map_err(|e| format!("commit revoke tx: {e}"))?;
        if changed > 0 || pruned > 0 {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        Ok(changed > 0)
    }
}

fn validate_subject(subject: &str) -> Result<(), String> {
    if subject.is_empty() || subject.len() > MAX_SUBJECT_LEN || subject.trim() != subject {
        return Err(format!(
            "subject must be trimmed non-empty ASCII within 1..={MAX_SUBJECT_LEN} bytes"
        ));
    }
    if !subject
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'@' | b'/'))
    {
        return Err(format!(
            "subject `{subject}` contains disallowed characters"
        ));
    }
    Ok(())
}

/// Lists sorted credential filenames staged inside `$CREDENTIALS_DIRECTORY` (names only; N8).
pub fn staged_credential_names() -> Vec<String> {
    let Ok(dir) = std::env::var("CREDENTIALS_DIRECTORY") else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Current Unix timestamp in seconds.
pub fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
