//! SQLite persistence. One connection in WAL mode, used only from blocking
//! threads so the async runtime's workers never wait on disk.

mod archive;
mod configurations;
mod logs;
mod scans;
mod secret_presence;
mod settings;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::application::ports::{RepoResult, RepositoryError};

const DATABASE_FILE: &str = "adtech-scanner.sqlite3";

/// Each entry migrates the schema from `user_version = index` to `index + 1`.
const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE scans (
        id TEXT PRIMARY KEY,
        url TEXT NOT NULL,
        hostname TEXT NOT NULL,
        device TEXT NOT NULL,
        status TEXT NOT NULL,
        started_at REAL NOT NULL,
        finished_at REAL,
        duration_ms REAL,
        cost_usd REAL NOT NULL DEFAULT 0,
        critical_count INTEGER NOT NULL DEFAULT 0,
        high_count INTEGER NOT NULL DEFAULT 0,
        medium_count INTEGER NOT NULL DEFAULT 0,
        low_count INTEGER NOT NULL DEFAULT 0,
        info_count INTEGER NOT NULL DEFAULT 0,
        health_score INTEGER,
        prebid_version TEXT,
        ai_used INTEGER NOT NULL DEFAULT 0,
        error TEXT,
        options TEXT NOT NULL,
        metrics TEXT,
        overview TEXT
    );
    CREATE INDEX scans_by_start ON scans (started_at DESC);

    CREATE TABLE findings (
        id TEXT PRIMARY KEY,
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX findings_by_scan ON findings (scan_id, position);

    CREATE TABLE auctions (
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL,
        PRIMARY KEY (scan_id, position)
    );
    CREATE TABLE slots (
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL,
        PRIMARY KEY (scan_id, position)
    );
    CREATE TABLE scripts (
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL,
        PRIMARY KEY (scan_id, position)
    );

    CREATE TABLE steps (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        data TEXT NOT NULL
    );
    CREATE INDEX steps_by_scan ON steps (scan_id, seq);

    CREATE TABLE logs (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        step_id TEXT NOT NULL,
        node TEXT NOT NULL,
        ts REAL NOT NULL,
        level TEXT NOT NULL,
        kind TEXT NOT NULL,
        message TEXT NOT NULL,
        data TEXT
    );
    CREATE INDEX logs_by_scan ON logs (scan_id, seq);

    CREATE VIRTUAL TABLE logs_fts USING fts5 (message, data_text, tokenize = 'unicode61');
    CREATE TRIGGER logs_fts_delete AFTER DELETE ON logs BEGIN
        DELETE FROM logs_fts WHERE rowid = old.seq;
    END;

    CREATE TABLE settings (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        data TEXT NOT NULL
    );
    ",
    // Settings are one JSON document and every field is required on load, so
    // fields added to the contract are written into the stored document here.
    "
    UPDATE settings
    SET data = json_set(data, '$.jevEnabled', json('false'), '$.hasTypesafeKey', json('false'));
    ",
    "
    ALTER TABLE scans ADD COLUMN diagnostics TEXT;
    UPDATE settings SET data = json_set(data, '$.checkSellersJson', json('false'));
    ",
    // Page views and the visit's environment per scan, saved configurations,
    // and the settings that came with visitor behaviours and proxies.
    "
    ALTER TABLE scans ADD COLUMN pages TEXT;
    ALTER TABLE scans ADD COLUMN environment TEXT;
    ALTER TABLE scans ADD COLUMN configuration_id TEXT;
    ALTER TABLE scans ADD COLUMN configuration_name TEXT;
    ALTER TABLE scans ADD COLUMN proxy TEXT;
    CREATE INDEX scans_by_configuration ON scans (configuration_id, started_at DESC);

    CREATE TABLE scan_configurations (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        options TEXT NOT NULL,
        created_at REAL NOT NULL,
        updated_at REAL NOT NULL,
        last_scan_id TEXT,
        last_run_at REAL
    );

    UPDATE settings
    SET data = json_set(
        data,
        '$.defaultVisitor', 'scroll',
        '$.defaultExtraPages', 2,
        '$.persistentProfile', json('false'),
        '$.proxies', json('[]'),
        '$.defaultProxy', json('{\"kind\":\"system\"}')
    );
    ",
    // The scan archive: one bookkeeping row per scan, the byte range of every
    // kept body in the scan's pack file, and one row per request. A request
    // row has no query string, no header and no body: those are in the pack.
    // `scan_no` keeps a 36-character scan id out of every row and index.
    "
    CREATE TABLE scan_archives (
        scan_no INTEGER PRIMARY KEY,
        scan_id TEXT NOT NULL UNIQUE REFERENCES scans (id) ON DELETE CASCADE,
        state TEXT NOT NULL,
        format INTEGER NOT NULL,
        pack_file TEXT,
        pack_bytes INTEGER NOT NULL DEFAULT 0,
        request_count INTEGER NOT NULL DEFAULT 0,
        body_count INTEGER NOT NULL DEFAULT 0,
        headers_mode TEXT NOT NULL,
        truncated INTEGER NOT NULL DEFAULT 0,
        created_at REAL NOT NULL
    );
    CREATE INDEX scan_archives_by_age ON scan_archives (state, created_at);

    CREATE TABLE hosts (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        site TEXT NOT NULL
    );
    CREATE INDEX hosts_by_site ON hosts (site);

    CREATE TABLE scan_bodies (
        scan_no INTEGER NOT NULL REFERENCES scan_archives (scan_no) ON DELETE CASCADE,
        body_no INTEGER NOT NULL,
        sha256 BLOB NOT NULL,
        kind INTEGER NOT NULL,
        raw_len INTEGER NOT NULL,
        pack_off INTEGER NOT NULL,
        pack_len INTEGER NOT NULL,
        truncated INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (scan_no, body_no)
    ) WITHOUT ROWID;
    CREATE UNIQUE INDEX scan_bodies_by_sha ON scan_bodies (sha256, scan_no);

    CREATE TABLE requests (
        scan_no INTEGER NOT NULL REFERENCES scan_archives (scan_no) ON DELETE CASCADE,
        seq INTEGER NOT NULL,
        page INTEGER NOT NULL,
        context INTEGER NOT NULL,
        started_ms REAL NOT NULL,
        ttfb_ms REAL,
        duration_ms REAL,
        method INTEGER NOT NULL,
        scheme INTEGER NOT NULL,
        host_id INTEGER NOT NULL REFERENCES hosts (id),
        path TEXT NOT NULL,
        query_len INTEGER NOT NULL,
        resource_type INTEGER NOT NULL,
        mime TEXT,
        status INTEGER,
        outcome INTEGER NOT NULL,
        error TEXT,
        flags INTEGER NOT NULL DEFAULT 0,
        served_from INTEGER NOT NULL,
        encoded_bytes INTEGER NOT NULL,
        body_bytes INTEGER NOT NULL,
        post_bytes INTEGER NOT NULL,
        initiator_type INTEGER NOT NULL,
        redirect_from INTEGER,
        preflight_of INTEGER,
        bidder TEXT,
        category INTEGER NOT NULL,
        body_state INTEGER NOT NULL,
        body_no INTEGER,
        record_off INTEGER NOT NULL,
        record_len INTEGER NOT NULL,
        record_idx INTEGER NOT NULL,
        ad TEXT,
        PRIMARY KEY (scan_no, seq)
    ) WITHOUT ROWID;
    CREATE INDEX requests_by_host ON requests (host_id, scan_no);

    UPDATE settings
    SET data = json_set(
        data,
        '$.keepNetworkData', json('true'),
        '$.archiveLimitBytes', 5368709120,
        '$.storeFullHeaders', json('false')
    );
    ",
    secret_presence::MIGRATION,
];

