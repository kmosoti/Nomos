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

    /// Issues a time-bounded lease enforcing `max_active_leases` and storing only BLAKE3 digest.
    pub fn issue(&self, req: &IssueLeaseRequest) -> Result<IssuedLease, String> {
        if req.subject.trim().is_empty() || req.scope.trim().is_empty() {
            return Err("subject and scope must be non-empty".to_owned());
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

        let conn = self.conn.lock().map_err(|_| "lease db mutex poisoned")?;
        let active_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM leases WHERE revoked = 0 AND expires_at > ?1",
                params![now as i64],
                |r| r.get(0),
            )
            .map_err(|e| format!("count active leases: {e}"))?;
        if (active_count as usize) >= self.max_active_leases {
            return Err(format!(
                "active lease budget exhausted ({}/{})",
                active_count, self.max_active_leases
            ));
        }

        conn.execute(
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
        let id = conn.last_insert_rowid();

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

    /// Revokes a lease by ID, returning true if an active row matched.
    pub fn revoke(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|_| "lease db mutex poisoned")?;
        let changed = conn
            .execute(
                "UPDATE leases SET revoked = 1 WHERE id = ?1 AND revoked = 0",
                params![id],
            )
            .map_err(|e| format!("revoke lease {id}: {e}"))?;
        Ok(changed > 0)
    }
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
