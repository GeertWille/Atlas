//! Where the API keys and proxy passwords are kept: one item in the OS
//! keychain, or process memory when the environment supplies them (CI, tests).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use adscan_model::ApiProvider;
use async_trait::async_trait;
use zeroize::{Zeroize, Zeroizing};

use super::keychain::Keychain;
use crate::application::ports::{
    RepoResult, SecretError, SecretName, SecretPresenceRepository, SecretStore,
};

/// The one keychain item. The system asks the user for permission per item,
/// so every secret shares it: one prompt at most, however many there are.
const VAULT_ACCOUNT: &str = "vault";

const PROXY_PREFIX: &str = "proxy:";

/// The name a secret goes by, in the vault and in the record of what exists.
pub(crate) fn secret_key(name: &SecretName) -> String {
    match name {
        SecretName::ApiKey(ApiProvider::Anthropic) => "anthropic".to_string(),
        SecretName::ApiKey(ApiProvider::Typesafe) => "typesafe".to_string(),
        SecretName::ProxyPassword(proxy_id) => format!("{PROXY_PREFIX}{proxy_id}"),
    }
}

pub(crate) fn secret_name(key: &str) -> Option<SecretName> {
    match key {
        "anthropic" => Some(SecretName::ApiKey(ApiProvider::Anthropic)),
        "typesafe" => Some(SecretName::ApiKey(ApiProvider::Typesafe)),
        _ => key
            .strip_prefix(PROXY_PREFIX)
            .map(|proxy_id| SecretName::ProxyPassword(proxy_id.to_string())),
    }
}

/// The account of the keychain item that held this secret alone, before the
/// vault. These names must not change, or secrets stored by earlier versions
/// would not be found.
fn legacy_account(name: &SecretName) -> String {
    match name {
        SecretName::ApiKey(ApiProvider::Anthropic) => "anthropic-api-key".to_string(),
        SecretName::ApiKey(ApiProvider::Typesafe) => "typesafe-api-key".to_string(),
        SecretName::ProxyPassword(proxy_id) => format!("proxy-password:{proxy_id}"),
    }
}

/// Every secret by its key. Deliberately without `Debug`.
#[derive(Default)]
struct Vault(BTreeMap<String, String>);

impl Vault {
    fn parse(document: &str) -> Result<Self, SecretError> {
        // The parser's own message could quote the document.
        serde_json::from_str(document).map(Self).map_err(|_| {
            SecretError::Failed("the stored secrets are not in the expected format".into())
        })
    }

    fn document(&self) -> Result<Zeroizing<String>, SecretError> {
        serde_json::to_string(&self.0)
            .map(Zeroizing::new)
            .map_err(|_| {
                SecretError::Failed("the secrets could not be prepared for storing".into())
            })
    }

    fn names(&self) -> HashSet<SecretName> {
        self.0.keys().filter_map(|key| secret_name(key)).collect()
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        self.0.values_mut().for_each(Zeroize::zeroize);
    }
}

/// Keeps every secret in one keychain item, read at most once per process.
///
/// Nothing touches the keychain until a secret is really needed: what exists
/// is answered from the presence record. The first need reads the item and
/// keeps it in memory; changes rewrite the item and the record together.
pub struct VaultSecretStore {
    keychain: Arc<dyn Keychain>,
    presence: Arc<dyn SecretPresenceRepository>,
    /// `None` until the first need. Held across the keychain read, so tasks
    /// that need a secret at the same moment cause one read and one prompt.
    vault: tokio::sync::Mutex<Option<Vault>>,
}

impl VaultSecretStore {
    pub fn new(keychain: Arc<dyn Keychain>, presence: Arc<dyn SecretPresenceRepository>) -> Self {
        Self {
            keychain,
            presence,
            vault: tokio::sync::Mutex::new(None),
        }
    }

    async fn secret(&self, name: SecretName) -> Result<Option<String>, SecretError> {
        let mut vault = self.vault.lock().await;
        if vault.is_none() && !self.recorded().await?.contains(&name) {
            return Ok(None);
        }
        let vault = self.unlocked(&mut vault).await?;
        Ok(vault.0.get(&secret_key(&name)).cloned())
    }

