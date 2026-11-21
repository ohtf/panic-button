// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Enrolment flows that happen before a core runs: setting up a new
//! practice (flow A) and token enrolment (flow B) in `spec/04-security.md`.
//! Both are pure: randomness comes from a caller-seeded [`CoreRng`].

use crate::crypto::cert::issue_cert;
use crate::crypto::token::enrol_with_token;
use crate::crypto::{CryptoError, DeviceKeys, GroupKey, SigningSecret};
use crate::proto::cert::CERT_VERSION;
use crate::proto::{Cbor, CertBody, CertSerial, PracticeId, Role, Token};

use crate::core::persist::{PersistedState, PracticeInfo, StoredGroupKey};
use crate::core::rng::CoreRng;
use crate::core::time::WallMs;

/// Fresh device keys.
pub fn new_device_keys(rng: &mut CoreRng) -> DeviceKeys {
    DeviceKeys::from_secrets(rng.bytes(), rng.bytes())
}

/// The result of setting up a new practice. The caller must seal `root`
/// into a recovery kit, check the passphrase opens it, then drop `root`
/// (it is zeroized on drop and never stored on any device).
#[derive(Debug)]
pub struct NewPractice {
    pub state: PersistedState,
    pub root: SigningSecret,
}

pub fn new_practice(
    rng: &mut CoreRng,
    device: &DeviceKeys,
    name: &str,
    label: &str,
    now: WallMs,
) -> NewPractice {
    let practice_id = PracticeId(rng.bytes());
    let root = SigningSecret::from_secret(rng.bytes());
    let group_key = GroupKey::from_bytes(rng.bytes());
    let serial = CertSerial(rng.bytes());
    let cert = issue_cert(
        &root,
        CertBody {
            version: CERT_VERSION,
            practice_id,
            serial,
            subject: device.device_id(),
            subject_kx: device.kx_pub(),
            role: Role::Admin,
            label: label.to_owned(),
            issued_at: now,
            expires_at: 0,
            issuer: [0; 32],
            token_id: None,
        },
    );
    let state = PersistedState {
        practice: Some(PracticeInfo {
            practice_id,
            root_pub: root.public(),
            name: name.to_owned(),
        }),
        own_serial: Some(serial),
        certs: vec![cert.to_cbor()],
        group_keys: vec![StoredGroupKey {
            epoch: 0,
            key: group_key,
            at: now,
        }],
        // The name given at setup is this computer's name to colleagues
        // too, as if an admin had assigned it (ADR 0014).
        own_settings: (!label.trim().is_empty()).then(|| {
            crate::proto::DeviceSettings {
                target: device.device_id(),
                label: Some(label.trim().to_owned()),
                room: None,
                assigned_at: now,
            }
            .to_cbor()
        }),
        ..Default::default()
    };
    NewPractice { state, root }
}

/// Token enrolment: self-issue a member cert with the token's enroller key.
pub fn enrol_with_token_state(
    rng: &mut CoreRng,
    device: &DeviceKeys,
    token: &Token,
    label: &str,
    now: WallMs,
) -> Result<PersistedState, CryptoError> {
    let serial = CertSerial(rng.bytes());
    let cert = enrol_with_token(token, device, label, serial, now)?;
    let mut certs = token.issuer_chain.clone();
    certs.push(token.enroller_cert.clone());
    certs.push(cert.to_cbor());
    Ok(PersistedState {
        practice: Some(PracticeInfo {
            practice_id: token.practice_id,
            root_pub: token.root_pub,
            name: token.practice_name.clone(),
        }),
        own_serial: Some(serial),
        certs,
        group_keys: vec![StoredGroupKey {
            epoch: token.epoch,
            key: GroupKey::from_bytes(token.group_key),
            at: now,
        }],
        ..Default::default()
    })
}

/// Recovery (flow D): the root signs a new admin cert for `device`. Returns
/// the raw cert to add to the device's state and announce.
pub fn recover_admin(
    rng: &mut CoreRng,
    root: &SigningSecret,
    practice_id: PracticeId,
    device: &DeviceKeys,
    label: &str,
    now: WallMs,
) -> (CertSerial, Vec<u8>) {
    let serial = CertSerial(rng.bytes());
    let cert = issue_cert(
        root,
        CertBody {
            version: CERT_VERSION,
            practice_id,
            serial,
            subject: device.device_id(),
            subject_kx: device.kx_pub(),
            role: Role::Admin,
            label: label.to_owned(),
            issued_at: now,
            expires_at: 0,
            issuer: [0; 32],
            token_id: None,
        },
    );
    (serial, cert.to_cbor())
}
