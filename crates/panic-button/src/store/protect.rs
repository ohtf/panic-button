// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! Secret values at rest: the device's key pair and the group keys.
//!
//! On Windows they are encrypted with DPAPI in machine scope
//! (`CRYPTPROTECT_LOCAL_MACHINE`, ADR 0010), so the database stops working
//! if it is copied to another computer, and stored as [`PREFIX`] followed by
//! the DPAPI blob. A value without the prefix was written before encryption
//! (v0.2.1 and earlier); it is read as it is and the caller seals it again
//! (`Unsealed::rewrite`). Elsewhere values are stored as they are, in a
//! file only its owner can read; a sealed value there cannot be read.
//!
//! A plain key starting with the prefix by chance is a 1 in 2^88 event.

use zeroize::Zeroizing;

use super::{Result, StoreError};

/// Marks a value encrypted with DPAPI (version 1 of this format).
pub(super) const PREFIX: &[u8] = b"pb-dpapi-1:";

pub(super) struct Unsealed {
    pub plain: Zeroizing<Vec<u8>>,
    /// Stored unencrypted where it should be encrypted: write it again.
    pub rewrite: bool,
}

/// The stored form of `plain`.
pub(super) fn seal(plain: &[u8], what: &'static str) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let blob = dpapi::protect(plain).map_err(|e| StoreError::Seal(what, e.to_string()))?;
        let mut v = Vec::with_capacity(PREFIX.len() + blob.len());
        v.extend_from_slice(PREFIX);
        v.extend_from_slice(&blob);
        Ok(v)
    }
    #[cfg(not(windows))]
    {
        let _ = what;
        Ok(plain.to_vec())
    }
}

/// The plain value of a stored one.
pub(super) fn unseal(stored: &[u8], what: &'static str) -> Result<Unsealed> {
    match stored.strip_prefix(PREFIX) {
        #[cfg(windows)]
        Some(blob) => Ok(Unsealed {
            plain: dpapi::unprotect(blob).map_err(|e| StoreError::Unseal(what, e.to_string()))?,
            rewrite: false,
        }),
        #[cfg(not(windows))]
        Some(_) => Err(StoreError::Unseal(
            what,
            "it was encrypted by Windows, and this is not Windows".into(),
        )),
        None => Ok(Unsealed {
            plain: Zeroizing::new(stored.to_vec()),
            rewrite: cfg!(windows),
        }),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
        CryptProtectData, CryptUnprotectData,
    };
    use windows::core::PCWSTR;
    use zeroize::Zeroizing;

    /// Extra input to DPAPI, so another program on the computer using
    /// machine scope does not decrypt these values by accident.
    const ENTROPY: &[u8] = b"org.ohtf.panic-button/store/v1";

    /// A blob describing `b`, for input only: DPAPI does not write to it.
    fn input(b: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: b.len() as u32,
            pbData: b.as_ptr().cast_mut(),
        }
    }

    /// Copies DPAPI's output and frees it, wiping it first.
    ///
    /// # Safety
    ///
    /// `out` must be the output of a successful DPAPI call, not yet freed.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Zeroizing<Vec<u8>> {
        if out.pbData.is_null() {
            return Zeroizing::new(Vec::new());
        }
        // SAFETY: DPAPI allocated `cbData` bytes at `pbData` with LocalAlloc
        // and handed them to us; nothing else refers to them.
        unsafe {
            let v = Zeroizing::new(
                std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec(),
            );
            std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
            let _ = LocalFree(Some(HLOCAL(out.pbData.cast())));
            v
        }
    }

    pub fn protect(plain: &[u8]) -> windows::core::Result<Vec<u8>> {
        let (data, entropy) = (input(plain), input(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: the input blobs point at live slices for the length given;
        // `out` receives a LocalAlloc buffer, which `take` frees.
        unsafe {
            CryptProtectData(
                &data,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )?;
            Ok(take(out).to_vec())
        }
    }

    pub fn unprotect(blob: &[u8]) -> windows::core::Result<Zeroizing<Vec<u8>>> {
        let (data, entropy) = (input(blob), input(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`.
        unsafe {
            CryptUnprotectData(
                &data,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )?;
            Ok(take(out))
        }
    }
}