/// Implements the scan, configuration, log and settings repositories on one
/// database.
#[derive(Clone)]
pub struct SqliteStore {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Opens (creating if needed) the database in `data_dir`.
    pub fn open(data_dir: &Path) -> RepoResult<Self> {
        std::fs::create_dir_all(data_dir).map_err(|error| {
            RepositoryError(format!("cannot create {}: {error}", data_dir.display()))
        })?;
        Self::prepare(Connection::open(data_dir.join(DATABASE_FILE)).map_err(storage_error)?)
    }

    pub fn open_in_memory() -> RepoResult<Self> {
        Self::prepare(Connection::open_in_memory().map_err(storage_error)?)
    }

    fn prepare(mut connection: Connection) -> RepoResult<Self> {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .and_then(|()| connection.pragma_update(None, "synchronous", "NORMAL"))
            .and_then(|()| connection.pragma_update(None, "foreign_keys", "ON"))
            // What the user deletes is overwritten in the file, not only
            // unlinked: a deleted scan's rows must not be readable in it.
            .and_then(|()| connection.pragma_update(None, "secure_delete", "ON"))
            .and_then(|()| connection.busy_timeout(std::time::Duration::from_secs(5)))
            .and_then(|()| migrate(&mut connection))
            .map_err(storage_error)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    async fn call<T, F>(&self, work: F) -> RepoResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let mut guard = connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            work(&mut guard)
        })
        .await
        .map_err(|error| RepositoryError(format!("storage task failed: {error}")))?
        .map_err(storage_error)
    }
}

