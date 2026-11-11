//! Where the Anthropic API key is kept: the OS keychain, or process memory
//! when `ADSCAN_API_KEY` supplies it (CI, tests).

use std::sync::Mutex;

use async_trait::async_trait;

use crate::application::ports::{SecretError, SecretStore};

pub const KEYCHAIN_SERVICE: &str = "com.webclew.adtech-scanner";
const KEYCHAIN_ACCOUNT: &str = "anthropic-api-key";

pub struct KeychainSecretStore;

impl KeychainSecretStore {
    fn entry() -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(keychain_error)
    }

    /// Keychain calls can block on the OS (unlock prompts), so they run on a
    /// blocking thread.
    async fn blocking<T: Send + 'static>(
        work: impl FnOnce() -> Result<T, SecretError> + Send + 'static,
    ) -> Result<T, SecretError> {
        tokio::task::spawn_blocking(work)
            .await
            .map_err(|error| SecretError::Failed(error.to_string()))?
    }
}

#[async_trait]
impl SecretStore for KeychainSecretStore {
    async fn api_key(&self) -> Result<Option<String>, SecretError> {
        Self::blocking(|| match Self::entry()?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        })
        .await
    }

    async fn store_api_key(&self, key: &str) -> Result<(), SecretError> {
        let key = key.to_string();
        Self::blocking(move || Self::entry()?.set_password(&key).map_err(keychain_error)).await
    }

    async fn clear_api_key(&self) -> Result<(), SecretError> {
        Self::blocking(|| match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keychain_error(error)),
        })
        .await
    }
}

fn keychain_error(error: keyring::Error) -> SecretError {
    match error {
        keyring::Error::NoStorageAccess(_) | keyring::Error::NoDefaultStore => {
            SecretError::Unavailable(error.to_string())
        }
        // On Linux a missing Secret Service surfaces as a platform failure.
        keyring::Error::PlatformFailure(_) if cfg!(target_os = "linux") => {
            SecretError::Unavailable(error.to_string())
        }
        other => SecretError::Failed(other.to_string()),
    }
}

/// Keeps the key in process memory only.
#[derive(Default)]
pub struct MemorySecretStore {
    key: Mutex<Option<String>>,
}

impl MemorySecretStore {
    pub fn with_key(key: impl Into<String>) -> Self {
        Self {
            key: Mutex::new(Some(key.into())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.key.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn api_key(&self) -> Result<Option<String>, SecretError> {
        Ok(self.lock().clone())
    }

    async fn store_api_key(&self, key: &str) -> Result<(), SecretError> {
        *self.lock() = Some(key.to_string());
        Ok(())
    }

    async fn clear_api_key(&self) -> Result<(), SecretError> {
        *self.lock() = None;
        Ok(())
    }
}
