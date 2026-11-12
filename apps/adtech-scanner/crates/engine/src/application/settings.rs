//! Settings and API key use cases.

use std::sync::Arc;

use adscan_model::{ApiKeyStatus, ApiProvider, Settings, SettingsPatch};
use tokio::sync::Mutex;

use super::AppError;
use super::llm::LlmError;
use super::ports::{JudgeClient, LlmClient, SecretStore, SettingsRepository};
use crate::domain::DomainError;
use crate::domain::pricing::Budget;
use crate::domain::scan::validate_observe_seconds;

pub struct SettingsService {
    settings: Arc<dyn SettingsRepository>,
    secrets: Arc<dyn SecretStore>,
    llm: Arc<dyn LlmClient>,
    judge: Arc<dyn JudgeClient>,
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
    ) -> Self {
        Self {
            settings,
            secrets,
            llm,
            judge,
            updating: Mutex::new(()),
        }
    }

    pub async fn get(&self) -> Result<Settings, AppError> {
        let mut settings = self.settings.load().await?.unwrap_or_default();
        settings.has_api_key = self.api_key(ApiProvider::Anthropic).await.is_some();
        settings.has_typesafe_key = self.api_key(ApiProvider::Typesafe).await.is_some();
        // The key can disappear from the keychain behind the app's back.
        settings.jev_enabled &= settings.has_typesafe_key;
        Ok(settings)
    }

    pub async fn update(&self, patch: SettingsPatch) -> Result<Settings, AppError> {
        let _updating = self.updating.lock().await;
        if patch.jev_enabled == Some(true) && self.api_key(ApiProvider::Typesafe).await.is_none() {
            return Err(AppError::InvalidInput(
                "Add a TypeSafe API key before turning on Jev.".into(),
            ));
        }
        let mut settings = self.settings.load().await?.unwrap_or_default();
        apply_patch(&mut settings, patch)?;
        self.save(settings).await?;
        self.get().await
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

    /// `None` when no key is stored or the keychain cannot be read; scans then
    /// run without what that provider would have done.
    pub async fn api_key(&self, provider: ApiProvider) -> Option<String> {
        self.secrets.api_key(provider).await.ok().flatten()
    }

    /// Whether a key exists is asked of the keychain on every read, never stored.
    async fn save(&self, mut settings: Settings) -> Result<(), AppError> {
        settings.has_api_key = false;
        settings.has_typesafe_key = false;
        Ok(self.settings.save(&settings).await?)
    }
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
