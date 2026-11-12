//! The operating system's keychain: generic-password items under the app's
//! service name, one per account.

use crate::application::ports::SecretError;

const KEYCHAIN_SERVICE: &str = "com.webclew.adtech-scanner";

/// Every call blocks on the OS and can put a permission prompt in front of
/// the user, so callers keep them off the async workers and make as few as
/// they can.
pub trait Keychain: Send + Sync {
    /// `None` when the account has no item.
    fn read(&self, account: &str) -> Result<Option<String>, SecretError>;
    fn write(&self, account: &str, secret: &str) -> Result<(), SecretError>;
    /// An account without an item is already deleted.
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

pub struct OsKeychain;

impl OsKeychain {
    fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(keychain_error)
    }
}

impl Keychain for OsKeychain {
    fn read(&self, account: &str) -> Result<Option<String>, SecretError> {
        match Self::entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    fn write(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        Self::entry(account)?
            .set_password(secret)
            .map_err(keychain_error)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keychain_error(error)),
        }
    }
}

fn keychain_error(error: keyring::Error) -> SecretError {
    match error {
        keyring::Error::NoStorageAccess(_) | keyring::Error::NoDefaultStore => {
            SecretError::Unavailable(error.to_string())
        }
        keyring::Error::PlatformFailure(cause) => platform_failure(cause.as_ref()),
        other => SecretError::Failed(other.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn platform_failure(cause: &(dyn std::error::Error + Send + Sync + 'static)) -> SecretError {
    const USER_CANCELED: i32 = -128;
    const AUTH_FAILED: i32 = -25293;
    // Writing looks the item up first and adds it when that fails, so a
    // refused lookup of an existing item surfaces as a duplicate.
    const DUPLICATE_ITEM: i32 = -25299;
    const INTERACTION_NOT_ALLOWED: i32 = -25308;
    let status = cause
        .downcast_ref::<security_framework::base::Error>()
        .map(|error| error.code());
    match status {
        Some(USER_CANCELED | AUTH_FAILED | DUPLICATE_ITEM) => SecretError::Denied,
        Some(INTERACTION_NOT_ALLOWED) => SecretError::Unavailable(cause.to_string()),
        _ => SecretError::Failed(cause.to_string()),
    }
}

/// A missing Secret Service surfaces as a platform failure.
#[cfg(target_os = "linux")]
fn platform_failure(cause: &(dyn std::error::Error + Send + Sync + 'static)) -> SecretError {
    SecretError::Unavailable(cause.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_failure(cause: &(dyn std::error::Error + Send + Sync + 'static)) -> SecretError {
    SecretError::Failed(cause.to_string())
}
