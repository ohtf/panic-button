// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

use crate::core::io::{AuditEvent, LocalAck};
use crate::core::persist::{AlarmLocal, PersistOp, PersistedState, PracticeInfo, StoredGroupKey};
use crate::crypto::{DeviceKeys, GroupKey};
use crate::proto::{CertSerial, MsgId, PracticeId, Source};

use super::Store;

fn ops() -> Vec<PersistOp> {
    let a = MsgId([1; 16]);
    let b = MsgId([2; 16]);
    vec![
        PersistOp::SetPractice(PracticeInfo {
            practice_id: PracticeId([3; 16]),
            root_pub: [4; 32],
            name: "Riverside Surgery".into(),
        }),
        PersistOp::SetOwnSerial(CertSerial([5; 16])),
        PersistOp::AddCert(vec![1, 2, 3]),
        PersistOp::AddCert(vec![0]),
        PersistOp::AddCert(vec![1, 2, 3]),
        PersistOp::AddRevocation(vec![9]),
        PersistOp::AddGroupKey(StoredGroupKey {
            epoch: 0,
            key: GroupKey::from_bytes([6; 32]),
            at: 10,
        }),
        PersistOp::AddGroupKey(StoredGroupKey {
            epoch: 1,
            key: GroupKey::from_bytes([7; 32]),
            at: 20,
        }),
        PersistOp::AddGroupKey(StoredGroupKey {
            epoch: 1,
            key: GroupKey::from_bytes([7; 32]),
            at: 30,
        }),
        PersistOp::AddKeyShare(vec![8, 8]),
        PersistOp::SetConfig(vec![1]),
        PersistOp::SetConfig(vec![2]),
        PersistOp::SetOwnSettings(vec![3]),
        PersistOp::AddAlarmEnvelope {
            alarm_id: a,
            msg_id: a,
            bytes: vec![1],
        },
        PersistOp::AddAlarmEnvelope {
            alarm_id: a,
            msg_id: b,
            bytes: vec![2],
        },
        PersistOp::AddAlarmEnvelope {
            alarm_id: a,
            msg_id: b,
            bytes: vec![99],
        },
        PersistOp::SetAlarmLocal {
            alarm_id: a,
            local: AlarmLocal {
                alerted: true,
                local_ack: Some(LocalAck::Responding),
                history_only: false,
                first_seen: 5,
            },
        },
        PersistOp::SetAlarmLocal {
            alarm_id: b,
            local: AlarmLocal::default(),
        },
        PersistOp::Processed { msg_id: a, at: 100 },
        PersistOp::Processed { msg_id: a, at: 200 },
        PersistOp::Processed { msg_id: b, at: 50 },
        PersistOp::PruneProcessedBefore(60),
        PersistOp::PruneAlarm(b),
    ]
}

#[test]
fn store_matches_persisted_state_apply() {
    let mut expected = PersistedState::default();
    let store = Store::open_in_memory().expect("open");
    for op in ops() {
        expected.apply(op.clone());
        store.apply(&op).expect("apply");
    }
    assert_eq!(store.load().expect("load"), expected);
}

#[test]
fn batches_survive_reopen() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("panic-button.db");
    let mut expected = PersistedState::default();
    {
        let mut store = Store::open(&path).expect("open");
        let ops = ops();
        store.apply_all(ops.iter()).expect("batch");
        for op in ops {
            expected.apply(op);
        }
        let keys = DeviceKeys::from_secrets([1; 32], [2; 32]);
        store.save_identity(&keys).expect("identity");
        store
            .audit(
                7,
                &AuditEvent::NearMiss {
                    source: Source::Button,
                    duration_ms: 800,
                },
            )
            .expect("audit");
    }
    let store = Store::open(&path).expect("reopen");
    assert_eq!(store.load().expect("load"), expected);
    let keys = store.load_identity().expect("identity").expect("present");
    assert_eq!(
        keys.device_id(),
        DeviceKeys::from_secrets([1; 32], [2; 32]).device_id()
    );
    let audit = store.audit_entries().expect("audit");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].1, "NearMiss");
    assert_eq!(store.prune_audit(8).expect("prune"), 1);
}

#[test]
fn refuses_newer_schema() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("panic-button.db");
    {
        let c = rusqlite::Connection::open(&path).expect("open");
        c.pragma_update(None, "user_version", 99).expect("pragma");
    }
    assert!(matches!(
        Store::open(&path),
        Err(super::StoreError::TooNew(99))
    ));
}

#[test]
fn drill_reports_are_kept_newest_first_and_pruned() {
    use crate::core::drill::{DrillReport, DrillRow};
    let s = Store::open_in_memory().expect("open");
    let report = |id: u8, at: u64| DrillReport {
        alarm_id: MsgId([id; 16]),
        started_at: at,
        rows: vec![DrillRow {
            device: crate::proto::DeviceId([id; 32]),
            label: "PC".into(),
            room: "Room 1".into(),
            received: true,
            rtt_ms: Some(12),
            path: None,
            audio: None,
        }],
    };
    s.save_drill_report(&report(1, 100)).expect("save");
    s.save_drill_report(&report(2, 200)).expect("save");
    let all = s.drill_reports().expect("list");
    assert_eq!(
        all.iter().map(|r| r.started_at).collect::<Vec<_>>(),
        [200, 100]
    );
    assert_eq!((all[0].received, all[0].expected), (1, 1));
    assert!(all[0].csv.contains("Room 1"));
    assert_eq!(s.prune_drill_reports(150).expect("prune"), 1);
    assert_eq!(s.drill_reports().expect("list").len(), 1);
    s.save_skipped_drill(300).expect("skip");
    s.save_skipped_drill(300).expect("idempotent");
    let all = s.drill_reports().expect("list");
    assert_eq!(all.len(), 2);
    assert!(all[0].skipped && all[0].started_at == 300);
    assert!(!all[1].skipped);
}

