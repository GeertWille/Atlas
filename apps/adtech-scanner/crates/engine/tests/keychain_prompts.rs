//! How often the app touches the OS keychain. On macOS every read of an item
//! can put a permission prompt in front of the user, so the counts here are
//! the user experience: nothing at start or while browsing, one read on the
//! first real need, and a refusal reported as a refusal.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_engine::application::AppError;
use adscan_engine::application::ports::{
    CaptureError, SecretError, SecretName, SecretPresenceRepository,
};
use adscan_engine::infrastructure::keychain::Keychain;
use adscan_engine::infrastructure::sqlite::SqliteStore;
use adscan_model::{
    ApiProvider, ProxyChoice, ProxyProfileDraft, ProxyScheme, ScanOptions, ScanStatus,
    SettingsPatch,
};
use common::*;
use serde_json::{Value, json};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const OFFLINE: &str = "http://127.0.0.1:1";
const VAULT: &str = "vault";

/// The OS keychain layer, counting every access the way macOS would see it.
#[derive(Default)]
struct CountingKeychain {
    items: Mutex<HashMap<String, String>>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    deletes: AtomicUsize,
    /// What every read answers instead of the item.
    read_failure: Mutex<Option<SecretError>>,
    write_failure: Mutex<Option<SecretError>>,
    /// How long a read takes, as a user looking at a prompt would make it.
    read_time: Mutex<Duration>,
}

impl CountingKeychain {
    fn item(&self, account: &str) -> Option<String> {
        self.items.lock().unwrap().get(account).cloned()
    }

    fn put(&self, account: &str, secret: &str) {
        self.items
            .lock()
            .unwrap()
            .insert(account.to_string(), secret.to_string());
    }

    fn accounts(&self) -> Vec<String> {
        let mut accounts: Vec<String> = self.items.lock().unwrap().keys().cloned().collect();
        accounts.sort();
        accounts
    }

    /// Reads, writes, deletes.
    fn accesses(&self) -> (usize, usize, usize) {
        (
            self.reads.load(Ordering::SeqCst),
            self.writes.load(Ordering::SeqCst),
            self.deletes.load(Ordering::SeqCst),
        )
    }

    fn forget_accesses(&self) {
        self.reads.store(0, Ordering::SeqCst);
        self.writes.store(0, Ordering::SeqCst);
        self.deletes.store(0, Ordering::SeqCst);
    }

    fn fail_reads(&self, failure: Option<SecretError>) {
        *self.read_failure.lock().unwrap() = failure;
    }

    fn fail_writes(&self, failure: Option<SecretError>) {
        *self.write_failure.lock().unwrap() = failure;
    }
}

impl Keychain for CountingKeychain {
    fn read(&self, account: &str) -> Result<Option<String>, SecretError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(*self.read_time.lock().unwrap());
        match self.read_failure.lock().unwrap().clone() {
            Some(failure) => Err(failure),
            None => Ok(self.item(account)),
        }
    }

    fn write(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        match self.write_failure.lock().unwrap().clone() {
            Some(failure) => Err(failure),
            None => {
                self.put(account, secret);
                Ok(())
            }
        }
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.items.lock().unwrap().remove(account);
        Ok(())
    }
}

/// The app, started on `store` and `keychain`. Calling it again is a restart.
fn launch(
    behavior: CaptureBehavior,
    store: &SqliteStore,
    keychain: &Arc<CountingKeychain>,
) -> Harness {
    launch_against(behavior, store, keychain, OFFLINE, OFFLINE)
}

fn launch_against(
    behavior: CaptureBehavior,
    store: &SqliteStore,
    keychain: &Arc<CountingKeychain>,
    anthropic_base_url: &str,
    typesafe_base_url: &str,
) -> Harness {
    harness_with_keychain(
        behavior,
        Providers {
            anthropic_base_url,
            api_key: None,
            typesafe_base_url,
            typesafe_key: None,
        },
        store.clone(),
        keychain.clone(),
    )
}

fn anthropic() -> SecretName {
    SecretName::ApiKey(ApiProvider::Anthropic)
}

fn typesafe() -> SecretName {
    SecretName::ApiKey(ApiProvider::Typesafe)
}

