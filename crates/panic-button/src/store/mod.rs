// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Panic Button SQLite persistence for identity, certificates, alarms and audit.
//!
//! The store applies the core's [`PersistOp`]s in order and rebuilds a
//! [`PersistedState`] at start-up. It mirrors [`PersistedState::apply`]
//! exactly (tested below), so a restart sees what the core persisted (P3).
//!
//! SQLite runs in WAL mode. Security-relevant operations
//! ([`PersistOp::must_sync`]) are committed with `synchronous = FULL`
//! before the runtime releases dependent outputs.
//!
//! Secret material (device keys, group keys) goes through [`protect`]: on
//! Windows it is encrypted with DPAPI in machine scope (ADR 0010). On Linux
//! and macOS it is stored as written, and the database is a 0600 file owned
//! by its user (`spec/04-security.md`, Key storage).

use std::path::Path;

use crate::core::io::{AuditEvent, LocalAck};
use crate::core::persist::{
    AlarmLocal, PersistOp, PersistedState, PracticeInfo, StoredAlarm, StoredGroupKey,
};
use crate::crypto::{DeviceKeys, GroupKey};
use crate::proto::{CertSerial, MsgId, PracticeId};
use rusqlite::{Connection, OptionalExtension, params};

/// Schema version (`PRAGMA user_version`).
pub const SCHEMA_VERSION: i64 = 3;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("database schema version {0} is newer than this program supports")]
    TooNew(i64),
    #[error("corrupt record in {0}")]
    Corrupt(&'static str),
    #[error("cannot encrypt {0} for this computer: {1}")]
    Seal(&'static str, String),
    #[error("cannot decrypt {0} (was the database copied from another computer?): {1}")]
    Unseal(&'static str, String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// The device's SQLite database.
#[derive(Debug)]
pub struct Store {
    db: Connection,
}

fn id16(b: &[u8], what: &'static str) -> Result<[u8; 16]> {
    <[u8; 16]>::try_from(b).map_err(|_| StoreError::Corrupt(what))
}

fn id32(b: &[u8], what: &'static str) -> Result<[u8; 32]> {
    <[u8; 32]>::try_from(b).map_err(|_| StoreError::Corrupt(what))
}

fn ack_code(a: Option<LocalAck>) -> i64 {
    match a {
        None => 0,
        Some(LocalAck::Seen) => 1,
        Some(LocalAck::Responding) => 2,
    }
}

fn ack_from(c: i64) -> Option<LocalAck> {
    match c {
        1 => Some(LocalAck::Seen),
        2 => Some(LocalAck::Responding),
        _ => None,
    }
}

impl Store {
    /// Opens (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let s = Self::init(Connection::open(path)?)?;
        // Holds key material: owner-only (the WAL and shared-memory files
        // inherit the database's mode).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| StoreError::Corrupt("cannot restrict database permissions"))?;
        }
        Ok(s)
    }

    /// An in-memory database, for tests and the simulator.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "FULL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::TooNew(version));
        }
        if version < 1 {
            db.execute_batch(
                "BEGIN;
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
                 CREATE TABLE certs (raw BLOB PRIMARY KEY, seq INTEGER NOT NULL);
                 CREATE TABLE revocations (raw BLOB PRIMARY KEY, seq INTEGER NOT NULL);
                 CREATE TABLE group_keys (
                     epoch INTEGER NOT NULL, key BLOB NOT NULL, at INTEGER NOT NULL,
                     PRIMARY KEY (epoch, key));
                 CREATE TABLE keyshares (raw BLOB PRIMARY KEY, seq INTEGER NOT NULL);
                 CREATE TABLE alarm_envelopes (
                     alarm_id BLOB NOT NULL, msg_id BLOB NOT NULL, bytes BLOB NOT NULL,
                     PRIMARY KEY (alarm_id, msg_id));
                 CREATE TABLE alarm_local (
                     alarm_id BLOB PRIMARY KEY, alerted INTEGER NOT NULL,
                     local_ack INTEGER NOT NULL, history_only INTEGER NOT NULL,
                     first_seen INTEGER NOT NULL);
                 CREATE TABLE processed (msg_id BLOB PRIMARY KEY, at INTEGER NOT NULL);
                 CREATE TABLE audit (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL,
                     kind TEXT NOT NULL, detail TEXT NOT NULL);
                 CREATE INDEX audit_at ON audit (at);
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        if version < 2 {
            db.execute_batch(
                "BEGIN;
                 CREATE TABLE drill_reports (
                     alarm_id BLOB PRIMARY KEY, started_at INTEGER NOT NULL,
                     received INTEGER NOT NULL, expected INTEGER NOT NULL,
                     csv TEXT NOT NULL);
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        }
        if version < 3 {
            db.execute_batch(
                "BEGIN;
                 ALTER TABLE drill_reports ADD COLUMN skipped INTEGER NOT NULL DEFAULT 0;
                 PRAGMA user_version = 3;
                 COMMIT;",
            )?;
        }
        Ok(Store { db })
    }

    fn set_meta(&self, key: &str, value: &[u8]) -> Result<()> {
        set_meta(&self.db, key, value)
    }

    fn meta(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Saves the device's own key pair.
    pub fn save_identity(&self, keys: &DeviceKeys) -> Result<()> {
        let (s, k) = keys.to_secrets();
        let mut v = zeroize::Zeroizing::new(Vec::with_capacity(64));
        v.extend_from_slice(&s);
        v.extend_from_slice(&k);
        self.set_meta("identity", &protect::seal(&v, "identity")?)
    }

    /// Loads the device's key pair, encrypting it first if it was stored
    /// before encryption (`protect`).
    pub fn load_identity(&self) -> Result<Option<DeviceKeys>> {
        let Some(stored) = self.meta("identity")? else {
            return Ok(None);
        };
        let u = protect::unseal(&stored, "identity")?;
        let v = &u.plain;
        if v.len() != 64 {
            return Err(StoreError::Corrupt("identity"));
        }
        let keys =
            DeviceKeys::from_secrets(id32(&v[..32], "identity")?, id32(&v[32..], "identity")?);
        if u.rewrite {
            self.save_identity(&keys)?;
        }
        Ok(Some(keys))
    }

    /// Applies one operation. Mirrors [`PersistedState::apply`].
    pub fn apply(&self, op: &PersistOp) -> Result<()> {
        apply_on(&self.db, op)
    }

    /// Applies a batch in one transaction.
    pub fn apply_all<'a>(&mut self, ops: impl IntoIterator<Item = &'a PersistOp>) -> Result<()> {
        let tx = self.db.transaction()?;
        for op in ops {
            apply_on(&tx, op)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records an audit entry (FR-LOG-001). `detail` must not contain
    /// secrets; [`AuditEvent`]'s `Debug` form never does.
    pub fn audit(&self, at: u64, e: &AuditEvent) -> Result<()> {
        let kind = format!("{e:?}");
        let kind = kind.split([' ', '{', '(']).next().unwrap_or("").to_owned();
        self.db.execute(
            "INSERT INTO audit (at, kind, detail) VALUES (?1, ?2, ?3)",
            params![at as i64, kind, format!("{e:?}")],
        )?;
        Ok(())
    }

    /// Keeps a drill report (FR-DRL-007); a later report for the same drill
    /// replaces it.
    pub fn save_drill_report(&self, r: &crate::core::drill::DrillReport) -> Result<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO drill_reports VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![
                &r.alarm_id.0[..],
                r.started_at as i64,
                r.received() as i64,
                r.rows.len() as i64,
                r.to_csv()
            ],
        )?;
        Ok(())
    }

    /// Records that the scheduled drill at `slot` (Unix ms) did not run
    /// (FR-DRL-006).
    pub fn save_skipped_drill(&self, slot: u64) -> Result<()> {
        let mut id = [0xffu8; 16];
        id[..8].copy_from_slice(&slot.to_be_bytes());
        self.db.execute(
            "INSERT OR IGNORE INTO drill_reports VALUES (?1, ?2, 0, 0, '', 1)",
            params![&id[..], slot as i64],
        )?;
        Ok(())
    }

    /// Drill reports, newest first.
    pub fn drill_reports(&self) -> Result<Vec<StoredDrillReport>> {
        let mut s = self.db.prepare(
            "SELECT alarm_id, started_at, received, expected, csv, skipped FROM drill_reports
             ORDER BY started_at DESC",
        )?;
        let rows = s.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, started_at, received, expected, csv, skipped) = row?;
            out.push(StoredDrillReport {
                alarm_id: crate::proto::MsgId(id16(&id, "drill report")?),
                started_at: started_at as u64,
                received: received as usize,
                expected: expected as usize,
                csv,
                skipped,
            });
        }
        Ok(out)
    }

    /// Deletes drill reports started before `before` (400-day retention).
    pub fn prune_drill_reports(&self, before: u64) -> Result<usize> {
        Ok(self.db.execute(
            "DELETE FROM drill_reports WHERE started_at < ?1",
            [before as i64],
        )?)
    }

    /// Deletes audit entries older than `before` (FR-LOG-002 retention).
    pub fn prune_audit(&self, before: u64) -> Result<usize> {
        Ok(self
            .db
            .execute("DELETE FROM audit WHERE at < ?1", [before as i64])?)
    }

    /// Audit entries in time order: (at, kind, detail).
    pub fn audit_entries(&self) -> Result<Vec<(u64, String, String)>> {
        let mut st = self
            .db
            .prepare("SELECT at, kind, detail FROM audit ORDER BY at, id")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Rebuilds the state the core persisted.
    pub fn load(&self) -> Result<PersistedState> {
        let mut s = PersistedState::default();
        if let Some(v) = self.meta("practice")? {
            if v.len() < 48 {
                return Err(StoreError::Corrupt("practice"));
            }
            s.practice = Some(PracticeInfo {
                practice_id: PracticeId(id16(&v[..16], "practice")?),
                root_pub: id32(&v[16..48], "practice")?,
                name: String::from_utf8(v[48..].to_vec())
                    .map_err(|_| StoreError::Corrupt("practice"))?,
            });
        }
        if let Some(v) = self.meta("own_serial")? {
            s.own_serial = Some(CertSerial(id16(&v, "own_serial")?));
        }
        s.config = self.meta("config")?;
        s.own_settings = self.meta("own_settings")?;
        for (table, out) in [
            ("certs", &mut s.certs),
            ("revocations", &mut s.revocations),
            ("keyshares", &mut s.keyshares),
        ] {
            let mut st = self
                .db
                .prepare(&format!("SELECT raw FROM {table} ORDER BY seq"))?;
            for r in st.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
                out.push(r?);
            }
        }
        {
            let mut st = self
                .db
                .prepare("SELECT epoch, key, at FROM group_keys ORDER BY rowid")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            let mut rewrite = Vec::new();
            for row in rows {
                let (epoch, stored, at) = row?;
                let u = protect::unseal(&stored, "group_keys")?;
                let key = GroupKey::from_bytes(id32(&u.plain, "group_keys")?);
                if u.rewrite {
                    rewrite.push((epoch, stored, protect::seal(key.as_bytes(), "group_keys")?));
                }
                s.group_keys.push(StoredGroupKey {
                    epoch: epoch as u64,
                    key,
                    at: at as u64,
                });
            }
            for (epoch, stored, sealed) in rewrite {
                self.db.execute(
                    "UPDATE group_keys SET key = ?1 WHERE epoch = ?2 AND key = ?3",
                    params![sealed, epoch, stored],
                )?;
            }
        }
        {
            let mut st = self
                .db
                .prepare("SELECT alarm_id, msg_id, bytes FROM alarm_envelopes")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?;
            for row in rows {
                let (a, m, b) = row?;
                let e: &mut StoredAlarm = s
                    .alarms
                    .entry(MsgId(id16(&a, "alarm_envelopes")?))
                    .or_default();
                e.envelopes.insert(MsgId(id16(&m, "alarm_envelopes")?), b);
            }
        }
        {
            let mut st = self.db.prepare(
                "SELECT alarm_id, alerted, local_ack, history_only, first_seen FROM alarm_local",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?;
            for row in rows {
                let (a, alerted, ack, history_only, first_seen) = row?;
                s.alarms
                    .entry(MsgId(id16(&a, "alarm_local")?))
                    .or_default()
                    .local = AlarmLocal {
                    alerted,
                    local_ack: ack_from(ack),
                    history_only,
                    first_seen: first_seen as u64,
                };
            }
        }
        {
            let mut st = self.db.prepare("SELECT msg_id, at FROM processed")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (m, at) = row?;
                s.processed.insert(MsgId(id16(&m, "processed")?), at as u64);
            }
        }
        Ok(s)
    }
}