    /// Stores `secret` under `name`, or with `None` removes it.
    async fn change(&self, name: SecretName, secret: Option<&str>) -> Result<(), SecretError> {
        let mut vault = self.vault.lock().await;
        if secret.is_none() && vault.is_none() && !self.recorded().await?.contains(&name) {
            return Ok(());
        }
        let vault = self.unlocked(&mut vault).await?;
        let mut changed = Vault(vault.0.clone());
        let key = secret_key(&name);
        match secret {
            Some(secret) => {
                changed.0.insert(key, secret.to_string());
            }
            None => {
                if changed.0.remove(&key).is_none() {
                    return Ok(());
                }
            }
        }
        // An emptied vault is still written, not deleted: a missing item
        // would send the next launch back to the items of earlier versions.
        let document = changed.document()?;
        self.blocking(move |keychain| keychain.write(VAULT_ACCOUNT, &document))
            .await?;
        let names = changed.names();
        *vault = changed;
        self.record(&names).await
    }

    async fn unlocked<'a>(
        &self,
        vault: &'a mut Option<Vault>,
    ) -> Result<&'a mut Vault, SecretError> {
        let unlocked = match vault.take() {
            Some(unlocked) => unlocked,
            None => self.read_vault().await?,
        };
        Ok(vault.insert(unlocked))
    }

    /// The one keychain read of this process. A failure leaves the presence
    /// record alone: a refused read says nothing about what is stored.
    async fn read_vault(&self) -> Result<Vault, SecretError> {
        let recorded = self.recorded().await?;
        let stored = self
            .blocking(|keychain| keychain.read(VAULT_ACCOUNT))
            .await?
            .map(Zeroizing::new);
        let vault = match stored {
            Some(document) => Vault::parse(&document)?,
            None => self.adopt_legacy_items(&recorded).await?,
        };
        let names = vault.names();
        if names != recorded {
            self.record(&names).await?;
        }
        Ok(vault)
    }

    /// Earlier versions kept one keychain item per secret. They are moved
    /// into the vault the first time it is found missing. Which proxy
    /// passwords to look for comes from the presence record.
    async fn adopt_legacy_items(
        &self,
        recorded: &HashSet<SecretName>,
    ) -> Result<Vault, SecretError> {
        let mut candidates = recorded.clone();
        candidates.insert(SecretName::ApiKey(ApiProvider::Anthropic));
        candidates.insert(SecretName::ApiKey(ApiProvider::Typesafe));
        self.blocking(move |keychain| {
            let mut vault = Vault::default();
            let mut adopted = Vec::new();
            for name in &candidates {
                let account = legacy_account(name);
                if let Some(secret) = keychain.read(&account)? {
                    vault.0.insert(secret_key(name), secret);
                    adopted.push(account);
                }
            }
            if adopted.is_empty() {
                return Ok(vault);
            }
            // The old items go only once the vault holds their secrets.
            keychain.write(VAULT_ACCOUNT, &vault.document()?)?;
            for account in adopted {
                // A leftover is harmless: with a vault in place it is never
                // read again.
                let _ = keychain.delete(&account);
            }
            Ok(vault)
        })
        .await
    }

    async fn recorded(&self) -> Result<HashSet<SecretName>, SecretError> {
        self.presence.load_presence().await.map_err(record_error)
    }

    async fn record(&self, names: &HashSet<SecretName>) -> Result<(), SecretError> {
        self.presence
            .save_presence(names)
            .await
            .map_err(record_error)
    }

    /// Keychain calls block on the OS (permission prompts), so they run on a
    /// blocking thread.
    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn Keychain) -> Result<T, SecretError> + Send + 'static,
    ) -> Result<T, SecretError> {
        let keychain = Arc::clone(&self.keychain);
        tokio::task::spawn_blocking(move || work(keychain.as_ref()))
            .await
            .map_err(|error| SecretError::Failed(error.to_string()))?
    }
}

fn record_error(error: crate::application::ports::RepositoryError) -> SecretError {
    SecretError::Failed(format!(
        "the record of stored secrets could not be used: {error}"
    ))
}

