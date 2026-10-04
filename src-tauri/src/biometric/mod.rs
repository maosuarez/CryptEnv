//! Windows Hello unlock. The vault key is wrapped under a key derived from a
//! Hello `KeyCredential` signature (see [`wrap`]); the master password is never
//! stored. The platform part is behind [`HelloSigner`] so the wrapping,
//! enrollment and unlock logic is testable without Windows.

pub mod wrap;

use std::sync::Arc;

use zeroize::Zeroizing;

#[derive(Debug, PartialEq)]
pub enum BiometricStatus {
    Available,
    NotConfiguredForUser,
    DisabledByPolicy,
    NotAvailable,
}

impl BiometricStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BiometricStatus::Available => "available",
            BiometricStatus::NotConfiguredForUser => "not_configured",
            BiometricStatus::DisabledByPolicy => "disabled_by_policy",
            BiometricStatus::NotAvailable => "not_available",
        }
    }
}

/// Settings key holding the enrollment blob (`wrap::Enrollment` JSON); empty
/// when not enrolled.
pub const BLOB_SETTING: &str = "biometric_blob";
/// Settings key set to "1" when an enrollment was dropped (upgrade from the
/// legacy format, or a password change) and the user should re-enroll.
pub const NOTICE_SETTING: &str = "biometric_reenroll_notice";

#[derive(Debug, PartialEq)]
pub enum HelloError {
    /// No Hello KeyCredential support on this platform/device.
    Unsupported,
    /// The user dismissed the Hello prompt.
    Cancelled,
    /// Two signatures over the same challenge differed.
    NotDeterministic,
    /// The stored enrollment cannot be opened (wrong signature, corrupt data,
    /// credential reset).
    Unusable,
    /// Platform failure. Never contains key material.
    Platform(String),
}

impl HelloError {
    pub fn message(&self) -> String {
        match self {
            HelloError::Unsupported => {
                "biometric unlock is not available on this platform".to_string()
            }
            HelloError::Cancelled => "Windows Hello verification was not completed".to_string(),
            HelloError::NotDeterministic => {
                "biometric unlock is not supported on this device".to_string()
            }
            HelloError::Unusable => {
                "biometric enrollment is no longer valid, disable it and enroll again".to_string()
            }
            HelloError::Platform(e) => format!("Windows Hello failed: {e}"),
        }
    }
}

/// Windows Hello `KeyCredential` operations. All calls block (WinRT async is
/// waited on), so callers run them in `spawn_blocking`.
pub trait HelloSigner: Send + Sync {
    /// Creates (replacing any existing) the vault credential. Prompts the user.
    fn create(&self) -> Result<(), HelloError>;
    /// Signs `challenge` with the vault credential. Prompts the user. The
    /// signature is deterministic for a given credential and challenge.
    fn sign(&self, challenge: &[u8]) -> Result<Zeroizing<Vec<u8>>, HelloError>;
    /// Deletes the vault credential. Succeeds if there is none.
    fn delete(&self) -> Result<(), HelloError>;
}

/// The signer for this platform.
pub fn platform_signer() -> Arc<dyn HelloSigner> {
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows_impl::WindowsHello)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Arc::new(UnsupportedHello)
    }
}

#[cfg(not(target_os = "windows"))]
struct UnsupportedHello;

