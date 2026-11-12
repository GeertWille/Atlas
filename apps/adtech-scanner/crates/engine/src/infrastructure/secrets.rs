//! Where the API keys and proxy passwords are kept: the OS keychain, or
//! process memory when the environment supplies them (CI, tests).

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

/// One keychain item per proxy profile, so deleting a profile removes
/// exactly its own password.
fn proxy_account(proxy_id: &str) -> String {
    format!("proxy-password:{proxy_id}")
}

pub struct KeychainSecretStore;

impl KeychainSecretStore {
    fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(keychain_error)
    }

    async fn read(account: String) -> Result<Option<String>, SecretError> {
        Self::blocking(move || match Self::entry(&account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        })
        .await
    }

    async fn write(account: String, secret: String) -> Result<(), SecretError> {
        Self::blocking(move || {
            Self::entry(&account)?
                .set_password(&secret)
                .map_err(keychain_error)
        })
        .await
    }

    async fn remove(account: String) -> Result<(), SecretError> {
        Self::blocking(move || match Self::entry(&account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keychain_error(error)),
        })
        .await
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
        Self::read(keychain_account(provider).to_string()).await
    }

    async fn store_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), SecretError> {
        Self::write(keychain_account(provider).to_string(), key.to_string()).await
    }

    async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), SecretError> {
        Self::remove(keychain_account(provider).to_string()).await
    }

    async fn proxy_password(&self, proxy_id: &str) -> Result<Option<String>, SecretError> {
        Self::read(proxy_account(proxy_id)).await
    }

    async fn store_proxy_password(
        &self,
        proxy_id: &str,
        password: &str,
    ) -> Result<(), SecretError> {
        Self::write(proxy_account(proxy_id), password.to_string()).await
    }

    async fn clear_proxy_password(&self, proxy_id: &str) -> Result<(), SecretError> {
        Self::remove(proxy_account(proxy_id)).await
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

/// Keeps the secrets in process memory only.
#[derive(Default)]
pub struct MemorySecretStore {
    keys: Mutex<HashMap<ApiProvider, String>>,
    proxy_passwords: Mutex<HashMap<String, String>>,
    /// Stands in for the password of every proxy that has none of its own.
    proxy_password_override: Option<String>,
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
            ..Self::default()
        }
    }

    /// Every proxy profile answers with `password` until it is given its own.
    pub fn with_proxy_password(mut self, password: Option<String>) -> Self {
        self.proxy_password_override = password;
        self
    }

    fn lock_proxy_passwords(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.proxy_passwords
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    async fn proxy_password(&self, proxy_id: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .lock_proxy_passwords()
            .get(proxy_id)
            .cloned()
            .or_else(|| self.proxy_password_override.clone()))
    }

    async fn store_proxy_password(
        &self,
        proxy_id: &str,
        password: &str,
    ) -> Result<(), SecretError> {
        self.lock_proxy_passwords()
            .insert(proxy_id.to_string(), password.to_string());
        Ok(())
    }

    async fn clear_proxy_password(&self, proxy_id: &str) -> Result<(), SecretError> {
        self.lock_proxy_passwords().remove(proxy_id);
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

    #[test]
    fn each_proxy_profile_has_its_own_keychain_account() {
        assert_eq!(proxy_account("p1"), "proxy-password:p1");
        assert_ne!(proxy_account("p1"), proxy_account("p2"));
    }

    #[tokio::test]
    async fn proxy_passwords_are_kept_per_profile() {
        let store = MemorySecretStore::default();
        store.store_proxy_password("p1", "secret-1").await.unwrap();
        assert_eq!(
            store.proxy_password("p1").await.unwrap().as_deref(),
            Some("secret-1")
        );
        assert_eq!(store.proxy_password("p2").await.unwrap(), None);
        store.clear_proxy_password("p1").await.unwrap();
        assert_eq!(store.proxy_password("p1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_environment_password_stands_in_until_a_profile_has_its_own() {
        let store = MemorySecretStore::default().with_proxy_password(Some("from-env".into()));
        assert_eq!(
            store.proxy_password("p1").await.unwrap().as_deref(),
            Some("from-env")
        );
        store.store_proxy_password("p1", "own").await.unwrap();
        assert_eq!(
            store.proxy_password("p1").await.unwrap().as_deref(),
            Some("own")
        );
    }
}
