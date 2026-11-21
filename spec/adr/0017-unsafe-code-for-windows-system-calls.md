# ADR 0017: Unsafe code for Windows system calls in the published crate

- Status: Accepted (2026-11-21)

## Context

ADR 0010 says the device keys and group keys on Windows are encrypted with DPAPI in machine scope. The store, in the `panic-button` crate, writes them. DPAPI (`CryptProtectData`, `CryptUnprotectData`) is reached through the `windows` crate, whose functions are `unsafe`: they take raw pointers, and the output buffer must be freed with `LocalFree`. A wrapper crate would be one more dependency to review (`dependencies.md`) for two calls, and the `windows` crate is already reviewed and used by the desktop app.

The crate had `#![forbid(unsafe_code)]` at its root. `forbid` cannot be relaxed further down, so any unsafe code at all needs the root changed.

The desktop app already calls Windows APIs with `unsafe` (audio endpoints, sessions, the watchdog task), but it is a separate, unpublished crate. Putting DPAPI there instead would leave the headless agent and the `panic-button` command on Windows writing plain keys, and the store would have to take sealing as a parameter from every caller.

## Decision

- The crate root changes to `#![deny(unsafe_code)]`. The only `#[allow(unsafe_code)]` is the Windows-only `dpapi` module in `store/protect.rs`, which is two functions and a helper that frees DPAPI's output. Each `unsafe` block has a `SAFETY` comment.
- `proto`, `crypto`, `core` and `relay` keep `#![forbid(unsafe_code)]` at module level, so those can never allow it.
- Encrypted values are stored as `pb-dpapi-1:` followed by the DPAPI blob, with fixed extra entropy `org.ohtf.panic-button/store/v1` and `CRYPTPROTECT_UI_FORBIDDEN`. Values without the prefix were written by v0.2.1 or earlier; on Windows they are read and encrypted again when first loaded. On Linux and macOS values are stored as they are (`spec/04-security.md`, Key storage), and a value with the prefix is refused with an error saying it was encrypted by Windows.

## Consequences

- Any other unsafe code in the crate fails to compile until it is given its own `allow` and reviewed against this ADR.
- On Windows, once a computer has run a version with DPAPI, an older version cannot read its keys and fails at start-up with a corrupt record. Downgrading needs the computer enrolled again.
- A database copied from one Windows computer to another, or to Linux or macOS, cannot be read there. That is the point of machine scope.
- DPAPI output differs each time, so the `group_keys` primary key no longer drops a repeated key. The store compares the decrypted keys before inserting.
- Linux and macOS behaviour is unchanged. Keychain and Secret Service storage (`spec/04-security.md`) remain to do.
