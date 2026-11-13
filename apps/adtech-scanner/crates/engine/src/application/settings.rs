//! Settings, API key and proxy profile use cases.

use std::sync::Arc;

use adscan_model::{
    ApiKeyStatus, ApiProvider, ArchiveStorage, BrowserKind, BrowserbaseCredentials,
    BrowserbaseProject, ProxyChoice, ProxyProfileDraft, Settings, SettingsPatch,
};
use tokio::sync::Mutex;

use super::AppError;
use super::archive::Archive;
use super::llm::LlmError;
use super::ports::{
    IdGenerator, JudgeClient, LlmClient, ScanConfigurationRepository, SecretError, SecretName,
    SecretStore, SettingsRepository,
};
use crate::domain::DomainError;
use crate::domain::archive::kinds::validate_archive_limit;
use crate::domain::pricing::Budget;
use crate::domain::proxy::{self, ResolvedProxy};
use crate::domain::scan::{validate_extra_pages, validate_observe_seconds};

pub struct SettingsService {
    settings: Arc<dyn SettingsRepository>,
    secrets: Arc<dyn SecretStore>,
    llm: Arc<dyn LlmClient>,
    judge: Arc<dyn JudgeClient>,
    /// Asked before a proxy profile is deleted: one in use stays.
    configurations: Arc<dyn ScanConfigurationRepository>,
    ids: Arc<dyn IdGenerator>,
    /// The limit on stored scan data is a setting; changing it acts on the
    /// archives.
    archive: Archive,
    /// Held from load to save: the UI sends one patch per changed field, and
    /// two patches working from the same copy would undo each other.
    updating: Mutex<()>,
}

impl SettingsService {
    pub fn new(
        settings: Arc<dyn SettingsRepository>,
        secrets: Arc<dyn SecretStore>,
        llm: Arc<dyn LlmClient>,
        judge: Arc<dyn JudgeClient>,
        configurations: Arc<dyn ScanConfigurationRepository>,
        ids: Arc<dyn IdGenerator>,
        archive: Archive,
    ) -> Self {
        Self {
            settings,
            secrets,
            llm,
            judge,
            configurations,
            ids,
            archive,
            updating: Mutex::new(()),
        }
    }

    pub async fn get(&self) -> Result<Settings, AppError> {
        let mut settings = self.settings.load().await?.unwrap_or_default();
        let anthropic = SecretName::ApiKey(ApiProvider::Anthropic);
        let typesafe = SecretName::ApiKey(ApiProvider::Typesafe);
        let candidates = settings
            .proxies
            .iter()
            .map(|profile| SecretName::ProxyPassword(profile.id.clone()))
            .chain([anthropic.clone(), typesafe.clone()])
            .chain(BROWSERBASE_SECRETS)
            .collect();
        let stored = self.secrets.stored_among(candidates).await?;
        settings.has_api_key = stored.contains(&anthropic);
        settings.has_typesafe_key = stored.contains(&typesafe);
        // The key can disappear from the keychain behind the app's back.
        settings.jev_enabled &= settings.has_typesafe_key;
        let connected = BROWSERBASE_SECRETS.iter().all(|name| stored.contains(name));
        settings.browserbase.connected = connected;
        if !connected {
            settings.browserbase.project_name = None;
            settings.browserbase.concurrency = None;
            settings.default_browser = BrowserKind::Local;
        }
        for profile in &mut settings.proxies {
            profile.has_password = stored.contains(&SecretName::ProxyPassword(profile.id.clone()));
        }
        Ok(settings)
    }

    pub async fn update(&self, patch: SettingsPatch) -> Result<Settings, AppError> {
        let _updating = self.updating.lock().await;
        let typesafe = SecretName::ApiKey(ApiProvider::Typesafe);
        if patch.jev_enabled == Some(true)
            && !self
                .secrets
                .stored_among(vec![typesafe.clone()])
                .await?
                .contains(&typesafe)
        {
            return Err(AppError::InvalidInput(
                "Add a TypeSafe API key before turning on Jev.".into(),
            ));
        }
        if patch.default_browser == Some(BrowserKind::Browserbase)
            && !self.browserbase_connected().await?
        {
            return Err(AppError::InvalidInput(
                "Add a Browserbase API key before making Browserbase the default.".into(),
            ));
        }
        let mut settings = self.settings.load().await?.unwrap_or_default();
        let limit_changed = patch.archive_limit_bytes.is_some();
        apply_patch(&mut settings, patch)?;
        let limit = settings.archive_limit_bytes;
        self.save(settings).await?;
        if limit_changed {
            // A lower limit takes effect now, not at the next scan.
            self.archive.enforce_limit(limit, None).await?;
        }
        self.get().await
    }