fn proxy(id: &str) -> SecretName {
    SecretName::ProxyPassword(id.to_string())
}

fn vault_of(keychain: &CountingKeychain) -> Value {
    serde_json::from_str(&keychain.item(VAULT).expect("a vault item")).unwrap()
}

fn denied() -> AppError {
    AppError::Internal("Keychain access was denied".into())
}

/// An earlier run of the app that stored both keys, turned Jev on and saved
/// a proxy profile with a password. Returns the profile's id.
async fn an_earlier_run_stored_everything(
    store: &SqliteStore,
    keychain: &Arc<CountingKeychain>,
) -> String {
    let earlier = launch(CaptureBehavior::NoBrowser, store, keychain);
    earlier
        .secrets
        .store_api_key(ApiProvider::Anthropic, "sk-ant-stored")
        .await
        .unwrap();
    earlier
        .secrets
        .store_api_key(ApiProvider::Typesafe, "typesafe-stored")
        .await
        .unwrap();
    let saved = earlier
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            id: None,
            name: "Belgium".into(),
            scheme: ProxyScheme::Http,
            host: "be.proxy.example".into(),
            port: 3128,
            username: Some("scanner".into()),
        })
        .await
        .unwrap();
    let proxy_id = saved.proxies[0].id.clone();
    earlier
        .settings
        .set_proxy_password(&proxy_id, "s3cret-proxy")
        .await
        .unwrap();
    earlier
        .settings
        .update(SettingsPatch {
            jev_enabled: Some(true),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    keychain.forget_accesses();
    proxy_id
}

fn ai_scan_through(proxy_id: &str) -> ScanOptions {
    ScanOptions {
        proxy: Some(ProxyChoice::Profile {
            id: proxy_id.into(),
            name: None,
            host: None,
        }),
        ..scan_options(true)
    }
}

#[tokio::test]
async fn every_secret_shares_one_keychain_item() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let proxy_id = an_earlier_run_stored_everything(&store, &keychain).await;

    assert_eq!(keychain.accounts(), [VAULT]);
    assert_eq!(
        vault_of(&keychain),
        json!({
            "anthropic": "sk-ant-stored",
            "typesafe": "typesafe-stored",
            format!("proxy:{proxy_id}"): "s3cret-proxy",
        })
    );
}

#[tokio::test]
async fn starting_the_app_and_fetching_settings_never_touches_the_keychain() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    an_earlier_run_stored_everything(&store, &keychain).await;

    let app = launch(CaptureBehavior::NoBrowser, &store, &keychain);
    app.scans.recover_interrupted().await.unwrap();
    for _ in 0..20 {
        let settings = app.settings.get().await.unwrap();
        assert!(settings.has_api_key && settings.has_typesafe_key && settings.jev_enabled);
        assert!(settings.proxies[0].has_password);
    }
    // What a session without a scan also does: reports, the browser, lists.
    app.scans.detect_browser().await.unwrap();
    app.scans.list(all_scans()).await.unwrap();
    app.configurations.list().await.unwrap();

    assert_eq!(keychain.accesses(), (0, 0, 0));
}

#[tokio::test]
async fn the_browserbase_key_and_its_project_are_one_write_of_the_same_item() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    an_earlier_run_stored_everything(&store, &keychain).await;

    let app = launch(CaptureBehavior::NoBrowser, &store, &keychain);
    let status = app
        .browserbase
        .set_credentials(BROWSERBASE_KEY, None)
        .await
        .unwrap();
    assert!(status.valid);
    // One read to unlock, one write for both secrets: one prompt at most.
    assert_eq!(keychain.accesses(), (1, 1, 0));
    assert_eq!(keychain.accounts(), [VAULT]);
    let vault = vault_of(&keychain);
    assert_eq!(vault["browserbase"], BROWSERBASE_KEY);
    assert_eq!(vault["browserbase-project"], "project-1");
    assert_eq!(vault["anthropic"], "sk-ant-stored", "the rest is kept");

    // What the settings show comes from the record, not from the keychain.
    keychain.forget_accesses();
    let restarted = launch(CaptureBehavior::NoBrowser, &store, &keychain);
    let settings = restarted.settings.get().await.unwrap();
    assert!(settings.browserbase.connected);
    assert_eq!(settings.browserbase.project_name.as_deref(), Some("Local"));
    assert_eq!(keychain.accesses(), (0, 0, 0));

    restarted.browserbase.clear_credentials().await.unwrap();
    assert_eq!(keychain.accesses(), (1, 1, 0));
    let vault = vault_of(&keychain);
    assert!(vault.get("browserbase").is_none() && vault.get("browserbase-project").is_none());
    assert!(
        !restarted
            .settings
            .get()
            .await
            .unwrap()
            .browserbase
            .connected
    );
}

