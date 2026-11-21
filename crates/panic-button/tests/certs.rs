// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Certificate chain validation: each rule in `spec/04-security.md`
//! (Certificate validation), positive and negative, plus token enrolment and
//! the cascade / non-cascade revocation distinction.

use panic_button::crypto::cert::{ENROLLER_MAX_LIFETIME_MS, EXPIRY_TOLERANCE_MS};
use panic_button::crypto::token::{TokenRequest, create_token, enrol_with_token};
use panic_button::crypto::{
    CertError, DeviceKeys, GroupKey, RevocationRecord, SigningSecret, TrustStore, issue_cert,
};
use panic_button::proto::cert::CERT_VERSION;
use panic_button::proto::{Cert, CertBody, CertSerial, DeviceId, MsgId, PracticeId, Role, TokenId};

const DAY: u64 = 24 * 60 * 60 * 1000;
const T0: u64 = 1_763_000_000_000;
const PID: PracticeId = PracticeId([1; 16]);

struct Practice {
    root: SigningSecret,
    store: TrustStore,
}

fn practice() -> Practice {
    let root = SigningSecret::from_secret([0xaa; 32]);
    let store = TrustStore::new(PID, root.public());
    Practice { root, store }
}

fn device(n: u8) -> DeviceKeys {
    DeviceKeys::from_secrets([n; 32], [n.wrapping_add(100); 32])
}

fn body(subject: &DeviceKeys, role: Role, serial: u8, issued_at: u64, expires_at: u64) -> CertBody {
    CertBody {
        version: CERT_VERSION,
        practice_id: PID,
        serial: CertSerial([serial; 16]),
        subject: subject.device_id(),
        subject_kx: subject.kx_pub(),
        role,
        label: format!("dev{serial}"),
        issued_at,
        expires_at,
        issuer: [0; 32],
        token_id: None,
    }
}

fn add(s: &mut TrustStore, c: Cert) -> CertSerial {
    s.add_cert(c).expect("adds").0
}

fn revoke(p: &mut Practice, target: &DeviceKeys, serial: u8, cascade: bool, at: u64, id: u8) {
    p.store.add_revocation(RevocationRecord {
        msg_id: MsgId([id; 16]),
        target_serial: CertSerial([serial; 16]),
        target_device: target.device_id(),
        cascade,
        at,
    });
}

#[test]
fn rule1_signature_must_verify() {
    let mut p = practice();
    let a = device(1);
    let mut c = issue_cert(&p.root, body(&a, Role::Admin, 1, T0, 0));
    c.sig[0] ^= 1;
    assert!(p.store.add_cert(c).is_err());
}

#[test]
fn rule2_practice_id_must_match() {
    let mut p = practice();
    let a = device(1);
    let mut b = body(&a, Role::Admin, 1, T0, 0);
    b.practice_id = PracticeId([2; 16]);
    let c = issue_cert(&p.root, b);
    assert!(p.store.add_cert(c).is_err());
}

#[test]
fn rule3_root_issues_admin_only() {
    let mut p = practice();
    let a = device(1);
    let s = add(
        &mut p.store,
        issue_cert(&p.root, body(&a, Role::Admin, 1, T0, 0)),
    );
    assert!(p.store.validate(&s, T0).is_ok());
    assert!(p.store.is_admin(&a.device_id(), T0));

    let m = device(2);
    let s = add(
        &mut p.store,
        issue_cert(&p.root, body(&m, Role::Member, 2, T0, 0)),
    );
    assert_eq!(p.store.validate(&s, T0), Err(CertError::IssuerNotAllowed));
}