    /// How much scan data is stored, against the limit.
    pub async fn archive_storage(&self) -> Result<ArchiveStorage, AppError> {
        let limit = self
            .settings
            .load()
            .await?
            .unwrap_or_default()
            .archive_limit_bytes;
        self.archive.storage(limit).await
    }

    /// Removes the network data of every finished scan. Their reports stay.
    pub async fn clear_archive(&self) -> Result<ArchiveStorage, AppError> {
        self.archive.clear().await?;
        self.archive_storage().await
    }

    /// Stores the key only when the provider's API accepts it.
    pub async fn set_api_key(
        &self,
        provider: ApiProvider,
        key: &str,
    ) -> Result<ApiKeyStatus, AppError> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(rejected("Enter an API key."));
        }
        let (validation, name) = match provider {
            ApiProvider::Anthropic => (self.llm.validate_key(key).await, "Anthropic"),
            ApiProvider::Typesafe => (self.judge.validate_key(key).await, "TypeSafe"),
        };
        match validation {
            Ok(()) => {
                self.secrets.store_api_key(provider, key).await?;
                Ok(ApiKeyStatus {
                    valid: true,
                    message: None,
                })
            }
            Err(LlmError::ApiKeyInvalid(_)) => {
                Ok(rejected(&format!("{name} rejected this API key.")))
            }
            Err(error) => Ok(rejected(&format!(
                "The key could not be verified ({error}). It was not saved."
            ))),
        }
    }

    /// Jev cannot stay on without its key, so clearing that key turns it off.
    pub async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), AppError> {
        self.secrets.clear_api_key(provider).await?;
        if provider == ApiProvider::Typesafe {
            let _updating = self.updating.lock().await;
            if let Some(mut settings) = self.settings.load().await?
                && settings.jev_enabled
            {
                settings.jev_enabled = false;
                self.save(settings).await?;
            }
        }
        Ok(())
    }

    /// `None` when no key is stored; scans then run without what that
    /// provider would have done. A keychain that refuses is an error instead.
    pub async fn api_key(&self, provider: ApiProvider) -> Result<Option<String>, AppError> {
        Ok(self.secrets.api_key(provider).await?)
    }

    /// `None` unless a key and its project are stored. A keychain that
    /// refuses is an error instead.
    pub async fn browserbase_credentials(
        &self,
    ) -> Result<Option<BrowserbaseCredentials>, AppError> {
        Ok(self.secrets.browserbase_credentials().await?)
    }

    /// Stores the credentials with what Browserbase said about their
    /// project, which is all the settings ever show of them.
    pub async fn connect_browserbase(
        &self,
        credentials: &BrowserbaseCredentials,
        project: &BrowserbaseProject,
    ) -> Result<(), AppError> {
        self.secrets
            .store_browserbase_credentials(credentials)
            .await?;
        self.record_browserbase_project(project).await
    }

    /// The project's name and limit as Browserbase last reported them.
    pub async fn record_browserbase_project(
        &self,
        project: &BrowserbaseProject,
    ) -> Result<(), AppError> {
        let _updating = self.updating.lock().await;
        let mut settings = self.settings.load().await?.unwrap_or_default();
        settings.browserbase.project_name = Some(project.name.clone());
        settings.browserbase.concurrency = Some(project.concurrency);
        self.save(settings).await
    }

    /// Removes the credentials. Browserbase cannot stay the default without
    /// them.
    pub async fn disconnect_browserbase(&self) -> Result<Settings, AppError> {
        self.secrets.clear_browserbase_credentials().await?;
        {
            let _updating = self.updating.lock().await;
            if let Some(mut settings) = self.settings.load().await? {
                settings.default_browser = BrowserKind::Local;
                settings.browserbase.project_name = None;
                settings.browserbase.concurrency = None;
                self.save(settings).await?;
            }
        }
        self.get().await
    }

    async fn browserbase_connected(&self) -> Result<bool, AppError> {
        let stored = self
            .secrets
            .stored_among(BROWSERBASE_SECRETS.to_vec())
            .await?;
        Ok(stored.len() == BROWSERBASE_SECRETS.len())
    }

    /// Creates the profile, or with an id replaces the one stored under it.
    /// Its password is untouched either way.
    pub async fn save_proxy_profile(&self, draft: ProxyProfileDraft) -> Result<Settings, AppError> {
        let _updating = self.updating.lock().await;
        let mut settings = self.settings.load().await?.unwrap_or_default();
        match draft.id.clone() {
            Some(id) => {
                let position = settings
                    .proxies
                    .iter()
                    .position(|profile| profile.id == id)
                    .ok_or_else(|| proxy_not_found(&id))?;
                settings.proxies[position] =
                    proxy::validated_profile(draft, id, &settings.proxies)?;
            }
            None => {
                let profile =
                    proxy::validated_profile(draft, self.ids.new_id(), &settings.proxies)?;
                settings.proxies.push(profile);
            }
        }
        settings
            .proxies
            .sort_by_key(|profile| profile.name.to_lowercase());
        // The defaults keep a snapshot of the profile's name and host.
        settings.default_proxy =
            ResolvedProxy::resolve(&settings.default_proxy, &settings.proxies)?.choice;
        settings.browserbase.default_proxy = match ResolvedProxy::resolve_for_browserbase(
            &settings.browserbase.default_proxy,
            &settings.proxies,
        ) {
            Ok(resolved) => resolved.choice,
            // The profile was moved to an address only this machine can
            // reach. It is the user's to edit for scans on this machine;
            // it can no longer be what a Browserbase browser leaves through.
            Err(DomainError::ProxyNotReachableFromCloud(_)) => ProxyChoice::None,
            Err(error) => return Err(error.into()),
        };
        self.save(settings).await?;
        self.get().await
    }

    /// Removes the profile and its password. A profile that a saved scan
    /// configuration runs through stays: that configuration would stop
    /// working without a word. The settings default falls back to the
    /// system's proxy instead, because it is in plain sight on the same screen.
    pub async fn delete_proxy_profile(&self, id: &str) -> Result<Settings, AppError> {
        let _updating = self.updating.lock().await;
        let mut settings = self.settings.load().await?.unwrap_or_default();
        let position = settings
            .proxies
            .iter()
            .position(|profile| profile.id == id)
            .ok_or_else(|| proxy_not_found(id))?;
        let users: Vec<String> = self
            .configurations
            .list()
            .await?
            .into_iter()
            .filter(|configuration| {
                configuration
                    .options
                    .proxy
                    .as_ref()
                    .and_then(proxy::profile_id)
                    == Some(id)
            })
            .map(|configuration| format!("\"{}\"", configuration.name))
            .collect();
        if !users.is_empty() {
            return Err(AppError::InvalidInput(format!(
                "The proxy \"{}\" is used by the scan configuration{} {}. Change or delete {} first.",
                settings.proxies[position].name,
                if users.len() == 1 { "" } else { "s" },
                users.join(", "),
                if users.len() == 1 { "it" } else { "them" },
            )));
        }
        match self.secrets.clear_proxy_password(id).await {
            // Without a keychain there is no stored password to leave behind,
            // and the profile must not become impossible to delete.
            Ok(()) | Err(SecretError::Unavailable(_)) => {}
            Err(error) => return Err(error.into()),
        }
        settings.proxies.remove(position);
        if proxy::profile_id(&settings.default_proxy) == Some(id) {
            settings.default_proxy = ProxyChoice::System;
        }
        if proxy::profile_id(&settings.browserbase.default_proxy) == Some(id) {
            settings.browserbase.default_proxy = ProxyChoice::None;
        }
        self.save(settings).await?;
        self.get().await
    }

    /// The password is stored as given: whether the proxy accepts it only
    /// shows when a scan runs through it.
    pub async fn set_proxy_password(&self, id: &str, password: &str) -> Result<Settings, AppError> {
        if password.is_empty() {
            return Err(AppError::InvalidInput("Enter the proxy password.".into()));
        }
        // Held so that a delete of the profile cannot run in between and
        // leave a password in the keychain that belongs to nothing.
        let _updating = self.updating.lock().await;
        self.require_proxy(id).await?;
        self.secrets.store_proxy_password(id, password).await?;
        self.get().await
    }

    pub async fn clear_proxy_password(&self, id: &str) -> Result<Settings, AppError> {
        self.require_proxy(id).await?;
        self.secrets.clear_proxy_password(id).await?;
        self.get().await
    }

    /// `None` when no password is stored; the proxy then rejects the scan,
    /// which says so. A keychain that refuses is an error instead.
    pub async fn proxy_password(&self, id: &str) -> Result<Option<String>, AppError> {
        Ok(self.secrets.proxy_password(id).await?)
    }

    async fn require_proxy(&self, id: &str) -> Result<(), AppError> {
        let settings = self.settings.load().await?.unwrap_or_default();
        if settings.proxies.iter().any(|profile| profile.id == id) {
            Ok(())
        } else {
            Err(proxy_not_found(id))
        }
    }

    /// Whether a key or password exists is the secret store's to say, on
    /// every read; this document never carries it.
    async fn save(&self, mut settings: Settings) -> Result<(), AppError> {
        settings.has_api_key = false;
        settings.has_typesafe_key = false;
        settings.browserbase.connected = false;
        for profile in &mut settings.proxies {
            profile.has_password = false;
        }
        Ok(self.settings.save(&settings).await?)
    }
}

