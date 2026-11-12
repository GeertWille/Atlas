//! Composition root: wires the engine's use cases to their adapters.

use std::path::PathBuf;
use std::sync::Arc;

use adscan_browser::DemoSite;
use adscan_engine::application::AppError;
use adscan_engine::application::pipeline::PipelineDeps;
use adscan_engine::application::ports::{CaptureSource, EventSink, SecretStore};
use adscan_engine::application::scans::ScanService;
use adscan_engine::application::settings::SettingsService;
use adscan_engine::infrastructure::anthropic::{AnthropicClient, DEFAULT_BASE_URL, RetryPolicy};
use adscan_engine::infrastructure::beautify::OxcFormatter;
use adscan_engine::infrastructure::secrets::{KeychainSecretStore, MemorySecretStore};
use adscan_engine::infrastructure::sqlite::SqliteStore;
use adscan_engine::infrastructure::system::{SystemClock, UuidGenerator};
use adscan_engine::infrastructure::typesafe::{self, TypesafeClient};

const DATA_DIR_ENV: &str = "ADSCAN_DATA_DIR";
const BASE_URL_ENV: &str = "ADSCAN_ANTHROPIC_BASE_URL";
const API_KEY_ENV: &str = "ADSCAN_API_KEY";
const TYPESAFE_BASE_URL_ENV: &str = "ADSCAN_TYPESAFE_BASE_URL";
const TYPESAFE_API_KEY_ENV: &str = "ADSCAN_TYPESAFE_API_KEY";

pub struct AppConfig {
    /// Directory of the SQLite file.
    pub data_dir: PathBuf,
    pub anthropic_base_url: String,
    pub typesafe_base_url: String,
    /// When either key override is set, keys live in memory for both
    /// providers and the keychain is never touched.
    pub api_key_override: Option<String>,
    pub typesafe_api_key_override: Option<String>,
}

impl AppConfig {
    /// `default_data_dir` is the OS app-data directory; the `ADSCAN_*`
    /// variables override it for tests and local development.
    pub fn from_environment(default_data_dir: PathBuf) -> Self {
        let non_empty = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        Self {
            data_dir: non_empty(DATA_DIR_ENV).map_or(default_data_dir, PathBuf::from),
            anthropic_base_url: non_empty(BASE_URL_ENV)
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            typesafe_base_url: non_empty(TYPESAFE_BASE_URL_ENV)
                .unwrap_or_else(|| typesafe::DEFAULT_BASE_URL.to_string()),
            api_key_override: non_empty(API_KEY_ENV),
            typesafe_api_key_override: non_empty(TYPESAFE_API_KEY_ENV),
        }
    }
}

pub struct AppState {
    pub scans: Arc<ScanService>,
    pub settings: Arc<SettingsService>,
    /// The built-in demo publisher page; it is served for as long as the app runs.
    pub demo_site: DemoSite,
}

impl AppState {
    pub async fn build(
        config: AppConfig,
        capture: Arc<dyn CaptureSource>,
        events: Arc<dyn EventSink>,
    ) -> Result<Self, AppError> {
        let store = SqliteStore::open(&config.data_dir)?;
        let llm = Arc::new(AnthropicClient::new(
            &config.anthropic_base_url,
            RetryPolicy::default(),
        )?);
        let judge = Arc::new(TypesafeClient::new(
            &config.typesafe_base_url,
            typesafe::default_retry(),
        )?);
        let secrets: Arc<dyn SecretStore> =
            match (config.api_key_override, config.typesafe_api_key_override) {
                (None, None) => Arc::new(KeychainSecretStore),
                (anthropic, typesafe) => {
                    Arc::new(MemorySecretStore::with_keys(anthropic, typesafe))
                }
            };
        let settings = Arc::new(SettingsService::new(
            Arc::new(store.clone()),
            secrets,
            llm.clone(),
            judge.clone(),
        ));
        let scans = Arc::new(ScanService::new(
            PipelineDeps {
                scans: Arc::new(store.clone()),
                logs: Arc::new(store),
                llm,
                judge,
                capture,
                formatter: Arc::new(OxcFormatter::default()),
                events,
                clock: Arc::new(SystemClock),
                ids: Arc::new(UuidGenerator),
            },
            Arc::clone(&settings),
        ));
        scans.recover_interrupted().await?;
        let demo_site = adscan_browser::start_demo_site().await.map_err(|error| {
            AppError::Internal(format!("could not start the demo site: {error}"))
        })?;
        Ok(Self {
            scans,
            settings,
            demo_site,
        })
    }
}