#[test]
fn rule3_role_constraints_on_issuers() {
    let mut p = practice();
    let admin = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    // Admin issues member and admin.
    let m = device(2);
    let sm = add(
        &mut p.store,
        issue_cert(&admin, body(&m, Role::Member, 2, T0, 0)),
    );
    assert!(p.store.validate(&sm, T0).is_ok());
    let a2 = device(3);
    let sa2 = add(
        &mut p.store,
        issue_cert(&admin, body(&a2, Role::Admin, 3, T0, 0)),
    );
    assert!(p.store.validate(&sa2, T0).is_ok());
    // A member cannot issue anything.
    let x = device(4);
    let sx = add(
        &mut p.store,
        issue_cert(&m, body(&x, Role::Member, 4, T0, 0)),
    );
    assert_eq!(p.store.validate(&sx, T0), Err(CertError::IssuerNotAllowed));
    // Unknown issuer.
    let stranger = device(5);
    let y = device(6);
    let sy = add(
        &mut p.store,
        issue_cert(&stranger, body(&y, Role::Member, 5, T0, 0)),
    );
    assert_eq!(p.store.validate(&sy, T0), Err(CertError::IssuerUnknown));
}

#[test]
fn rule3_enroller_issues_members_only_and_has_bounded_lifetime() {
    let mut p = practice();
    let admin = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    let enroller = SigningSecret::from_secret([0x55; 32]);
    let enr_dev = DeviceKeys::from_secrets(enroller.to_secret(), [0; 32]);
    // No expiry: invalid enroller.
    let s = add(
        &mut p.store,
        issue_cert(&admin, body(&enr_dev, Role::Enroller, 2, T0, 0)),
    );
    assert_eq!(p.store.validate(&s, T0), Err(CertError::EnrollerLifetime));
    // Over 365 days: invalid.
    let s = add(
        &mut p.store,
        issue_cert(
            &admin,
            body(
                &enr_dev,
                Role::Enroller,
                3,
                T0,
                T0 + ENROLLER_MAX_LIFETIME_MS + 1,
            ),
        ),
    );
    assert_eq!(p.store.validate(&s, T0), Err(CertError::EnrollerLifetime));
    // 30 days: fine.
    add(
        &mut p.store,
        issue_cert(&admin, body(&enr_dev, Role::Enroller, 4, T0, T0 + 30 * DAY)),
    );
    let m = device(9);
    let sm = add(
        &mut p.store,
        issue_cert(&enroller, body(&m, Role::Member, 5, T0 + DAY, 0)),
    );
    assert!(p.store.validate(&sm, T0 + DAY).is_ok());
    // An enroller cannot issue an admin.
    let a = device(10);
    let sa = add(
        &mut p.store,
        issue_cert(&enroller, body(&a, Role::Admin, 6, T0 + DAY, 0)),
    );
    assert_eq!(
        p.store.validate(&sa, T0 + DAY),
        Err(CertError::IssuerNotAllowed)
    );
}

#[test]
fn rule4_expiry_with_tolerance() {
    let mut p = practice();
    let admin = device(1);
    let s = add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, T0 + DAY)),
    );
    assert!(p.store.validate(&s, T0 + DAY + EXPIRY_TOLERANCE_MS).is_ok());
    assert_eq!(
        p.store.validate(&s, T0 + DAY + EXPIRY_TOLERANCE_MS + 1),
        Err(CertError::Expired)
    );
}

#[test]
fn members_enrolled_by_an_expired_token_stay_valid() {
    // ADR 0012: an issuer's expiry is judged at the child's issued_at.
    let mut p = practice();
    let admin = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    let enroller = SigningSecret::from_secret([0x55; 32]);
    let enr_dev = DeviceKeys::from_secrets(enroller.to_secret(), [0; 32]);
    add(
        &mut p.store,
        issue_cert(&admin, body(&enr_dev, Role::Enroller, 2, T0, T0 + 30 * DAY)),
    );
    let m = device(9);
    let sm = add(
        &mut p.store,
        issue_cert(&enroller, body(&m, Role::Member, 3, T0 + DAY, 0)),
    );
    let much_later = T0 + 400 * DAY;
    assert!(p.store.validate(&sm, much_later).is_ok());
    // But a member issued after expiry is invalid.
    let late = device(10);
    let sl = add(
        &mut p.store,
        issue_cert(&enroller, body(&late, Role::Member, 4, T0 + 40 * DAY, 0)),
    );
    assert_eq!(p.store.validate(&sl, much_later), Err(CertError::Expired));
}

