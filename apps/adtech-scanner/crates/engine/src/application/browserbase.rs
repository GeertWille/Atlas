//! Browserbase use cases apart from a scan: the credentials, what the
//! account reports, the profile kept there, and the sessions an earlier run
//! of the app may have left behind.

use std::sync::Arc;

use adscan_model::{
    BrowserbaseCredentials, BrowserbaseCredentialsStatus, BrowserbaseProbe, BrowserbaseProfileInfo,
    BrowserbaseProject, BrowserbaseRegion, BrowserbaseStatus, Settings,
};
use tokio::sync::Semaphore;

use super::AppError;
use super::ports::{BrowserbaseApi, BrowserbaseError, BrowserbaseLedger, Clock};
use super::settings::SettingsService;
use crate::domain::DomainError;

/// The longest a Browserbase session can live (its `timeout` is at most
/// 21600 s), with a margin. A session started longer ago has ended whatever
/// became of the app.
const LONGEST_SESSION_MS: f64 = (6.0 * 60.0 + 5.0) * 60.0 * 1_000.0;

pub struct BrowserbaseService {
    api: Arc<dyn BrowserbaseApi>,
    ledger: Arc<dyn BrowserbaseLedger>,
    settings: Arc<SettingsService>,
    clock: Arc<dyn Clock>,
    /// One browser at a time in the profile kept at Browserbase: two
    /// sessions on one context would overwrite each other's cookies.
    profile: Arc<Semaphore>,
}

