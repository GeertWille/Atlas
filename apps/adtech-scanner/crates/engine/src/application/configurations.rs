//! Saved scan configurations: one site's setup, kept to be run again.

use std::sync::Arc;

use adscan_model::{ScanConfiguration, ScanConfigurationDraft};
use tokio::sync::Mutex;

use super::AppError;
use super::ports::{Clock, IdGenerator, ScanConfigurationRepository};
use super::settings::SettingsService;
use crate::domain::DomainError;
use crate::domain::proxy::same_name;
use crate::domain::scan::ValidatedScan;

const MAX_NAME_CHARS: usize = 80;

pub struct ConfigurationService {
    configurations: Arc<dyn ScanConfigurationRepository>,
    settings: Arc<SettingsService>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    /// Held from the name check to the write, so two saves cannot both take
    /// the same name.
    saving: Mutex<()>,
}

impl ConfigurationService {
    pub fn new(
        configurations: Arc<dyn ScanConfigurationRepository>,
        settings: Arc<SettingsService>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
    ) -> Self {
        Self {
            configurations,
            settings,
            clock,
            ids,
            saving: Mutex::new(()),
        }
    }

    pub async fn list(&self) -> Result<Vec<ScanConfiguration>, AppError> {
        Ok(self.configurations.list().await?)
    }

    pub async fn get(&self, id: &str) -> Result<ScanConfiguration, AppError> {
        self.configurations
            .get(id)
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Creates the configuration, or with an id replaces the name and options
    /// of the one stored under it.
    pub async fn save(&self, draft: ScanConfigurationDraft) -> Result<ScanConfiguration, AppError> {
        let _saving = self.saving.lock().await;
        let name = draft.name.trim().to_string();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(DomainError::ConfigurationNameInvalid {
                max: MAX_NAME_CHARS,
            }
            .into());
        }
        let existing = self.configurations.list().await?;
        if existing
            .iter()
            .any(|other| Some(&other.id) != draft.id.as_ref() && same_name(&other.name, &name))
        {
            return Err(DomainError::ConfigurationNameTaken(name).into());
        }

        // The options must be able to start a scan today. What is stored are
        // the options as given, so a choice left open keeps following the
        // settings; only the URL is normalised and the proxy's snapshot set.
        let settings = self.settings.get().await?;
        let validated = ValidatedScan::new(draft.options.clone(), &settings, 0)?;
        let mut options = draft.options;
        options.url = validated.options.url;
        options.auto_scroll = validated.options.auto_scroll;
        // Every run plans its own visit.
        options.seed = None;
        if options.proxy.is_some() {
            options.proxy = Some(validated.proxy.choice);
        }

        let now = self.clock.now_ms();
        let configuration = match draft.id {
            Some(id) => {
                let stored = existing
                    .into_iter()
                    .find(|other| other.id == id)
                    .ok_or_else(|| not_found(&id))?;
                ScanConfiguration {
                    name,
                    options,
                    updated_at: now,
                    ..stored
                }
            }
            None => ScanConfiguration {
                id: self.ids.new_id(),
                name,
                options,
                created_at: now,
                updated_at: now,
                last_scan_id: None,
                last_run_at: None,
            },
        };
        self.configurations.save(&configuration).await?;
        Ok(configuration)
    }

    /// Scans started from the configuration stay, with its name as it was.
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        if self.configurations.delete(id).await? {
            Ok(())
        } else {
            Err(not_found(id))
        }
    }
}

fn not_found(id: &str) -> AppError {
    AppError::NotFound(format!("Scan configuration {id}"))
}