#[test]
fn rule5_revoked_leaf_is_invalid() {
    let mut p = practice();
    let admin = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    let m = device(2);
    let sm = add(
        &mut p.store,
        issue_cert(&admin, body(&m, Role::Member, 2, T0, 0)),
    );
    revoke(&mut p, &m, 2, false, T0 + DAY, 1);
    assert_eq!(p.store.validate(&sm, T0 + DAY), Err(CertError::Revoked));
    assert!(!p.store.is_member(&m.device_id(), T0 + DAY));
}

#[test]
fn non_cascade_revocation_keeps_issued_certs() {
    // "This lets a practice manager leave without de-enrolling every PC
    // they set up."
    let mut p = practice();
    let pm = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&pm, Role::Admin, 1, T0, 0)),
    );
    let pc = device(2);
    let spc = add(
        &mut p.store,
        issue_cert(&pm, body(&pc, Role::Member, 2, T0 + DAY, 0)),
    );
    revoke(&mut p, &pm, 1, false, T0 + 10 * DAY, 1);
    assert!(!p.store.is_admin(&pm.device_id(), T0 + 11 * DAY));
    assert!(p.store.validate(&spc, T0 + 11 * DAY).is_ok());
    // Certs issued by the revoked admin after the revocation are invalid.
    let late = device(3);
    let sl = add(
        &mut p.store,
        issue_cert(&pm, body(&late, Role::Member, 3, T0 + 12 * DAY, 0)),
    );
    assert_eq!(
        p.store.validate(&sl, T0 + 12 * DAY),
        Err(CertError::Revoked)
    );
}

#[test]
fn cascade_revocation_removes_descendants() {
    let mut p = practice();
    let pm = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&pm, Role::Admin, 1, T0, 0)),
    );
    let a2 = device(2);
    add(
        &mut p.store,
        issue_cert(&pm, body(&a2, Role::Admin, 2, T0, 0)),
    );
    let pc = device(3);
    let spc = add(
        &mut p.store,
        issue_cert(&a2, body(&pc, Role::Member, 3, T0, 0)),
    );
    revoke(&mut p, &pm, 1, true, T0 + DAY, 1);
    assert_eq!(p.store.validate(&spc, T0 + DAY), Err(CertError::Revoked));
    assert!(!p.store.is_admin(&a2.device_id(), T0 + DAY));
}

#[test]
fn validity_is_order_independent() {
    // Add the leaf before its issuer: it becomes valid once the issuer
    // arrives.
    let mut p = practice();
    let admin = device(1);
    let m = device(2);
    let leaf = issue_cert(&admin, body(&m, Role::Member, 2, T0, 0));
    let sm = add(&mut p.store, leaf);
    assert_eq!(p.store.validate(&sm, T0), Err(CertError::IssuerUnknown));
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    assert!(p.store.validate(&sm, T0).is_ok());
}

#[test]
fn chain_depth_is_bounded() {
    let mut p = practice();
    let mut prev = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&prev, Role::Admin, 1, T0, 0)),
    );
    for i in 2..=12u8 {
        let d = device(i);
        add(
            &mut p.store,
            issue_cert(&prev, body(&d, Role::Admin, i, T0, 0)),
        );
        prev = d;
    }
    let last = CertSerial([12; 16]);
    assert_eq!(p.store.validate(&last, T0), Err(CertError::ChainTooDeep));
    // Depth 8 is fine.
    assert!(p.store.validate(&CertSerial([9; 16]), T0).is_ok());
}

#[test]
fn serial_conflict_is_rejected() {
    let mut p = practice();
    let a = device(1);
    let b = device(2);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&a, Role::Admin, 1, T0, 0)),
    );
    assert!(
        p.store
            .add_cert(issue_cert(&p.root, body(&b, Role::Admin, 1, T0, 0)))
            .is_err()
    );
}