impl BrowserbaseService {
    pub fn new(
        api: Arc<dyn BrowserbaseApi>,
        ledger: Arc<dyn BrowserbaseLedger>,
        settings: Arc<SettingsService>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            api,
            ledger,
            settings,
            clock,
            profile: Arc::new(Semaphore::new(1)),
        }
    }

    /// The turn to run a browser in the profile kept at Browserbase.
    pub fn profile_turn(&self) -> Arc<Semaphore> {
        Arc::clone(&self.profile)
    }

    /// Stores the key and its project only when Browserbase accepts them.
    /// Asking for the key's projects costs nothing.
    pub async fn set_credentials(
        &self,
        api_key: &str,
        project_id: Option<&str>,
    ) -> Result<BrowserbaseCredentialsStatus, AppError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Ok(rejected("Enter a Browserbase API key.", Vec::new()));
        }
        let mut projects = match self.api.projects(api_key).await {
            Ok(projects) => projects,
            Err(BrowserbaseError::Auth(_)) => {
                return Ok(rejected(
                    "Browserbase did not accept the API key.",
                    Vec::new(),
                ));
            }
            Err(error) => {
                return Ok(rejected(
                    &format!("The key could not be verified ({error}). It was not saved."),
                    Vec::new(),
                ));
            }
        };
        let wanted = project_id.map(str::trim).filter(|id| !id.is_empty());
        let project = match wanted {
            Some(id) => match projects.iter().position(|project| project.id == id) {
                Some(position) => projects.swap_remove(position),
                None => {
                    return Ok(rejected(
                        "This Browserbase project was not found for this key.",
                        Vec::new(),
                    ));
                }
            },
            None => match projects.len() {
                0 => return Ok(rejected("This key has no Browserbase project.", projects)),
                1 => projects.swap_remove(0),
                _ => {
                    return Ok(rejected(
                        "This key has more than one Browserbase project. Choose the one to use.",
                        projects,
                    ));
                }
            },
        };
        let credentials = BrowserbaseCredentials {
            api_key: api_key.to_string(),
            project_id: project.id.clone(),
        };
        // A kept profile belongs to the project it was made in.
        if let Some(previous) = self.settings.browserbase_credentials().await?
            && previous.project_id != credentials.project_id
        {
            self.drop_profile(&previous).await?;
        }
        self.settings
            .connect_browserbase(&credentials, &project)
            .await?;
        Ok(BrowserbaseCredentialsStatus {
            valid: true,
            message: None,
            projects: Vec::new(),
        })
    }

    /// Removes the credentials, and first the profile kept at Browserbase:
    /// without the key the app could never delete it again.
    pub async fn clear_credentials(&self) -> Result<Settings, AppError> {
        if let Some(credentials) = self.settings.browserbase_credentials().await? {
            self.drop_profile(&credentials).await?;
        }
        self.settings.disconnect_browserbase().await
    }

    /// The project, its running totals and how many browsers run in it now.
    /// Starts no browser and costs nothing.
    pub async fn status(&self) -> Result<BrowserbaseStatus, AppError> {
        let credentials = self.credentials().await?;
        let project = self.api.project(&credentials).await?;
        let usage = self.api.project_usage(&credentials).await?;
        let running_sessions = self.api.running_session_count(&credentials).await?;
        self.settings.record_browserbase_project(&project).await?;
        Ok(BrowserbaseStatus {
            project_name: project.name,
            concurrency: project.concurrency,
            browser_minutes: usage.browser_minutes,
            proxy_bytes: usage.proxy_bytes,
            running_sessions,
        })
    }

    /// What a page in a Browserbase browser sees. Starts one browser for a
    /// moment, which Browserbase bills as a minute.
    pub async fn probe(
        &self,
        region: Option<BrowserbaseRegion>,
    ) -> Result<BrowserbaseProbe, AppError> {
        let credentials = self.credentials().await?;
        let region = match region {
            Some(region) => region,
            None => self.settings.get().await?.browserbase.region,
        };
        let install_id = self.ledger.install_id().await?;
        Ok(self.api.probe(&credentials, region, &install_id).await?)
    }

    /// Answered from the app's own record: no request is made.
    pub async fn profile(&self) -> Result<BrowserbaseProfileInfo, AppError> {
        let contexts = self.ledger.contexts().await?;
        Ok(BrowserbaseProfileInfo {
            exists: !contexts.is_empty(),
            last_used_at: contexts
                .iter()
                .filter_map(|context| context.last_used_at)
                .reduce(f64::max),
            in_use: self.profile.available_permits() == 0,
        })
    }

    /// Deletes the contexts at Browserbase and forgets them.
    pub async fn clear_profile(&self) -> Result<BrowserbaseProfileInfo, AppError> {
        // Held while clearing, so no scan can start a browser in a context
        // that is being deleted.
        let turn = self
            .profile
            .try_acquire()
            .map_err(|_| AppError::ProfileBusy)?;
        if !self.ledger.contexts().await?.is_empty() {
            let credentials = self.credentials().await?;
            self.delete_contexts(&credentials).await?;
        }
        drop(turn);
        self.profile().await
    }

    /// Releases the sessions that an earlier run of the app started and did
    /// not see end: those on record from before `app_started_at` (epoch
    /// ms). Returns how many it released.
    ///
    /// Without such sessions on record nothing happens, and the keychain is
    /// not read. Only sessions that are both on this installation's record
    /// and reported by Browserbase with this installation's id are touched:
    /// the project may be shared with other applications. A session of a
    /// scan that started since is left alone.
    pub async fn release_abandoned_sessions(&self, app_started_at: f64) -> Result<u32, AppError> {
        self.ledger.forget_released_sessions().await?;
        let now = self.clock.now_ms();
        let (open, expired): (Vec<_>, Vec<_>) = self
            .ledger
            .open_sessions()
            .await?
            .into_iter()
            .filter(|session| session.created_at < app_started_at)
            .partition(|session| now - session.created_at < LONGEST_SESSION_MS);
        for session in &expired {
            self.ledger
                .session_released(&session.session_id, now)
                .await?;
        }
        if open.is_empty() {
            return Ok(0);
        }
        let Some(credentials) = self.settings.browserbase_credentials().await? else {
            // Nothing can be done for them without the key; Browserbase
            // ends each at its own time limit.
            for session in &open {
                self.ledger
                    .session_released(&session.session_id, now)
                    .await?;
            }
            return Ok(0);
        };
        let install_id = self.ledger.install_id().await?;
        let running = self
            .api
            .running_sessions_of(&credentials, &install_id)
            .await?;
        let mut released = 0;
        for session in &open {
            if running.contains(&session.session_id) {
                self.api
                    .release_session(&credentials, &session.session_id)
                    .await?;
                released += 1;
            }
            self.ledger
                .session_released(&session.session_id, self.clock.now_ms())
                .await?;
        }
        Ok(released)
    }

    async fn credentials(&self) -> Result<BrowserbaseCredentials, AppError> {
        self.settings
            .browserbase_credentials()
            .await?
            .ok_or_else(|| DomainError::BrowserbaseNotSetUp.into())
    }

    /// Best effort before the credentials go or change: a profile that
    /// cannot be deleted now (a scan uses it, Browserbase is unreachable) is
    /// forgotten all the same, because the key that could delete it later is
    /// about to be gone.
    async fn drop_profile(&self, credentials: &BrowserbaseCredentials) -> Result<(), AppError> {
        if let Ok(_turn) = self.profile.try_acquire() {
            let _ = self.delete_contexts(credentials).await;
        }
        Ok(self.ledger.forget_contexts().await?)
    }

    async fn delete_contexts(&self, credentials: &BrowserbaseCredentials) -> Result<(), AppError> {
        for context in self.ledger.contexts().await? {
            self.api
                .delete_context(credentials, &context.context_id)
                .await?;
        }
        Ok(self.ledger.forget_contexts().await?)
    }
}

fn rejected(message: &str, projects: Vec<BrowserbaseProject>) -> BrowserbaseCredentialsStatus {
    BrowserbaseCredentialsStatus {
        valid: false,
        message: Some(message.to_string()),
        projects,
    }
}
