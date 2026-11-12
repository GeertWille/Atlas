//! Where the API keys are kept: the OS keychain, or process memory when the
//! environment supplies them (CI, tests).

use std::collections::HashMap;
use std::sync::Mutex;

use adscan_model::ApiProvider;
use async_trait::async_trait;

use crate::application::ports::{SecretError, SecretStore};

const KEYCHAIN_SERVICE: &str = "com.webclew.adtech-scanner";

/// The Anthropic account name predates the second provider and must not
/// change, or keys stored by earlier versions would be lost.
fn keychain_account(provider: ApiProvider) -> &'static str {
    match provider {
        ApiProvider::Anthropic => "anthropic-api-key",
        ApiProvider::Typesafe => "typesafe-api-key",
    }
}

pub struct KeychainSecretStore;

impl KeychainSecretStore {
    fn entry(provider: ApiProvider) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, keychain_account(provider)).map_err(keychain_error)
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
    async fn api_key(&self, provider: ApiProvider) -> Result<Option<String>, SecretError> {
        Self::blocking(move || match Self::entry(provider)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        })
        .await
    }

    async fn store_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), SecretError> {
        let key = key.to_string();
        Self::blocking(move || {
            Self::entry(provider)?
                .set_password(&key)
                .map_err(keychain_error)
        })
        .await
    }

    async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), SecretError> {
        Self::blocking(move || match Self::entry(provider)?.delete_credential() {
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

/// Keeps the keys in process memory only.
#[derive(Default)]
pub struct MemorySecretStore {
    keys: Mutex<HashMap<ApiProvider, String>>,
}

impl MemorySecretStore {
    /// A store that already holds the Anthropic key.
    pub fn with_key(key: impl Into<String>) -> Self {
        Self::with_keys(Some(key.into()), None)
    }

    pub fn with_keys(anthropic: Option<String>, typesafe: Option<String>) -> Self {
        let keys = [
            (ApiProvider::Anthropic, anthropic),
            (ApiProvider::Typesafe, typesafe),
        ]
        .into_iter()
        .filter_map(|(provider, key)| Some((provider, key?)))
        .collect();
        Self {
            keys: Mutex::new(keys),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ApiProvider, String>> {
        self.keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn api_key(&self, provider: ApiProvider) -> Result<Option<String>, SecretError> {
        Ok(self.lock().get(&provider).cloned())
    }

    async fn store_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), SecretError> {
        self.lock().insert(provider, key.to_string());
        Ok(())
    }

    async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), SecretError> {
        self.lock().remove(&provider);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn each_provider_has_its_own_key() {
        let store = MemorySecretStore::with_key("anthropic-key");
        assert_eq!(store.api_key(ApiProvider::Typesafe).await.unwrap(), None);

        store
            .store_api_key(ApiProvider::Typesafe, "typesafe-key")
            .await
            .unwrap();
        store.clear_api_key(ApiProvider::Anthropic).await.unwrap();

        assert_eq!(store.api_key(ApiProvider::Anthropic).await.unwrap(), None);
        assert_eq!(
            store
                .api_key(ApiProvider::Typesafe)
                .await
                .unwrap()
                .as_deref(),
            Some("typesafe-key")
        );
    }

    #[test]
    fn the_keychain_accounts_differ_and_the_anthropic_one_is_unchanged() {
        assert_eq!(
            keychain_account(ApiProvider::Anthropic),
            "anthropic-api-key"
        );
        assert_eq!(keychain_account(ApiProvider::Typesafe), "typesafe-api-key");
    }
}
