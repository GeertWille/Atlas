//! Composition root: wires the engine's use cases to their adapters.

use std::path::PathBuf;
use std::sync::Arc;

use adscan_browser::DemoSite;
use adscan_engine::application::AppError;
use adscan_engine::application::archive::Archive;
use adscan_engine::application::archive::network::NetworkView;
use adscan_engine::application::archive::source::ScriptSources;
use adscan_engine::application::configurations::ConfigurationService;
use adscan_engine::application::pipeline::PipelineDeps;
use adscan_engine::application::ports::{
    CaptureSource, Clock, EventSink, IdGenerator, SecretStore, SourceFormatter,
};
use adscan_engine::application::scans::ScanService;
use adscan_engine::application::settings::SettingsService;
use adscan_engine::infrastructure::anthropic::{AnthropicClient, DEFAULT_BASE_URL, RetryPolicy};
use adscan_engine::infrastructure::beautify::OxcFormatter;
use adscan_engine::infrastructure::keychain::OsKeychain;
use adscan_engine::infrastructure::pack_store::LocalPackStore;
use adscan_engine::infrastructure::public_files::HttpPublicFileFetcher;
use adscan_engine::infrastructure::secrets::{MemorySecretStore, VaultSecretStore};
use adscan_engine::infrastructure::sqlite::SqliteStore;
use adscan_engine::infrastructure::system::{SystemClock, UuidGenerator};
use adscan_engine::infrastructure::typesafe::{self, TypesafeClient};

const DATA_DIR_ENV: &str = "ADSCAN_DATA_DIR";
const BASE_URL_ENV: &str = "ADSCAN_ANTHROPIC_BASE_URL";
const API_KEY_ENV: &str = "ADSCAN_API_KEY";
const TYPESAFE_BASE_URL_ENV: &str = "ADSCAN_TYPESAFE_BASE_URL";
const TYPESAFE_API_KEY_ENV: &str = "ADSCAN_TYPESAFE_API_KEY";
const PROXY_PASSWORD_ENV: &str = "ADSCAN_PROXY_PASSWORD";

pub struct AppConfig {
    /// Directory of the SQLite file.
    pub data_dir: PathBuf,
    pub anthropic_base_url: String,
    pub typesafe_base_url: String,
    /// When any secret override is set, every secret lives in memory and the
    /// keychain is never touched.
    pub api_key_override: Option<String>,
    pub typesafe_api_key_override: Option<String>,
    /// Stands in for the password of every proxy profile.
    pub proxy_password_override: Option<String>,
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
            proxy_password_override: non_empty(PROXY_PASSWORD_ENV),
        }
    }
}

/// Held for as long as the app runs: says that this process is the one that
/// may clean up what an earlier run left in the data directory.
const INSTANCE_LOCK_FILE: &str = "instance.lock";

/// The lock of the first app running on `data_dir`, or `None` when another
/// one holds it (or the lock cannot be taken at all).
fn first_instance(data_dir: &std::path::Path) -> Option<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join(INSTANCE_LOCK_FILE))
        .ok()?;
    lock.try_lock().ok()?;
    Some(lock)
}

pub struct AppState {
    pub scans: Arc<ScanService>,
    pub settings: Arc<SettingsService>,
    pub configurations: Arc<ConfigurationService>,
    pub network: NetworkView,
    pub sources: ScriptSources,
    /// The built-in demo publisher page; it is served for as long as the app runs.
    pub demo_site: DemoSite,
    /// Released when the app ends, by the system if need be.
    _instance: Option<std::fs::File>,
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
        let secrets: Arc<dyn SecretStore> = match (
            config.api_key_override,
            config.typesafe_api_key_override,
            config.proxy_password_override,
        ) {
            (None, None, None) => Arc::new(VaultSecretStore::new(
                Arc::new(OsKeychain),
                Arc::new(store.clone()),
            )),
            (anthropic, typesafe, proxy_password) => Arc::new(
                MemorySecretStore::with_keys(anthropic, typesafe)
                    .with_proxy_password(proxy_password),
            ),
        };
        let archive = Archive {
            packs: Arc::new(LocalPackStore::new(&config.data_dir)),
            index: Arc::new(store.clone()),
        };
        let formatter: Arc<dyn SourceFormatter> = Arc::new(OxcFormatter::default());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let ids: Arc<dyn IdGenerator> = Arc::new(UuidGenerator);
        let settings = Arc::new(SettingsService::new(
            Arc::new(store.clone()),
            secrets,
            llm.clone(),
            judge.clone(),
            Arc::new(store.clone()),
            Arc::clone(&ids),
            archive.clone(),
        ));
        let configurations = Arc::new(ConfigurationService::new(
            Arc::new(store.clone()),
            Arc::clone(&settings),
            Arc::clone(&clock),
            Arc::clone(&ids),
        ));
        let scans = Arc::new(ScanService::new(
            PipelineDeps {
                scans: Arc::new(store.clone()),
                logs: Arc::new(store.clone()),
                llm,
                judge,
                capture,
                files: Arc::new(HttpPublicFileFetcher),
                formatter: Arc::clone(&formatter),
                archive: archive.clone(),
                events,
                clock,
                ids,
            },
            Arc::clone(&settings),
            Arc::new(store.clone()),
        ));
        // A second app on the same data directory finds scans that are
        // running, not interrupted, and packs that are being written, not
        // abandoned: only the first one cleans up.
        let instance = first_instance(&config.data_dir);
        if instance.is_some() {
            scans.recover_interrupted().await?;
            // After the interrupted scans are closed: what they were still
            // writing, and files that a crash left without rows, are removed.
            archive.sweep().await?;
        }
        let network = NetworkView::new(archive.clone(), Arc::new(store.clone()));
        let sources =
            ScriptSources::new(archive, Arc::new(store.clone()), Arc::new(store), formatter);
        let demo_site = adscan_browser::start_demo_site().await.map_err(|error| {
            AppError::Internal(format!("could not start the demo site: {error}"))
        })?;
        Ok(Self {
            scans,
            settings,
            configurations,
            network,
            sources,
            demo_site,
            _instance: instance,
        })
    }
}