#[tokio::test]
async fn the_start_up_sweep_reads_the_keychain_only_for_a_session_left_open() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    an_earlier_run_stored_everything(&store, &keychain).await;
    launch(CaptureBehavior::NoBrowser, &store, &keychain)
        .browserbase
        .set_credentials(BROWSERBASE_KEY, None)
        .await
        .unwrap();
    keychain.forget_accesses();

    // A clean start: nothing on record, nothing read.
    let clean = launch(CaptureBehavior::NoBrowser, &store, &keychain);
    assert_eq!(
        clean
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(0)
    );
    assert_eq!(keychain.accesses(), (0, 0, 0));
    assert!(clean.browserbase_api.calls().is_empty());

    // A start after a run that left a browser behind.
    use adscan_engine::application::ports::BrowserbaseLedger;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1_000.0;
    store.session_created("left-open", now).await.unwrap();
    let after_a_crash = launch(CaptureBehavior::NoBrowser, &store, &keychain);
    *after_a_crash.browserbase_api.running.lock().unwrap() = vec!["left-open".to_string()];
    assert_eq!(
        after_a_crash
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(1)
    );
    assert_eq!(keychain.accesses(), (1, 0, 0));
}

#[tokio::test]
async fn the_first_scan_that_needs_every_secret_reads_the_keychain_once() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let proxy_id = an_earlier_run_stored_everything(&store, &keychain).await;

    let app = launch(
        CaptureBehavior::Fail(CaptureError::Navigation("unreachable".into())),
        &store,
        &keychain,
    );
    let first = run_scan(&app, ai_scan_through(&proxy_id)).await;
    assert_eq!(first.summary.status, ScanStatus::Failed);
    assert_eq!(
        keychain.accesses(),
        (1, 0, 0),
        "AI key, Jev key and proxy password come from one read"
    );
    assert_eq!(
        *app.capture.proxy_passwords.lock().unwrap(),
        [Some("s3cret-proxy".to_string())]
    );

    // From here on the process holds them: more scans, the settings and a
    // direct look at each secret read nothing.
    run_scan(&app, ai_scan_through(&proxy_id)).await;
    app.settings.get().await.unwrap();
    assert_eq!(
        app.settings
            .api_key(ApiProvider::Anthropic)
            .await
            .unwrap()
            .as_deref(),
        Some("sk-ant-stored")
    );
    assert_eq!(
        app.settings
            .api_key(ApiProvider::Typesafe)
            .await
            .unwrap()
            .as_deref(),
        Some("typesafe-stored")
    );
    assert_eq!(keychain.accesses(), (1, 0, 0));
}

#[tokio::test]
async fn a_scan_that_needs_no_secret_does_not_touch_the_keychain() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    an_earlier_run_stored_everything(&store, &keychain).await;

    let app = launch(
        CaptureBehavior::Fail(CaptureError::Navigation("unreachable".into())),
        &store,
        &keychain,
    );
    run_scan(&app, scan_options(false)).await;
    assert_eq!(keychain.accesses(), (0, 0, 0));
}

#[tokio::test]
async fn an_ai_scan_without_any_stored_key_does_not_touch_the_keychain() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let app = launch(
        CaptureBehavior::Fail(CaptureError::Navigation("unreachable".into())),
        &store,
        &keychain,
    );
    run_scan(&app, scan_options(true)).await;
    assert_eq!(keychain.accesses(), (0, 0, 0));
}

