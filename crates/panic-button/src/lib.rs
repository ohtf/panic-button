// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Panic Button: a staff panic alarm for GP practices and other small
//! workplaces, over the local network (`spec/02-architecture.md`).
//!
//! One crate, three programs: `panic-button` (admin and diagnostics),
//! `panic-button-agent` (headless agent) and `panic-button-relay` (the
//! optional relay, behind the `relay` feature). The desktop app embeds the
//! agent. Modules, from the bottom up:
//!
//! - [`proto`]: wire types and deterministic CBOR (no I/O);
//! - [`crypto`]: keys, certificates, sealing and recovery kits (no I/O);
//! - [`core`]: the sans-IO state machine (no I/O, clock or randomness);
//! - [`store`] and [`net`]: SQLite persistence and the network drivers;
//! - [`agent`]: the runtime wiring them together, and the control API;
//! - [`relay`]: the relay server, which needs only `proto` and `crypto`;
//! - [`cli`]: the `panic-button` command.
//!
//! `tests/boundaries.rs` keeps the lower modules free of I/O and of the
//! modules above them, which separate crates used to guarantee.

// Unsafe code only for Windows system calls with no safe wrapper (DPAPI in
// `store::protect`, ADR 0017); the modules below `store` forbid it.
#![deny(unsafe_code)]

pub mod proto;

pub mod crypto;

pub mod core;

pub mod store;

pub mod net;

pub mod agent;

#[cfg(feature = "relay")]
pub mod relay;

pub mod cli;
