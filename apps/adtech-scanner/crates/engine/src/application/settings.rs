//! Settings and API key use cases.

use std::sync::Arc;

use adscan_model::{ApiKeyStatus, Settings, SettingsPatch};

use super::AppError;
use super::llm::LlmError;
use super::ports::{LlmClient, SecretStore, SettingsRepository};
use crate::domain::DomainError;
use crate::domain::pricing::Budget;
use crate::domain::scan::validate_observe_seconds;

pub struct SettingsService {
    settings: Arc<dyn SettingsRepository>,
    secrets: Arc<dyn SecretStore>,
    llm: Arc<dyn LlmClient>,
}

impl SettingsService {
    pub fn new(
        settings: Arc<dyn SettingsRepository>,
        secrets: Arc<dyn SecretStore>,
        llm: Arc<dyn LlmClient>,
    ) -> Self {
        Self {
            settings,
            secrets,
            llm,
        }
    }

    pub async fn get(&self) -> Result<Settings, AppError> {
        let mut settings = self.settings.load().await?.unwrap_or_default();
        settings.has_api_key = self.api_key().await.is_some();
        Ok(settings)
    }

    pub async fn update(&self, patch: SettingsPatch) -> Result<Settings, AppError> {
        let mut settings = self.settings.load().await?.unwrap_or_default();
        apply_patch(&mut settings, patch)?;
        settings.has_api_key = false;
        self.settings.save(&settings).await?;
        self.get().await
    }

    /// Stores the key only when the API accepts it.
    pub async fn set_api_key(&self, key: &str) -> Result<ApiKeyStatus, AppError> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(rejected("Enter an API key."));
        }
        match self.llm.validate_key(key).await {
            Ok(()) => {
                self.secrets.store_api_key(key).await?;
                Ok(ApiKeyStatus {
                    valid: true,
                    message: None,
                })
            }
            Err(LlmError::ApiKeyInvalid(_)) => Ok(rejected("Anthropic rejected this API key.")),
            Err(error) => Ok(rejected(&format!(
                "The key could not be verified ({error}). It was not saved."
            ))),
        }
    }

    pub async fn clear_api_key(&self) -> Result<(), AppError> {
        Ok(self.secrets.clear_api_key().await?)
    }

    /// `None` when no key is stored or the keychain cannot be read; scans then
    /// run without the AI phases.
    pub async fn api_key(&self) -> Option<String> {
        self.secrets.api_key().await.ok().flatten()
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
