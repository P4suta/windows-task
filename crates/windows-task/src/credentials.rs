#![cfg_attr(
    windows,
    allow(
        unsafe_code,
        reason = "Credential Manager returns an owned native credential buffer"
    )
)]

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

/// A password whose storage is zeroed and never exposed through `Debug`.
pub struct Password {
    utf16: Zeroizing<Vec<u16>>,
}

impl Password {
    /// Copies a password into zeroizing UTF-16 storage.
    #[must_use]
    pub fn new(value: impl AsRef<str>) -> Self {
        Self {
            utf16: Zeroizing::new(value.as_ref().encode_utf16().collect()),
        }
    }

    /// Borrows the UTF-16 code units without a terminating NUL.
    #[must_use]
    pub fn as_utf16(&self) -> &[u16] {
        &self.utf16
    }

    /// Reads the secret blob of a generic Windows Credential Manager entry.
    /// `cmdkey` and the Windows credential UI store these password blobs as
    /// UTF-16LE. The native buffer and the returned password are zeroed.
    pub fn from_credential_manager(target: &str) -> crate::Result<Self> {
        read_credential_manager(target).map(|credential| credential.password)
    }

    #[cfg(windows)]
    pub(crate) fn into_utf16(mut self) -> Zeroizing<Vec<u16>> {
        std::mem::take(&mut self.utf16)
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Password([REDACTED])")
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.utf16.zeroize();
    }
}

/// One username/password pair used only for a scheduler operation.
#[derive(Debug)]
pub struct Credential {
    username: String,
    password: Password,
}

impl Credential {
    /// Creates a credential. Use `DOMAIN\\user` or a UPN for remote targets.
    #[must_use]
    pub fn new(username: impl Into<String>, password: Password) -> Self {
        Self {
            username: username.into(),
            password,
        }
    }

    /// Returns the non-secret username.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Returns the secret password.
    #[must_use]
    pub fn password(&self) -> &Password {
        &self.password
    }

    /// Reads a username/password pair from a generic Windows Credential
    /// Manager entry. No plaintext is accepted from manifests or command-line
    /// arguments.
    pub fn from_credential_manager(target: &str) -> crate::Result<Self> {
        read_credential_manager(target)
    }

    #[cfg(windows)]
    pub(crate) fn into_parts(self) -> (String, Zeroizing<Vec<u16>>) {
        (self.username, self.password.into_utf16())
    }
}

/// Credentials supplied independently for connection and task registration.
#[derive(Debug, Default)]
pub struct Credentials {
    /// Optional credential used to establish a remote scheduler session.
    pub connection: Option<Credential>,
    /// Optional password used when registering a password-backed principal.
    pub registration: Option<Password>,
}

#[cfg(not(windows))]
fn read_credential_manager(target: &str) -> crate::Result<Credential> {
    Err(crate::Error::new(
        crate::ErrorKind::UnsupportedPlatform,
        "Windows Credential Manager is only available on Windows",
    )
    .with_target(target))
}

#[cfg(windows)]
fn read_credential_manager(target: &str) -> crate::Result<Credential> {
    use windows::{
        Win32::Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW},
        core::BSTR,
    };

    struct NativeCredential(*mut CREDENTIALW);

    impl Drop for NativeCredential {
        fn drop(&mut self) {
            unsafe {
                if let Some(credential) = self.0.as_ref() {
                    let length = usize::try_from(credential.CredentialBlobSize)
                        .expect("credential blob size fits usize");
                    if !credential.CredentialBlob.is_null() {
                        for index in 0..length {
                            credential.CredentialBlob.add(index).write_volatile(0);
                        }
                    }
                }
                CredFree(self.0.cast());
            }
        }
    }

    let mut raw = std::ptr::null_mut();
    unsafe { CredReadW(&BSTR::from(target), CRED_TYPE_GENERIC, None, &raw mut raw) }.map_err(
        |error| {
            let code = error.code().0;
            let kind = if u32::from_ne_bytes(code.to_ne_bytes()) == 0x8007_0490 {
                crate::ErrorKind::NotFound
            } else {
                crate::ErrorKind::Authentication
            };
            crate::Error::new(kind, error.message())
                .with_operation("read Windows Credential Manager")
                .with_target(target)
                .with_native_code(code)
        },
    )?;
    let native = NativeCredential(raw);
    let credential = unsafe { raw.as_ref() }.ok_or_else(|| {
        crate::Error::new(
            crate::ErrorKind::Authentication,
            "Credential Manager returned a null credential",
        )
        .with_target(target)
    })?;
    let username = if credential.UserName.is_null() {
        String::new()
    } else {
        unsafe { credential.UserName.to_string() }.map_err(|error| {
            crate::Error::new(
                crate::ErrorKind::Authentication,
                format!("Credential Manager username is invalid UTF-16: {error}"),
            )
            .with_target(target)
        })?
    };
    let byte_length =
        usize::try_from(credential.CredentialBlobSize).expect("credential blob size fits usize");
    if byte_length % 2 != 0 {
        return Err(crate::Error::new(
            crate::ErrorKind::Authentication,
            "Credential Manager secret is not UTF-16LE",
        )
        .with_target(target));
    }
    if byte_length != 0 && credential.CredentialBlob.is_null() {
        return Err(crate::Error::new(
            crate::ErrorKind::Authentication,
            "Credential Manager returned a null secret blob",
        )
        .with_target(target));
    }
    let bytes = if byte_length == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(credential.CredentialBlob, byte_length) }
    };
    let mut units = Zeroizing::new(
        bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<u16>>(),
    );
    while units.last() == Some(&0) {
        units.pop();
    }
    let password = Zeroizing::new(String::from_utf16(&units).map_err(|error| {
        crate::Error::new(
            crate::ErrorKind::Authentication,
            format!("Credential Manager secret is invalid UTF-16: {error}"),
        )
        .with_target(target)
    })?);
    let result = Credential::new(username, Password::new(password.as_str()));
    drop(native);
    Ok(result)
}