#[tokio::test]
async fn storing_a_key_is_one_read_when_still_locked_and_one_write() {
    let accepting = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&accepting)
        .await;
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    an_earlier_run_stored_everything(&store, &keychain).await;

    let app = launch_against(
        CaptureBehavior::NoBrowser,
        &store,
        &keychain,
        &accepting.uri(),
        &accepting.uri(),
    );
    let status = app
        .settings
        .set_api_key(ApiProvider::Anthropic, "sk-ant-new")
        .await
        .unwrap();
    assert!(status.valid);
    assert_eq!(keychain.accesses(), (1, 1, 0));

    let status = app
        .settings
        .set_api_key(ApiProvider::Typesafe, "typesafe-new")
        .await
        .unwrap();
    assert!(status.valid);
    assert_eq!(keychain.accesses(), (1, 2, 0), "already unlocked: no read");

    let vault = vault_of(&keychain);
    assert_eq!(vault["anthropic"], "sk-ant-new");
    assert_eq!(vault["typesafe"], "typesafe-new");
    assert_eq!(keychain.accounts(), [VAULT]);
}

#[tokio::test]
async fn the_very_first_key_creates_the_vault() {
    let accepting = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&accepting)
        .await;
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let app = launch_against(
        CaptureBehavior::NoBrowser,
        &store,
        &keychain,
        &accepting.uri(),
        OFFLINE,
    );
    assert!(!app.settings.get().await.unwrap().has_api_key);

    app.settings
        .set_api_key(ApiProvider::Anthropic, "sk-ant-first")
        .await
        .unwrap();

    // With no vault yet, the two key items of earlier versions are looked
    // for as well. None of the three exists, so macOS has nothing to ask.
    assert_eq!(keychain.accesses(), (3, 1, 0));
    assert_eq!(vault_of(&keychain), json!({ "anthropic": "sk-ant-first" }));
    assert!(app.settings.get().await.unwrap().has_api_key);
}

#[tokio::test]
async fn clearing_secrets_rewrites_the_vault_and_the_record() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let proxy_id = an_earlier_run_stored_everything(&store, &keychain).await;
    let app = launch(CaptureBehavior::NoBrowser, &store, &keychain);

    // Nothing is recorded for this one, so there is nothing to unlock for.
    app.secrets.clear_proxy_password("unknown").await.unwrap();
    assert_eq!(keychain.accesses(), (0, 0, 0));

    app.settings
        .clear_api_key(ApiProvider::Typesafe)
        .await
        .unwrap();
    assert_eq!(keychain.accesses(), (1, 1, 0));
    let settings = app.settings.delete_proxy_profile(&proxy_id).await.unwrap();
    assert_eq!(keychain.accesses(), (1, 2, 0));

    assert!(settings.has_api_key);
    assert!(!settings.has_typesafe_key && !settings.jev_enabled);
    assert_eq!(vault_of(&keychain), json!({ "anthropic": "sk-ant-stored" }));

    // The last secret leaves an empty vault behind, not a missing one.
    app.settings
        .clear_api_key(ApiProvider::Anthropic)
        .await
        .unwrap();
    assert_eq!(vault_of(&keychain), json!({}));
    assert!(store.load_presence().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_separate_items_of_earlier_versions_move_into_the_vault_on_first_need() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    keychain.put("anthropic-api-key", "sk-ant-legacy");
    keychain.put("typesafe-api-key", "typesafe-legacy");
    keychain.put("proxy-password:p1", "proxy-legacy");
    // What the database migration records for an earlier database with one
    // AI scan and two proxy profiles; the TypeSafe key it could not know of.
    let guessed = HashSet::from([anthropic(), proxy("p1"), proxy("p2")]);
    store.save_presence(&guessed).await.unwrap();

    let app = launch(CaptureBehavior::NoBrowser, &store, &keychain);
    assert!(app.settings.get().await.unwrap().has_api_key);
    assert_eq!(
        keychain.accesses(),
        (0, 0, 0),
        "not before a secret is needed"
    );

    assert_eq!(
        app.settings
            .api_key(ApiProvider::Anthropic)
            .await
            .unwrap()
            .as_deref(),
        Some("sk-ant-legacy")
    );
    // The vault, both key items and one item per recorded proxy password.
    assert_eq!(keychain.accesses(), (5, 1, 3));
    assert_eq!(keychain.accounts(), [VAULT]);
    assert_eq!(
        vault_of(&keychain),
        json!({
            "anthropic": "sk-ant-legacy",
            "typesafe": "typesafe-legacy",
            "proxy:p1": "proxy-legacy",
        })
    );
    // The vault wins over the guesses.
    assert_eq!(
        store.load_presence().await.unwrap(),
        HashSet::from([anthropic(), typesafe(), proxy("p1")])
    );
    assert!(app.settings.get().await.unwrap().has_typesafe_key);

    // The next launch finds the vault and nothing else to look for.
    keychain.forget_accesses();
    let restarted = launch(CaptureBehavior::NoBrowser, &store, &keychain);
    assert_eq!(
        restarted
            .secrets
            .proxy_password("p1")
            .await
            .unwrap()
            .as_deref(),
        Some("proxy-legacy")
    );
    assert_eq!(keychain.accesses(), (1, 0, 0));
}