#[test]
fn chain_for_relay_auth_excludes_root() {
    let mut p = practice();
    let admin = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    let m = device(2);
    add(
        &mut p.store,
        issue_cert(&admin, body(&m, Role::Member, 2, T0, 0)),
    );
    let chain = p.store.chain_for(&m.device_id(), T0).expect("chain");
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].body().expect("b").subject, m.device_id());
    assert_eq!(chain[1].body().expect("b").subject, admin.device_id());
}

#[test]
fn token_enrolment_end_to_end() {
    let mut p = practice();
    let admin = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    let gk = GroupKey::from_bytes([7; 32]);
    let (token, enroller_cert) = create_token(
        &admin,
        [0x55; 32],
        &gk,
        TokenRequest {
            practice_id: PID,
            practice_name: "Riverside Surgery".into(),
            root_pub: p.root.public(),
            label: "IT rollout".into(),
            token_id: TokenId([3; 16]),
            enroller_serial: CertSerial([2; 16]),
            issued_at: T0,
            expires_at: T0 + 30 * DAY,
            epoch: 0,
            relay_urls: vec![],
            seed_peers: vec![],
            network: Default::default(),
            issuer_chain: vec![],
        },
    )
    .expect("creates");
    let parsed = panic_button::proto::Token::parse(&token.to_token_string()).expect("parses");
    let dev = device(9);
    let cert =
        enrol_with_token(&parsed, &dev, "ROOM4-PC", CertSerial([9; 16]), T0 + DAY).expect("enrols");
    add(&mut p.store, enroller_cert);
    let s = add(&mut p.store, cert);
    let b = p.store.validate(&s, T0 + DAY).expect("valid");
    assert_eq!(b.token_id, Some(TokenId([3; 16])));
    assert!(p.store.is_member(&dev.device_id(), T0 + DAY));
    // Expired token refuses to enrol.
    assert!(
        enrol_with_token(
            &parsed,
            &device(10),
            "X",
            CertSerial([10; 16]),
            T0 + 31 * DAY
        )
        .is_err()
    );
}

#[test]
fn revoked_device_cannot_return_with_new_cert() {
    let mut p = practice();
    let admin = device(1);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&admin, Role::Admin, 1, T0, 0)),
    );
    let m = device(2);
    add(
        &mut p.store,
        issue_cert(&admin, body(&m, Role::Member, 2, T0, 0)),
    );
    revoke(&mut p, &m, 2, false, T0 + DAY, 1);
    let s = add(
        &mut p.store,
        issue_cert(&admin, body(&m, Role::Member, 3, T0 + 2 * DAY, 0)),
    );
    assert_eq!(p.store.validate(&s, T0 + 2 * DAY), Err(CertError::Revoked));
    let _ = DeviceId::default();
}

/// `03-protocol.md`, Clarifications: unknown certificate roles carry no
/// authority. A cert with an unknown role, even from the root, cannot issue
/// anything, and its holder is not an admin.
#[test]
fn unknown_roles_carry_no_authority() {
    let mut p = practice();
    let odd = device(1);
    let s = add(
        &mut p.store,
        issue_cert(&p.root, body(&odd, Role::Other(9), 1, T0, 0)),
    );
    assert!(
        p.store.validate(&s, T0).is_err(),
        "the root only issues admin certs"
    );
    assert!(!p.store.is_admin(&odd.device_id(), T0));

    let a = device(2);
    add(
        &mut p.store,
        issue_cert(&p.root, body(&a, Role::Admin, 2, T0, 0)),
    );
    let odd2 = device(3);
    add(
        &mut p.store,
        issue_cert(&a, body(&odd2, Role::Other(9), 3, T0, 0)),
    );
    assert!(!p.store.is_admin(&odd2.device_id(), T0));
    let m = device(4);
    let child = add(
        &mut p.store,
        issue_cert(&odd2, body(&m, Role::Member, 4, T0, 0)),
    );
    assert!(
        p.store.validate(&child, T0).is_err(),
        "an unknown role cannot issue"
    );
}