#[test]
fn version_1_databases_gain_drill_reports() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("panic-button.db");
    drop(Store::open(&path).expect("open"));
    {
        let c = rusqlite::Connection::open(&path).expect("open");
        c.execute_batch("DROP TABLE drill_reports; PRAGMA user_version = 1;")
            .expect("downgrade");
    }
    let s = Store::open(&path).expect("reopen migrates");
    assert!(s.drill_reports().expect("list").is_empty());
}

fn raw_identity(s: &Store) -> Vec<u8> {
    s.db.query_row("SELECT value FROM meta WHERE key = 'identity'", [], |r| {
        r.get(0)
    })
    .expect("identity row")
}

fn raw_group_keys(s: &Store) -> Vec<Vec<u8>> {
    let mut st =
        s.db.prepare("SELECT key FROM group_keys ORDER BY rowid")
            .expect("prepare");
    st.query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

/// On Windows the secrets are stored encrypted (DPAPI); elsewhere as they
/// are, in an owner-only file (`spec/04-security.md`, Key storage).
#[test]
fn secrets_are_encrypted_on_windows() {
    let s = Store::open_in_memory().expect("open");
    let keys = DeviceKeys::from_secrets([1; 32], [2; 32]);
    s.save_identity(&keys).expect("save");
    s.apply(&PersistOp::AddGroupKey(StoredGroupKey {
        epoch: 0,
        key: GroupKey::from_bytes([6; 32]),
        at: 10,
    }))
    .expect("apply");
    let (id, gk) = (raw_identity(&s), raw_group_keys(&s));
    let mut plain = vec![1; 32];
    plain.extend([2; 32]);
    if cfg!(windows) {
        assert!(id.starts_with(super::protect::PREFIX));
        assert!(!id.windows(32).any(|w| w == [1; 32] || w == [2; 32]));
        assert!(gk[0].starts_with(super::protect::PREFIX));
        assert!(!gk[0].windows(32).any(|w| w == [6; 32]));
    } else {
        assert_eq!(id, plain);
        assert_eq!(gk, vec![vec![6; 32]]);
    }
    let back = s.load_identity().expect("load").expect("some");
    assert_eq!(back.device_id(), keys.device_id());
    assert_eq!(
        s.load().expect("load").group_keys[0].key,
        GroupKey::from_bytes([6; 32])
    );
}

/// Keys written unencrypted by v0.2.1 and earlier still load, and on
/// Windows are encrypted when they are first loaded.
#[test]
fn unencrypted_keys_from_older_versions_load_and_are_encrypted() {
    let s = Store::open_in_memory().expect("open");
    let mut plain = vec![1; 32];
    plain.extend([2; 32]);
    s.db.execute(
        "INSERT INTO meta (key, value) VALUES ('identity', ?1)",
        [&plain],
    )
    .expect("insert");
    s.db.execute(
        "INSERT INTO group_keys (epoch, key, at) VALUES (0, ?1, 10)",
        [vec![6u8; 32]],
    )
    .expect("insert");
    let want = DeviceKeys::from_secrets([1; 32], [2; 32]).device_id();
    for _ in 0..2 {
        assert_eq!(
            s.load_identity().expect("load").expect("some").device_id(),
            want
        );
        let st = s.load().expect("load");
        assert_eq!(st.group_keys.len(), 1);
        assert_eq!(st.group_keys[0].key, GroupKey::from_bytes([6; 32]));
        assert_eq!(
            raw_identity(&s).starts_with(super::protect::PREFIX),
            cfg!(windows)
        );
        assert_eq!(
            raw_group_keys(&s)[0].starts_with(super::protect::PREFIX),
            cfg!(windows)
        );
    }
    // A repeat of the migrated key is still recognised.
    s.apply(&PersistOp::AddGroupKey(StoredGroupKey {
        epoch: 0,
        key: GroupKey::from_bytes([6; 32]),
        at: 20,
    }))
    .expect("apply");
    assert_eq!(raw_group_keys(&s).len(), 1);
}

/// A database copied from Windows cannot be read elsewhere, and says why.
#[cfg(not(windows))]
#[test]
fn keys_encrypted_on_windows_are_refused_elsewhere() {
    let s = Store::open_in_memory().expect("open");
    let mut v = super::protect::PREFIX.to_vec();
    v.extend([0; 40]);
    s.db.execute(
        "INSERT INTO meta (key, value) VALUES ('identity', ?1)",
        [&v],
    )
    .expect("insert");
    match s.load_identity() {
        Err(super::StoreError::Unseal("identity", _)) => {}
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("read a key encrypted by Windows"),
    }
}

/// Linux, macOS and headless (`spec/04-security.md`, Key storage): the
/// database holding the keys is readable by its owner only.
#[cfg(unix)]
#[test]
fn the_database_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().expect("tempdir");
    let path = d.path().join("panic-button.db");
    let s = Store::open(&path).expect("open");
    s.save_identity(&DeviceKeys::from_secrets([1; 32], [2; 32]))
        .expect("save");
    let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
}