#[tokio::test]
async fn no_secret_is_lost_when_the_vault_cannot_be_written_during_the_move() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    keychain.put("anthropic-api-key", "sk-ant-legacy");
    keychain.put("proxy-password:p1", "proxy-legacy");
    let guessed = HashSet::from([anthropic(), proxy("p1")]);
    store.save_presence(&guessed).await.unwrap();
    let app = launch(CaptureBehavior::NoBrowser, &store, &keychain);

    keychain.fail_writes(Some(SecretError::Failed("disk full".into())));
    assert_eq!(
        app.secrets.api_key(ApiProvider::Anthropic).await,
        Err(SecretError::Failed("disk full".into()))
    );
    assert_eq!(
        keychain.accounts(),
        ["anthropic-api-key", "proxy-password:p1"],
        "the old items stay until the vault holds their secrets"
    );
    assert_eq!(keychain.accesses().2, 0);
    assert_eq!(store.load_presence().await.unwrap(), guessed);

    keychain.fail_writes(None);
    assert_eq!(
        app.secrets
            .api_key(ApiProvider::Anthropic)
            .await
            .unwrap()
            .as_deref(),
        Some("sk-ant-legacy")
    );
    assert_eq!(keychain.accounts(), [VAULT]);
}

#[tokio::test]
async fn a_record_that_disagrees_with_the_vault_is_corrected_on_the_first_read() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    keychain.put(VAULT, r#"{"typesafe": "typesafe-stored"}"#);
    store
        .save_presence(&HashSet::from([anthropic()]))
        .await
        .unwrap();
    let app = launch(
        CaptureBehavior::Fail(CaptureError::Navigation("unreachable".into())),
        &store,
        &keychain,
    );
    let before = app.settings.get().await.unwrap();
    assert!(before.has_api_key && !before.has_typesafe_key);

    // The record promises an Anthropic key, so the AI scan asks for it.
    run_scan(&app, scan_options(true)).await;

    assert_eq!(keychain.accesses(), (1, 0, 0));
    let after = app.settings.get().await.unwrap();
    assert!(!after.has_api_key && after.has_typesafe_key);
}

#[tokio::test]
async fn a_denied_keychain_is_reported_as_denied_and_changes_no_record() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let proxy_id = an_earlier_run_stored_everything(&store, &keychain).await;
    let recorded = store.load_presence().await.unwrap();
    let app = launch(
        CaptureBehavior::Fail(CaptureError::Navigation("unreachable".into())),
        &store,
        &keychain,
    );

    keychain.fail_reads(Some(SecretError::Denied));
    assert_eq!(
        app.scans.start(ai_scan_through(&proxy_id)).await.err(),
        Some(denied()),
        "not a scan that quietly runs without AI"
    );
    assert!(app.scans.list(all_scans()).await.unwrap().items.is_empty());
    assert_eq!(
        app.settings.api_key(ApiProvider::Anthropic).await,
        Err(denied())
    );
    assert_eq!(
        app.settings
            .set_proxy_password(&proxy_id, "another")
            .await
            .err(),
        Some(denied())
    );
    assert_eq!(
        app.settings.clear_api_key(ApiProvider::Anthropic).await,
        Err(denied())
    );

    // The settings still say what is stored, without asking again.
    let reads_so_far = keychain.accesses().0;
    let settings = app.settings.get().await.unwrap();
    assert!(settings.has_api_key && settings.has_typesafe_key && settings.jev_enabled);
    assert!(settings.proxies[0].has_password);
    assert_eq!(store.load_presence().await.unwrap(), recorded);
    assert_eq!(keychain.accesses(), (reads_so_far, 0, 0));

    // A refusal is not remembered: the next need asks again.
    keychain.fail_reads(None);
    let detail = run_scan(&app, ai_scan_through(&proxy_id)).await;
    assert_eq!(detail.summary.status, ScanStatus::Failed);
    assert_eq!(
        *app.capture.proxy_passwords.lock().unwrap(),
        [Some("s3cret-proxy".to_string())]
    );
}

