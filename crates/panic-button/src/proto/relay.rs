// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Relay control structures and WebSocket framing (`spec/06-relay.md`).
//!
//! Each WebSocket message starts with one frame-type byte:
//! 0x00 envelope, 0x01 presence, 0x02 challenge (server to client),
//! 0x03 auth result (server to client). The client's RelayAuth travels as an
//! envelope frame.

use crate::proto::cbor::{Cbor, Reader, Writer, req};
use crate::proto::error::DecodeError;
use crate::proto::ids::DeviceId;
use crate::proto::limits;

pub const FRAME_ENVELOPE: u8 = 0x00;
pub const FRAME_PRESENCE: u8 = 0x01;
pub const FRAME_CHALLENGE: u8 = 0x02;
pub const FRAME_AUTH_RESULT: u8 = 0x03;

/// Close codes.
pub const CLOSE_AUTH_FAILED: u16 = 4401;
pub const CLOSE_REVOKED: u16 = 4403;
pub const CLOSE_UNKNOWN_TENANT: u16 = 4404;
pub const CLOSE_RATE_LIMITED: u16 = 4429;

/// Maximum devices listed in a presence frame or auth result.
pub const PRESENCE_MAX: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub nonce: [u8; 32],
    pub server_time: u64,
    pub relay_version: String,
}

impl Cbor for Challenge {
    fn encode(&self, w: &mut Writer) {
        w.map(3)
            .u64(0)
            .bytes(&self.nonce)
            .u64(1)
            .u64(self.server_time)
            .u64(2)
            .text(&self.relay_version);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut n, mut t, mut v) = (None, None, None);
        r.map(|k, r| {
            match k {
                0 => n = Some(r.fixed::<32>()?),
                1 => t = Some(r.u64()?),
                2 => v = Some(r.text()?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(Challenge {
            nonce: req(n, 0)?,
            server_time: req(t, 1)?,
            relay_version: req(v, 2)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthResult {
    pub ok: bool,
    pub session_id: [u8; 16],
    pub cursor: u64,
    pub presence: Vec<DeviceId>,
}

fn encode_ids(w: &mut Writer, ids: &[DeviceId]) {
    w.array(ids.len());
    for id in ids {
        id.encode(w);
    }
}

impl Cbor for AuthResult {
    fn encode(&self, w: &mut Writer) {
        w.map(4)
            .u64(0)
            .bool(self.ok)
            .u64(1)
            .bytes(&self.session_id)
            .u64(2)
            .u64(self.cursor)
            .u64(3);
        encode_ids(w, &self.presence);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let (mut ok, mut s, mut c, mut p) = (None, None, None, None);
        r.map(|k, r| {
            match k {
                0 => ok = Some(r.bool()?),
                1 => s = Some(r.fixed::<16>()?),
                2 => c = Some(r.u64()?),
                3 => p = Some(r.array_of(PRESENCE_MAX, DeviceId::decode)?),
                _ => return Ok(false),
            }
            Ok(true)
        })?;
        Ok(AuthResult {
            ok: req(ok, 0)?,
            session_id: req(s, 1)?,
            cursor: req(c, 2)?,
            presence: p.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Presence {
    pub connected: Vec<DeviceId>,
}

impl Cbor for Presence {
    fn encode(&self, w: &mut Writer) {
        w.map(1).u64(0);
        encode_ids(w, &self.connected);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let mut c = None;
        r.map(|k, r| {
            if k == 0 {
                c = Some(r.array_of(PRESENCE_MAX, DeviceId::decode)?);
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        Ok(Presence {
            connected: req(c, 0)?,
        })
    }
}

/// A WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Raw Envelope bytes.
    Envelope(Vec<u8>),
    Presence(Presence),
    Challenge(Challenge),
    AuthResult(AuthResult),
}

impl Frame {
    pub fn to_bytes(&self) -> Vec<u8> {
        let (t, body) = match self {
            Frame::Envelope(b) => (FRAME_ENVELOPE, b.clone()),
            Frame::Presence(p) => (FRAME_PRESENCE, p.to_cbor()),
            Frame::Challenge(c) => (FRAME_CHALLENGE, c.to_cbor()),
            Frame::AuthResult(a) => (FRAME_AUTH_RESULT, a.to_cbor()),
        };
        let mut v = Vec::with_capacity(body.len() + 1);
        v.push(t);
        v.extend_from_slice(&body);
        v
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, DecodeError> {
        if b.len() > limits::RELAY_WS_FRAME_MAX {
            return Err(DecodeError::TooLarge);
        }
        let (&t, rest) = b.split_first().ok_or(DecodeError::Malformed)?;
        Ok(match t {
            FRAME_ENVELOPE => Frame::Envelope(rest.to_vec()),
            FRAME_PRESENCE => Frame::Presence(Presence::from_cbor(rest)?),
            FRAME_CHALLENGE => Frame::Challenge(Challenge::from_cbor(rest)?),
            FRAME_AUTH_RESULT => Frame::AuthResult(AuthResult::from_cbor(rest)?),
            _ => return Err(DecodeError::UnknownType(u64::from(t))),
        })
    }
}

/// Encodes a long-poll batch: a CBOR array of envelope byte strings.
pub fn encode_batch(envelopes: &[Vec<u8>]) -> Vec<u8> {
    let mut w = Writer::new();
    w.array(envelopes.len());
    for e in envelopes {
        w.bytes(e);
    }
    w.into_bytes()
}

pub fn decode_batch(b: &[u8]) -> Result<Vec<Vec<u8>>, DecodeError> {
    if b.len() > limits::RELAY_LP_BATCH_MAX {
        return Err(DecodeError::TooLarge);
    }
    let mut r = Reader::new(b);
    let v = r.array_of(limits::LIST_MAX, |r| {
        Ok(r.bytes(limits::RELAY_WS_FRAME_MAX)?.to_vec())
    })?;
    r.finish()?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `03-protocol.md`, Size limits: 64 KiB per WebSocket frame, 256 KiB
    /// per long-poll batch.
    #[test]
    fn oversized_frames_and_batches_are_refused() {
        let mut f = vec![FRAME_ENVELOPE];
        f.resize(limits::RELAY_WS_FRAME_MAX, 0);
        assert!(Frame::from_bytes(&f).is_ok());
        f.push(0);
        assert!(matches!(Frame::from_bytes(&f), Err(DecodeError::TooLarge)));
        let big = encode_batch(&vec![vec![0; limits::RELAY_WS_FRAME_MAX]; 4]);
        assert!(big.len() > limits::RELAY_LP_BATCH_MAX);
        assert!(matches!(decode_batch(&big), Err(DecodeError::TooLarge)));
    }

    #[test]
    fn frames_round_trip() {
        let frames = [
            Frame::Envelope(vec![1, 2, 3]),
            Frame::Presence(Presence {
                connected: vec![DeviceId([1; 32])],
            }),
            Frame::Challenge(Challenge {
                nonce: [4; 32],
                server_time: 5,
                relay_version: "0.1.0".into(),
            }),
            Frame::AuthResult(AuthResult {
                ok: true,
                session_id: [6; 16],
                cursor: 7,
                presence: vec![],
            }),
        ];
        for f in frames {
            assert_eq!(Frame::from_bytes(&f.to_bytes()), Ok(f));
        }
        assert!(Frame::from_bytes(&[]).is_err());
        assert!(Frame::from_bytes(&[9]).is_err());
    }

    #[test]
    fn batch_round_trip() {
        let b = vec![vec![1], vec![2, 3]];
        assert_eq!(decode_batch(&encode_batch(&b)), Ok(b));
    }
}