fn migrate(connection: &mut Connection) -> rusqlite::Result<()> {
    let current: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (version, migration) in (1u32..).zip(MIGRATIONS).skip(current as usize) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }
    Ok(())
}

fn storage_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError(error.to_string())
}

fn to_json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn from_json<T: DeserializeOwned>(text: &str) -> rusqlite::Result<T> {
    serde_json::from_str(text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

/// Contract enums are stored by their JSON name (`"completed"`, `"mobile"`).
fn enum_text<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        Ok(other) => Err(rusqlite::Error::ToSqlConversionFailure(
            format!("expected a unit enum, got {other}").into(),
        )),
        Err(error) => Err(rusqlite::Error::ToSqlConversionFailure(Box::new(error))),
    }
}

fn enum_from_text<T: DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schema_version_two_database_gains_the_diagnostics_column_and_setting() {
        const V2_SETTINGS: &str = r#"{"filterModel":"claude-haiku-5-5","analysisModel":"claude-sonnet-5-5","analysisEffort":"medium","budgetUsd":0.5,"chromePath":null,"headless":true,"defaultDevice":"desktop","defaultObserveSeconds":20,"theme":"system","hasApiKey":false,"jevEnabled":true,"hasTypesafeKey":false}"#;
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATIONS[0]).unwrap();
        connection.execute_batch(MIGRATIONS[1]).unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        connection
            .execute(
                "INSERT INTO settings (id, data) VALUES (1, ?1)",
                [V2_SETTINGS],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO scans (id, url, hostname, device, status, started_at, options)
                 VALUES ('old', 'https://a.test/', 'a.test', 'desktop', 'completed', 1, '{}')",
                [],
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        let stored: String = connection
            .query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let settings: adscan_model::Settings = from_json(&stored).unwrap();
        assert!(settings.jev_enabled, "existing choices are kept");
        assert!(!settings.check_sellers_json, "the new lookup starts off");
        let diagnostics: Option<String> = connection
            .query_row(
                "SELECT diagnostics FROM scans WHERE id = 'old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(diagnostics, None);
    }

    #[test]
    fn migrations_set_user_version_and_are_idempotent_across_reopens() {
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            let store = SqliteStore::open(dir.path()).unwrap();
            let connection = store.connection.lock().unwrap();
            let version: u32 = connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version as usize, MIGRATIONS.len());
            let journal: String = connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert_eq!(journal, "wal");
            let foreign_keys: i64 = connection
                .pragma_query_value(None, "foreign_keys", |row| row.get(0))
                .unwrap();
            assert_eq!(foreign_keys, 1);
        }
    }

    #[test]
    fn a_database_from_an_older_schema_version_is_upgraded() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 0).unwrap();
        migrate(&mut connection).unwrap();
        let tables: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('scans', 'findings', 'logs', 'logs_fts', 'settings')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 5);
    }

    #[test]
    fn settings_saved_by_schema_version_one_load_after_the_upgrade() {
        const V1_SETTINGS: &str = r#"{"filterModel":"claude-haiku-5-5","analysisModel":"claude-sonnet-5-5","analysisEffort":"high","budgetUsd":1.25,"chromePath":null,"headless":false,"defaultDevice":"mobile","defaultObserveSeconds":30,"theme":"dark","hasApiKey":false}"#;
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATIONS[0]).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute(
                "INSERT INTO settings (id, data) VALUES (1, ?1)",
                [V1_SETTINGS],
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let stored: String = connection
            .query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let settings: adscan_model::Settings = from_json(&stored).unwrap();
        assert_eq!(
            settings,
            adscan_model::Settings {
                analysis_effort: adscan_model::Effort::High,
                budget_usd: 1.25,
                headless: false,
                default_device: adscan_model::DeviceProfile::Mobile,
                default_observe_seconds: 30,
                theme: adscan_model::ThemePreference::Dark,
                ..adscan_model::Settings::default()
            }
        );
    }

    /// A database as version 3 of the app left it: one finished scan with an
    /// auction, a slot, a script and a finding, and saved settings.
    fn version_3_database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        for migration in &MIGRATIONS[..3] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 3).unwrap();
        connection
            .execute_batch(
                r#"
                INSERT INTO scans (id, url, hostname, device, status, started_at, finished_at, options, metrics)
                VALUES ('old-scan', 'https://pub.example/', 'pub.example', 'desktop', 'completed', 1000, 2000,
                    '{"url":"https://pub.example/","device":"desktop","observeSeconds":20,"autoScroll":true,"acceptConsent":true,"aiEnabled":false}',
                    '{"pageLoadMs":950,"auctionCount":1,"adUnitCount":1,"bidderCount":1,"bidRequestCount":1,"bidResponseCount":1,"bidTimeoutCount":0,"noBidCount":0,"timeoutRate":0,"slotCount":1,"emptySlotCount":0,"fillRate":1,"avgWinningCpm":null,"totalWinningCpm":0,"timeToFirstAuctionMs":360,"timeToFirstAdMs":900,"scriptCount":1,"adScriptCount":1,"totalScriptBytes":10,"prebidTimeoutMs":1500,"hasPrebid":true,"hasGpt":true,"hasAmazonTam":false}');
                INSERT INTO auctions (scan_id, position, data) VALUES ('old-scan', 0,
                    '{"id":"a1","startedAtMs":360,"endedAtMs":700,"durationMs":340,"timeoutMs":1500,"adUnits":[]}');
                INSERT INTO slots (scan_id, position, data) VALUES ('old-scan', 0,
                    '{"elementId":"top","adUnitPath":"/123/top","sizes":["728x90"],"outcome":"filled","renderedSize":"728x90","definedAtMs":200,"requestedAtMs":710,"responseAtMs":880,"renderedAtMs":900,"viewableAtMs":null,"inInitialViewport":true,"hadPrebidTargeting":true,"requestedBeforeAuctionEnd":false,"advertiserId":null,"lineItemId":null}');
                INSERT INTO scripts (scan_id, position, data) VALUES ('old-scan', 0,
                    '{"id":"s1","url":"https://pub.example/ads.js","hostname":"pub.example","inline":false,"sizeBytes":10,"hash":"abc","category":"wrapper","adRelated":true,"classificationReason":null,"classificationConfidence":null,"classifiedBy":"heuristic","classifierModel":null,"adProbability":null,"analyzed":false,"vendor":null,"loadedAtMs":100,"firstParty":true}');
                INSERT INTO findings (id, scan_id, position, data) VALUES ('f1', 'old-scan', 0,
                    '{"id":"f1","scanId":"old-scan","severity":"high","category":"lazyLoad","source":"heuristic","title":"t","description":"d","recommendation":"r","confidence":0.7,"impact":"i","estimatedRevenueImpactPct":null,"evidence":[],"affectedSlots":["top"],"affectedBidders":[]}');
                INSERT INTO settings (id, data) VALUES (1,
                    '{"filterModel":"claude-haiku-5-5","analysisModel":"claude-sonnet-5-5","analysisEffort":"high","budgetUsd":0.25,"chromePath":"/opt/chrome","headless":false,"defaultDevice":"mobile","defaultObserveSeconds":30,"theme":"dark","hasApiKey":false,"jevEnabled":false,"hasTypesafeKey":false,"checkSellersJson":true}');
                "#,
            )
            .unwrap();
        connection
    }

    #[tokio::test]
    async fn a_schema_version_three_database_opens_with_its_scans_and_settings_intact() {
        use adscan_model::{ProxyChoice, ScanListQuery, ScanStatus, VisitorBehavior};

        use crate::application::ports::{
            ScanConfigurationRepository, ScanRepository, SettingsRepository,
        };
        use crate::domain::scan::ValidatedScan;

        let store = SqliteStore::prepare(version_3_database()).unwrap();
        let version: u32 = store
            .call(|connection| {
                connection.pragma_query_value(None, "user_version", |row| row.get(0))
            })
            .await
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());

        // The old scan loads: one implicit page view, nothing recorded about
        // the environment, the proxy or a configuration.
        let detail = ScanRepository::detail(&store, "old-scan")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.summary.status, ScanStatus::Completed);
        assert_eq!(detail.summary.configuration, None);
        assert_eq!(detail.summary.proxy, None);
        assert!(detail.pages.is_empty());
        assert_eq!(detail.environment, None);
        assert_eq!(detail.metrics.auction_count, 1);
        assert_eq!(
            (detail.auctions[0].page, detail.auctions[0].started_at_ms),
            (0, 360.0)
        );
        assert_eq!(detail.slots[0].page, 0);
        assert!(detail.slots[0].requests.is_empty());
        assert!(detail.scripts[0].pages.is_empty());
        assert!(detail.findings[0].pages.is_empty());

        // Its options predate visitors: repeated, they are a scroll visit.
        assert_eq!(detail.options.visitor, None);
        let settings = SettingsRepository::load(&store).await.unwrap().unwrap();
        let repeated = ValidatedScan::new(detail.options.clone(), &settings, 9).unwrap();
        assert_eq!(repeated.visitor, VisitorBehavior::Scroll);

        // The settings keep what the user chose and gain the new defaults.
        assert_eq!(settings.budget_usd, 0.25);
        assert_eq!(settings.chrome_path.as_deref(), Some("/opt/chrome"));
        assert!(settings.check_sellers_json && !settings.headless);
        assert_eq!(settings.default_visitor, VisitorBehavior::Scroll);
        assert_eq!(settings.default_extra_pages, 2);
        assert!(!settings.persistent_profile);
        assert!(settings.proxies.is_empty());
        assert_eq!(settings.default_proxy, ProxyChoice::System);

        // The new table is there and empty, and the old scan is in no
        // configuration's history.
        assert!(
            ScanConfigurationRepository::list(&store)
                .await
                .unwrap()
                .is_empty()
        );
        let of_configuration = ScanRepository::list(
            &store,
            &ScanListQuery {
                search: None,
                status: None,
                device: None,
                limit: None,
                offset: None,
                configuration_id: Some("any".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(of_configuration.total, 0);
    }

    #[tokio::test]
    async fn a_schema_version_four_database_gains_the_archive_and_its_scans_read_as_not_recorded() {
        use crate::application::ports::{ArchiveRepository, ScanRepository, SettingsRepository};

        let connection = Connection::open_in_memory().unwrap();
        for migration in &MIGRATIONS[..4] {
            connection.execute_batch(migration).unwrap();
        }
        connection.pragma_update(None, "user_version", 4).unwrap();
        connection
            .execute_batch(
                r#"
                INSERT INTO scans (id, url, hostname, device, status, started_at, finished_at, options, pages)
                VALUES ('v4-scan', 'https://pub.example/', 'pub.example', 'desktop', 'completed', 1000, 2000,
                    '{"url":"https://pub.example/","device":"desktop","observeSeconds":20,"autoScroll":true,"acceptConsent":true,"aiEnabled":false,"visitor":"scroll"}',
                    '[]');
                INSERT INTO scripts (scan_id, position, data) VALUES ('v4-scan', 0,
                    '{"id":"s1","url":"https://pub.example/ads.js","hostname":"pub.example","inline":false,"sizeBytes":10,"hash":"abc","category":"wrapper","adRelated":true,"classificationReason":null,"classificationConfidence":null,"classifiedBy":"heuristic","classifierModel":null,"adProbability":null,"analyzed":false,"vendor":null,"loadedAtMs":100,"firstParty":true,"pages":[0]}');
                INSERT INTO auctions (scan_id, position, data) VALUES ('v4-scan', 0,
                    '{"id":"a1","page":0,"startedAtMs":360,"endedAtMs":700,"durationMs":340,"timeoutMs":1500,"adUnits":[{"code":"top","sizes":[],"bids":[{"bidder":"rubicon","status":"bid","cpm":1.2,"currency":"USD","size":null,"requestedAtMs":365,"responseTimeMs":200,"dealId":null}],"winningBidder":null,"winningCpm":null}]}');
                INSERT INTO findings (id, scan_id, position, data) VALUES ('f1', 'v4-scan', 0,
                    '{"id":"f1","scanId":"v4-scan","severity":"high","category":"logicBug","source":"ai","title":"t","description":"d","recommendation":"r","confidence":0.7,"impact":"i","estimatedRevenueImpactPct":null,"evidence":[{"kind":"code","label":"l","body":"if (x) {","scriptUrl":"https://pub.example/ads.js","line":12}],"affectedSlots":[],"affectedBidders":[],"pages":[]}');
                INSERT INTO settings (id, data) VALUES (1,
                    '{"filterModel":"claude-haiku-5-5","analysisModel":"claude-sonnet-5-5","analysisEffort":"medium","budgetUsd":0.75,"chromePath":null,"headless":true,"defaultDevice":"desktop","defaultObserveSeconds":20,"theme":"system","hasApiKey":false,"jevEnabled":false,"hasTypesafeKey":false,"checkSellersJson":true,"defaultVisitor":"browse","defaultExtraPages":3,"persistentProfile":true,"proxies":[],"defaultProxy":{"kind":"none"}}');
                "#,
            )
            .unwrap();

        let store = SqliteStore::prepare(connection).unwrap();
        let version: u32 = store
            .call(|connection| {
                connection.pragma_query_value(None, "user_version", |row| row.get(0))
            })
            .await
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());

        // The settings keep what the user chose; storage starts on, with the
        // default limit and redacted headers.
        let settings = SettingsRepository::load(&store).await.unwrap().unwrap();
        assert_eq!(settings.budget_usd, 0.75);
        assert_eq!(settings.default_extra_pages, 3);
        assert!(settings.persistent_profile && settings.check_sellers_json);
        assert!(settings.keep_network_data);
        assert!(!settings.store_full_headers);
        assert_eq!(
            settings.archive_limit_bytes,
            adscan_model::DEFAULT_ARCHIVE_LIMIT_BYTES
        );

        // The old scan reads as before, with nothing where the links are.
        let detail = ScanRepository::detail(&store, "v4-scan")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.network, None);
        assert_eq!(detail.auctions[0].ad_units[0].bids[0].exchange, None);
        let script = &detail.scripts[0];
        assert_eq!(
            (&script.sha256, &script.request_id, &script.initiator),
            (&None, &None, &None)
        );
        let evidence = &detail.findings[0].evidence[0];
        assert_eq!(evidence.line, Some(12));
        assert_eq!(
            (
                &evidence.end_line,
                &evidence.script_id,
                &evidence.request_id
            ),
            (&None, &None, &None)
        );
        assert_eq!(
            ArchiveRepository::record(&store, "v4-scan").await.unwrap(),
            None
        );
        let (rows, total) = ArchiveRepository::rows(&store, "v4-scan", &[], 0, 10)
            .await
            .unwrap();
        assert!(rows.is_empty() && total == 0);
        let storage = ArchiveRepository::storage(&store, 1.0).await.unwrap();
        assert_eq!(
            (storage.scans, storage.evicted_scans, storage.stored_bytes),
            (0, 0, 0.0)
        );

        // The new tables work for it, and go with the scan.
        ArchiveRepository::begin(
            &store,
            "v4-scan",
            Some("scans/v4-scan.pack.zst"),
            true,
            1000.0,
        )
        .await
        .unwrap();
        assert!(ScanRepository::delete(&store, "v4-scan").await.unwrap());
        assert_eq!(
            ArchiveRepository::record(&store, "v4-scan").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn the_request_list_reads_by_primary_key_and_host_lookups_use_their_index() {
        let store = SqliteStore::open_in_memory().unwrap();
        let plan = |sql: &'static str| {
            let store = store.clone();
            async move {
                store
                    .call(move |connection| {
                        let mut statement =
                            connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                        let lines = statement
                            .query_map([], |row| row.get::<_, String>(3))?
                            .collect::<rusqlite::Result<Vec<_>>>()?;
                        Ok(lines.join("; "))
                    })
                    .await
                    .unwrap()
            }
        };
        let list = plan(
            "SELECT * FROM requests r WHERE r.scan_no = 1 ORDER BY r.page, r.started_ms, r.seq",
        )
        .await;
        assert!(
            list.contains("PRIMARY KEY") && !list.contains("SCAN r"),
            "{list}"
        );
        let by_host = plan("SELECT scan_no FROM requests WHERE host_id = 7").await;
        assert!(by_host.contains("requests_by_host"), "{by_host}");
        let by_content = plan("SELECT scan_no FROM scan_bodies WHERE sha256 = x'00'").await;
        assert!(by_content.contains("scan_bodies_by_sha"), "{by_content}");
    }
}