const BROWSERBASE_SECRETS: [SecretName; 2] = [
    SecretName::BrowserbaseApiKey,
    SecretName::BrowserbaseProjectId,
];

fn proxy_not_found(id: &str) -> AppError {
    AppError::NotFound(format!("Proxy {id}"))
}

fn rejected(message: &str) -> ApiKeyStatus {
    ApiKeyStatus {
        valid: false,
        message: Some(message.to_string()),
    }
}

fn apply_patch(settings: &mut Settings, patch: SettingsPatch) -> Result<(), DomainError> {
    if let Some(model) = patch.filter_model {
        settings.filter_model = model_id(model)?;
    }
    if let Some(model) = patch.analysis_model {
        settings.analysis_model = model_id(model)?;
    }
    if let Some(effort) = patch.analysis_effort {
        settings.analysis_effort = effort;
    }
    if let Some(budget_usd) = patch.budget_usd {
        settings.budget_usd = Budget::new(budget_usd)?.limit_usd();
    }
    if let Some(chrome_path) = patch.chrome_path {
        settings.chrome_path = chrome_path
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());
    }
    if let Some(headless) = patch.headless {
        settings.headless = headless;
    }
    if let Some(device) = patch.default_device {
        settings.default_device = device;
    }
    if let Some(seconds) = patch.default_observe_seconds {
        validate_observe_seconds(seconds)?;
        settings.default_observe_seconds = seconds;
    }
    if let Some(theme) = patch.theme {
        settings.theme = theme;
    }
    if let Some(jev_enabled) = patch.jev_enabled {
        settings.jev_enabled = jev_enabled;
    }
    if let Some(check_sellers_json) = patch.check_sellers_json {
        settings.check_sellers_json = check_sellers_json;
    }
    if let Some(visitor) = patch.default_visitor {
        settings.default_visitor = visitor;
    }
    if let Some(pages) = patch.default_extra_pages {
        validate_extra_pages(pages)?;
        settings.default_extra_pages = pages;
    }
    if let Some(persistent_profile) = patch.persistent_profile {
        settings.persistent_profile = persistent_profile;
    }
    if let Some(choice) = patch.default_proxy {
        settings.default_proxy = ResolvedProxy::resolve(&choice, &settings.proxies)?.choice;
    }
    if let Some(keep) = patch.keep_network_data {
        settings.keep_network_data = keep;
    }
    if let Some(bytes) = patch.archive_limit_bytes {
        settings.archive_limit_bytes = validate_archive_limit(bytes)?;
    }
    if let Some(full) = patch.store_full_headers {
        settings.store_full_headers = full;
    }
    if let Some(browser) = patch.default_browser {
        settings.default_browser = browser;
    }
    if let Some(cloud) = patch.browserbase {
        if let Some(region) = cloud.region {
            settings.browserbase.region = region;
        }
        if let Some(choice) = cloud.default_proxy {
            settings.browserbase.default_proxy =
                ResolvedProxy::resolve_for_browserbase(&choice, &settings.proxies)?.choice;
        }
        if let Some(persistent_profile) = cloud.persistent_profile {
            settings.browserbase.persistent_profile = persistent_profile;
        }
    }
    Ok(())
}

fn model_id(model: String) -> Result<String, DomainError> {
    let model = model.trim();
    if model.is_empty() {
        Err(DomainError::EmptyModelId)
    } else {
        Ok(model.to_string())
    }
}