fn set_meta(db: &Connection, key: &str, value: &[u8]) -> Result<()> {
    db.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Appends a raw record to a set-like table, keeping insertion order (the
/// core relies on certs and revocations coming back in the order written).
fn insert_raw(db: &Connection, table: &str, raw: &[u8]) -> Result<()> {
    let seq: i64 = db.query_row(
        &format!("SELECT COALESCE(MAX(seq), 0) + 1 FROM {table}"),
        [],
        |r| r.get(0),
    )?;
    db.execute(
        &format!("INSERT OR IGNORE INTO {table} (raw, seq) VALUES (?1, ?2)"),
        params![raw, seq],
    )?;
    Ok(())
}

fn apply_on(db: &Connection, op: &PersistOp) -> Result<()> {
    match op {
        PersistOp::SetPractice(p) => {
            let mut v = Vec::new();
            v.extend_from_slice(&p.practice_id.0);
            v.extend_from_slice(&p.root_pub);
            v.extend_from_slice(p.name.as_bytes());
            set_meta(db, "practice", &v)?;
        }
        PersistOp::SetOwnSerial(s) => set_meta(db, "own_serial", &s.0)?,
        PersistOp::AddCert(c) => insert_raw(db, "certs", c)?,
        PersistOp::AddRevocation(r) => insert_raw(db, "revocations", r)?,
        PersistOp::AddKeyShare(k) => insert_raw(db, "keyshares", k)?,
        PersistOp::AddGroupKey(k) => {
            // Sealed values differ each time, so the primary key cannot
            // drop a repeat: compare the keys themselves.
            let mut st = db.prepare("SELECT key FROM group_keys WHERE epoch = ?1")?;
            let mut known = false;
            for stored in st.query_map([k.epoch as i64], |r| r.get::<_, Vec<u8>>(0))? {
                if protect::unseal(&stored?, "group_keys")?.plain[..] == k.key.as_bytes()[..] {
                    known = true;
                    break;
                }
            }
            if !known {
                db.execute(
                    "INSERT OR IGNORE INTO group_keys (epoch, key, at) VALUES (?1, ?2, ?3)",
                    params![
                        k.epoch as i64,
                        protect::seal(k.key.as_bytes(), "group_keys")?,
                        k.at as i64
                    ],
                )?;
            }
        }
        PersistOp::SetConfig(c) => set_meta(db, "config", c)?,
        PersistOp::SetOwnSettings(c) => set_meta(db, "own_settings", c)?,
        PersistOp::AddAlarmEnvelope {
            alarm_id,
            msg_id,
            bytes,
        } => {
            db.execute(
                "INSERT OR IGNORE INTO alarm_envelopes (alarm_id, msg_id, bytes)
                     VALUES (?1, ?2, ?3)",
                params![&alarm_id.0[..], &msg_id.0[..], bytes],
            )?;
        }
        PersistOp::SetAlarmLocal { alarm_id, local } => {
            db.execute(
                "INSERT INTO alarm_local (alarm_id, alerted, local_ack, history_only, first_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (alarm_id) DO UPDATE SET alerted = excluded.alerted,
                       local_ack = excluded.local_ack, history_only = excluded.history_only,
                       first_seen = excluded.first_seen",
                params![
                    &alarm_id.0[..],
                    local.alerted,
                    ack_code(local.local_ack),
                    local.history_only,
                    local.first_seen as i64
                ],
            )?;
        }
        PersistOp::Processed { msg_id, at } => {
            db.execute(
                "INSERT OR IGNORE INTO processed (msg_id, at) VALUES (?1, ?2)",
                params![&msg_id.0[..], *at as i64],
            )?;
        }
        PersistOp::PruneAlarm(id) => {
            db.execute(
                "DELETE FROM alarm_envelopes WHERE alarm_id = ?1",
                [&id.0[..]],
            )?;
            db.execute("DELETE FROM alarm_local WHERE alarm_id = ?1", [&id.0[..]])?;
        }
        PersistOp::PruneProcessedBefore(t) => {
            db.execute("DELETE FROM processed WHERE at < ?1", [*t as i64])?;
        }
    }
    Ok(())
}

/// A kept drill report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDrillReport {
    pub alarm_id: crate::proto::MsgId,
    pub started_at: u64,
    pub received: usize,
    pub expected: usize,
    pub csv: String,
    /// A scheduled drill that did not run; `started_at` is the slot.
    pub skipped: bool,
}

mod protect;

#[cfg(test)]
mod tests;