#[async_trait]
impl SecretStore for VaultSecretStore {
    async fn stored_among(&self, candidates: Vec<SecretName>) -> RepoResult<HashSet<SecretName>> {
        let recorded = self.presence.load_presence().await?;
        Ok(candidates
            .into_iter()
            .filter(|name| recorded.contains(name))
            .collect())
    }

    async fn api_key(&self, provider: ApiProvider) -> Result<Option<String>, SecretError> {
        self.secret(SecretName::ApiKey(provider)).await
    }

    async fn store_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), SecretError> {
        self.change(SecretName::ApiKey(provider), Some(key)).await
    }

    async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), SecretError> {
        self.change(SecretName::ApiKey(provider), None).await
    }

    async fn proxy_password(&self, proxy_id: &str) -> Result<Option<String>, SecretError> {
        self.secret(SecretName::ProxyPassword(proxy_id.to_string()))
            .await
    }

    async fn store_proxy_password(
        &self,
        proxy_id: &str,
        password: &str,
    ) -> Result<(), SecretError> {
        self.change(
            SecretName::ProxyPassword(proxy_id.to_string()),
            Some(password),
        )
        .await
    }

    async fn clear_proxy_password(&self, proxy_id: &str) -> Result<(), SecretError> {
        self.change(SecretName::ProxyPassword(proxy_id.to_string()), None)
            .await
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
    async fn stored_among(&self, candidates: Vec<SecretName>) -> RepoResult<HashSet<SecretName>> {
        let mut stored = HashSet::new();
        for name in candidates {
            let exists = match &name {
                SecretName::ApiKey(provider) => self.lock().contains_key(provider),
                SecretName::ProxyPassword(proxy_id) => {
                    self.proxy_password_override.is_some()
                        || self.lock_proxy_passwords().contains_key(proxy_id)
                }
            };
            if exists {
                stored.insert(name);
            }
        }
        Ok(stored)
    }

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

    fn proxy(id: &str) -> SecretName {
        SecretName::ProxyPassword(id.to_string())
    }

    #[test]
    fn the_legacy_accounts_differ_and_the_anthropic_one_is_unchanged() {
        assert_eq!(
            legacy_account(&SecretName::ApiKey(ApiProvider::Anthropic)),
            "anthropic-api-key"
        );
        assert_eq!(
            legacy_account(&SecretName::ApiKey(ApiProvider::Typesafe)),
            "typesafe-api-key"
        );
    }

    #[test]
    fn each_proxy_profile_had_its_own_legacy_account() {
        assert_eq!(legacy_account(&proxy("p1")), "proxy-password:p1");
        assert_ne!(legacy_account(&proxy("p1")), legacy_account(&proxy("p2")));
    }

    #[test]
    fn every_secret_name_survives_the_round_trip_through_its_key() {
        for name in [
            SecretName::ApiKey(ApiProvider::Anthropic),
            SecretName::ApiKey(ApiProvider::Typesafe),
            proxy("p1"),
            proxy("with:colon"),
        ] {
            assert_eq!(secret_name(&secret_key(&name)), Some(name));
        }
        assert_eq!(secret_name("something-else"), None);
    }

    #[test]
    fn a_vault_that_cannot_be_parsed_is_reported_without_quoting_it() {
        let Err(error) = Vault::parse("{\"anthropic\": sk-ant-leaked") else {
            panic!("a broken document must not parse");
        };
        assert!(!error.to_string().contains("sk-ant-leaked"));
    }

    #[tokio::test]
    async fn what_exists_in_memory_is_answered_per_candidate() {
        let store = MemorySecretStore::with_key("anthropic-key");
        store.store_proxy_password("p1", "secret-1").await.unwrap();
        let candidates = vec![
            SecretName::ApiKey(ApiProvider::Anthropic),
            SecretName::ApiKey(ApiProvider::Typesafe),
            proxy("p1"),
            proxy("p2"),
        ];
        assert_eq!(
            store.stored_among(candidates.clone()).await.unwrap(),
            HashSet::from([SecretName::ApiKey(ApiProvider::Anthropic), proxy("p1")])
        );

        let with_override =
            MemorySecretStore::default().with_proxy_password(Some("from-env".into()));
        assert_eq!(
            with_override.stored_among(candidates).await.unwrap(),
            HashSet::from([proxy("p1"), proxy("p2")])
        );
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