#[tokio::test]
async fn an_unavailable_keychain_is_reported_as_unavailable() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    an_earlier_run_stored_everything(&store, &keychain).await;
    let app = launch(CaptureBehavior::NoBrowser, &store, &keychain);

    keychain.fail_reads(Some(SecretError::Unavailable("it is locked".into())));
    assert_eq!(
        app.scans.start(scan_options(true)).await.err(),
        Some(AppError::Internal(
            "The keychain is unavailable: it is locked".into()
        ))
    );
    assert!(app.settings.get().await.unwrap().has_api_key);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tasks_that_need_secrets_at_the_same_moment_cause_one_read() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let proxy_id = an_earlier_run_stored_everything(&store, &keychain).await;
    *keychain.read_time.lock().unwrap() = Duration::from_millis(150);
    let app = launch(CaptureBehavior::NoBrowser, &store, &keychain);

    let mut tasks = Vec::new();
    for index in 0..12 {
        let secrets = Arc::clone(&app.secrets);
        let proxy_id = proxy_id.clone();
        tasks.push(tokio::spawn(async move {
            match index % 3 {
                0 => secrets.api_key(ApiProvider::Anthropic).await,
                1 => secrets.api_key(ApiProvider::Typesafe).await,
                _ => secrets.proxy_password(&proxy_id).await,
            }
        }));
    }
    for task in tasks {
        assert!(task.await.unwrap().unwrap().is_some());
    }
    assert_eq!(keychain.accesses(), (1, 0, 0));
}

#[tokio::test]
async fn a_vault_that_cannot_be_read_as_one_is_reported_and_never_overwritten() {
    let store = SqliteStore::open_in_memory().unwrap();
    let keychain = Arc::new(CountingKeychain::default());
    let proxy_id = an_earlier_run_stored_everything(&store, &keychain).await;
    let recorded = store.load_presence().await.unwrap();
    // Something else wrote to the item, or it was cut short: the secrets
    // may still be recoverable from it by hand.
    const DAMAGED: &str = r#"{"anthropic":"sk-ant-still-in-here","typesafe":"#;
    keychain.put(VAULT, DAMAGED);
    let app = launch(
        CaptureBehavior::Fail(CaptureError::Navigation("unreachable".into())),
        &store,
        &keychain,
    );

    let reading = app.settings.api_key(ApiProvider::Anthropic).await;
    let storing = app
        .settings
        .set_proxy_password(&proxy_id, "another")
        .await
        .map(drop);
    let clearing = app.settings.clear_api_key(ApiProvider::Typesafe).await;
    for failed in [reading.map(drop), storing, clearing] {
        let message = failed
            .expect_err("a damaged vault must not read as empty")
            .to_string();
        assert!(message.contains("not in the expected format"), "{message}");
        assert!(!message.contains("sk-ant-still-in-here"), "{message}");
    }
    assert_eq!(
        app.scans
            .start(ai_scan_through(&proxy_id))
            .await
            .err()
            .map(|error| error.to_string()),
        app.settings
            .api_key(ApiProvider::Anthropic)
            .await
            .err()
            .map(|error| error.to_string()),
        "a scan that needs a key says the same, instead of running without AI"
    );

    // Nothing was written, deleted or forgotten.
    assert_eq!(keychain.item(VAULT).as_deref(), Some(DAMAGED));
    let (_, writes, deletes) = keychain.accesses();
    assert_eq!((writes, deletes), (0, 0));
    assert_eq!(store.load_presence().await.unwrap(), recorded);
    let settings = app.settings.get().await.unwrap();
    assert!(settings.has_api_key && settings.proxies[0].has_password);
}