#[cfg(not(target_os = "windows"))]
impl HelloSigner for UnsupportedHello {
    fn create(&self) -> Result<(), HelloError> {
        Err(HelloError::Unsupported)
    }
    fn sign(&self, _challenge: &[u8]) -> Result<Zeroizing<Vec<u8>>, HelloError> {
        Err(HelloError::Unsupported)
    }
    fn delete(&self) -> Result<(), HelloError> {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{BiometricStatus, HelloError, HelloSigner};
    use windows::Security::Credentials::UI::{UserConsentVerifier, UserConsentVerifierAvailability};
    use windows::Security::Credentials::{
        KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
    };
    use windows::Security::Cryptography::CryptographicBuffer;
    use windows::Storage::Streams::IBuffer;
    use windows::core::{Array, HSTRING};
    use zeroize::Zeroizing;

    /// Name of the per-app Hello credential.
    const CREDENTIAL_NAME: &str = "CryptEnv.Vault";

    fn platform<E: std::fmt::Display>(e: E) -> HelloError {
        HelloError::Platform(e.to_string())
    }

    fn status_error(status: KeyCredentialStatus) -> HelloError {
        match status {
            KeyCredentialStatus::UserCanceled | KeyCredentialStatus::UserPrefersPassword => {
                HelloError::Cancelled
            }
            KeyCredentialStatus::NotFound => HelloError::Unusable,
            other => HelloError::Platform(format!("status code {}", other.0)),
        }
    }

    pub async fn check_availability() -> BiometricStatus {
        // IAsyncOperation::get() blocks; run it on the blocking thread pool.
        let result = tokio::task::spawn_blocking(|| {
            let availability =
                UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.get())?;
            Ok::<_, windows::core::Error>((availability, hello_supported()))
        })
        .await;

        let (availability, key_credentials) = match result {
            Ok(Ok(a)) => a,
            _ => return BiometricStatus::NotAvailable,
        };

        match availability {
            // Unlock needs KeyCredential support, not only a verifier.
            UserConsentVerifierAvailability::Available
            | UserConsentVerifierAvailability::DeviceBusy => {
                // DeviceBusy means biometrics exist but are temporarily occupied.
                if key_credentials {
                    BiometricStatus::Available
                } else {
                    BiometricStatus::NotAvailable
                }
            }
            UserConsentVerifierAvailability::DisabledByPolicy => BiometricStatus::DisabledByPolicy,
            UserConsentVerifierAvailability::NotConfiguredForUser => {
                BiometricStatus::NotConfiguredForUser
            }
            _ => BiometricStatus::NotAvailable,
        }
    }

    pub fn hello_supported() -> bool {
        KeyCredentialManager::IsSupportedAsync()
            .and_then(|op| op.get())
            .unwrap_or(false)
    }

    /// Creates the credential, replacing an existing one.
    pub fn hello_create() -> Result<KeyCredential, HelloError> {
        let name = HSTRING::from(CREDENTIAL_NAME);
        let result = KeyCredentialManager::RequestCreateAsync(
            &name,
            KeyCredentialCreationOption::ReplaceExisting,
        )
        .and_then(|op| op.get())
        .map_err(platform)?;
        match result.Status().map_err(platform)? {
            KeyCredentialStatus::Success => result.Credential().map_err(platform),
            other => Err(status_error(other)),
        }
    }

    pub fn hello_open() -> Result<KeyCredential, HelloError> {
        let name = HSTRING::from(CREDENTIAL_NAME);
        let result = KeyCredentialManager::OpenAsync(&name)
            .and_then(|op| op.get())
            .map_err(platform)?;
        match result.Status().map_err(platform)? {
            KeyCredentialStatus::Success => result.Credential().map_err(platform),
            other => Err(status_error(other)),
        }
    }

    /// Signs `challenge`; shows the Hello prompt.
    pub fn hello_sign(
        credential: &KeyCredential,
        challenge: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, HelloError> {
        let buffer: IBuffer = CryptographicBuffer::CreateFromByteArray(challenge).map_err(platform)?;
        let result = credential
            .RequestSignAsync(&buffer)
            .and_then(|op| op.get())
            .map_err(platform)?;
        match result.Status().map_err(platform)? {
            KeyCredentialStatus::Success => {}
            other => return Err(status_error(other)),
        }
        let signature = result.Result().map_err(platform)?;
        let mut bytes = Array::<u8>::new();
        CryptographicBuffer::CopyToByteArray(&signature, &mut bytes).map_err(platform)?;
        Ok(Zeroizing::new(bytes.to_vec()))
    }

    pub fn hello_delete() -> Result<(), HelloError> {
        let name = HSTRING::from(CREDENTIAL_NAME);
        KeyCredentialManager::DeleteAsync(&name)
            .and_then(|op| op.get())
            .map_err(platform)
    }

    pub struct WindowsHello;

    impl HelloSigner for WindowsHello {
        fn create(&self) -> Result<(), HelloError> {
            if !hello_supported() {
                return Err(HelloError::Unsupported);
            }
            hello_create().map(|_| ())
        }

        fn sign(&self, challenge: &[u8]) -> Result<Zeroizing<Vec<u8>>, HelloError> {
            let credential = hello_open()?;
            hello_sign(&credential, challenge)
        }

        fn delete(&self) -> Result<(), HelloError> {
            hello_delete()
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::check_availability;

#[cfg(not(target_os = "windows"))]
pub async fn check_availability() -> BiometricStatus {
    BiometricStatus::NotAvailable
}
