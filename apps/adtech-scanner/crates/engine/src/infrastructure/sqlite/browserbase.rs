//! The app's own record of its Browserbase use. Identifiers only: the API
//! key and the project id are in the keychain, and no connect or live-view
//! URL is ever written anywhere.

/// Browserbase as a second place the browser can run: the usage of a scan,
/// this installation's id (sent as session metadata, so that the start-up
/// sweep only ever releases its own sessions), the sessions it started, the
/// contexts that hold a kept browser profile, and the new settings.
pub(super) const MIGRATION: &str = r#"
    ALTER TABLE scans ADD COLUMN remote TEXT;

    CREATE TABLE browserbase_install (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        install_id TEXT NOT NULL
    );
    INSERT INTO browserbase_install (id, install_id) VALUES (1, lower(hex(randomblob(16))));

    CREATE TABLE browserbase_sessions (
        session_id TEXT PRIMARY KEY,
        created_at REAL NOT NULL,
        released_at REAL
    ) WITHOUT ROWID;

    CREATE TABLE browserbase_contexts (
        device TEXT PRIMARY KEY,
        context_id TEXT NOT NULL,
        created_at REAL NOT NULL,
        last_used_at REAL
    ) WITHOUT ROWID;

    UPDATE settings
    SET data = json_set(
        data,
        '$.defaultBrowser', 'local',
        '$.browserbase', json('{"region":"us-west-2","defaultProxy":{"kind":"none"},"persistentProfile":false,"connected":false,"projectName":null,"concurrency":null}')
    );
    "#;

use adscan_model::DeviceProfile;
use async_trait::async_trait;
use rusqlite::params;

use super::{SqliteStore, enum_from_text, enum_text};
use crate::application::ports::{BrowserbaseLedger, OpenRemoteSession, RepoResult, StoredContext};

#[async_trait]
impl BrowserbaseLedger for SqliteStore {
    async fn install_id(&self) -> RepoResult<String> {
        self.call(|connection| {
            connection.query_row(
                "SELECT install_id FROM browserbase_install WHERE id = 1",
                [],
                |row| row.get(0),
            )
        })
        .await
    }

    async fn session_created(&self, session_id: &str, at_ms: f64) -> RepoResult<()> {
        let session_id = session_id.to_string();
        self.call(move |connection| {
            connection.execute(
                "INSERT OR IGNORE INTO browserbase_sessions (session_id, created_at) VALUES (?1, ?2)",
                params![session_id, at_ms],
            )?;
            Ok(())
        })
        .await
    }

    async fn session_released(&self, session_id: &str, at_ms: f64) -> RepoResult<()> {
        let session_id = session_id.to_string();
        self.call(move |connection| {
            connection.execute(
                "UPDATE browserbase_sessions SET released_at = ?2 WHERE session_id = ?1",
                params![session_id, at_ms],
            )?;
            Ok(())
        })
        .await
    }

    async fn open_sessions(&self) -> RepoResult<Vec<OpenRemoteSession>> {
        self.call(|connection| {
            connection
                .prepare(
                    "SELECT session_id, created_at FROM browserbase_sessions
                     WHERE released_at IS NULL ORDER BY created_at, session_id",
                )?
                .query_map([], |row| {
                    Ok(OpenRemoteSession {
                        session_id: row.get(0)?,
                        created_at: row.get(1)?,
                    })
                })?
                .collect()
        })
        .await
    }

    async fn forget_released_sessions(&self) -> RepoResult<()> {
        self.call(|connection| {
            connection.execute(
                "DELETE FROM browserbase_sessions WHERE released_at IS NOT NULL",
                [],
            )?;
            Ok(())
        })
        .await
    }

    async fn contexts(&self) -> RepoResult<Vec<StoredContext>> {
        self.call(|connection| {
            connection
                .prepare(
                    "SELECT device, context_id, last_used_at FROM browserbase_contexts ORDER BY device",
                )?
                .query_map([], |row| {
                    Ok(StoredContext {
                        device: enum_from_text(row.get(0)?)?,
                        context_id: row.get(1)?,
                        last_used_at: row.get(2)?,
                    })
                })?
                .collect()
        })
        .await
    }

    async fn save_context(
        &self,
        device: DeviceProfile,
        context_id: &str,
        at_ms: f64,
    ) -> RepoResult<()> {
        let context_id = context_id.to_string();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO browserbase_contexts (device, context_id, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (device) DO UPDATE
                 SET context_id = excluded.context_id, created_at = excluded.created_at, last_used_at = NULL",
                params![enum_text(&device)?, context_id, at_ms],
            )?;
            Ok(())
        })
        .await
    }

    async fn context_used(&self, device: DeviceProfile, at_ms: f64) -> RepoResult<()> {
        self.call(move |connection| {
            connection.execute(
                "UPDATE browserbase_contexts SET last_used_at = ?2 WHERE device = ?1",
                params![enum_text(&device)?, at_ms],
            )?;
            Ok(())
        })
        .await
    }

    async fn forget_contexts(&self) -> RepoResult<()> {
        self.call(|connection| {
            connection.execute("DELETE FROM browserbase_contexts", [])?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use adscan_model::{BrowserKind, BrowserbaseSettings, Settings};
    use rusqlite::Connection;

    use super::super::{MIGRATIONS, from_json, migrate};
    use super::*;

    /// A database as the version before this migration left it.
    fn database_before() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        let earlier = MIGRATIONS
            .iter()
            .position(|migration| *migration == MIGRATION)
            .unwrap();
        for migration in &MIGRATIONS[..earlier] {
            connection.execute_batch(migration).unwrap();
        }
        connection
            .pragma_update(None, "user_version", earlier as u32)
            .unwrap();
        connection
    }

    #[test]
    fn an_earlier_database_gains_the_browserbase_settings_and_keeps_the_rest() {
        let mut connection = database_before();
        // The settings document as the previous version wrote it: the
        // contract of today minus the two new fields.
        let mut before = serde_json::to_value(Settings {
            budget_usd: 0.25,
            persistent_profile: true,
            ..Settings::default()
        })
        .unwrap();
        let document = before.as_object_mut().unwrap();
        document.remove("defaultBrowser");
        document.remove("browserbase");
        connection
            .execute(
                "INSERT INTO settings (id, data) VALUES (1, ?1)",
                [before.to_string()],
            )
            .unwrap();
        connection
            .execute_batch(
                r#"
                INSERT INTO scans (id, url, hostname, device, status, started_at, options, environment)
                VALUES ('old', 'https://pub.example/', 'pub.example', 'desktop', 'completed', 1,
                    '{"url":"https://pub.example/","device":"desktop","observeSeconds":10,"autoScroll":true,"acceptConsent":true,"aiEnabled":false}',
                    '{"visitor":"scroll","seed":1,"headless":true,"persistentProfile":false,"webdriver":true,"userAgentOverridden":false,"viewportOverridden":false,"emulatedMobile":false,"detectors":[],"proxy":{"kind":"system","name":null,"host":null}}');
                "#,
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        let stored: String = connection
            .query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let settings: Settings = from_json(&stored).unwrap();
        assert_eq!(settings.default_browser, BrowserKind::Local);
        assert_eq!(settings.browserbase, BrowserbaseSettings::default());
        assert_eq!(settings.budget_usd, 0.25, "existing choices are kept");
        assert!(settings.persistent_profile);
        let remote: Option<String> = connection
            .query_row("SELECT remote FROM scans WHERE id = 'old'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remote, None);
    }

    #[tokio::test]
    async fn a_scan_stored_before_browserbase_reads_as_a_local_one() {
        use crate::application::ports::ScanRepository;
        use adscan_model::{
            BrowserUse, DeviceProfile, ScanOptions, ScanStatus, ScanSummary, SeverityCounts,
        };

        let store = SqliteStore::open_in_memory().unwrap();
        let summary = ScanSummary {
            id: "old".into(),
            url: "https://pub.example/".into(),
            hostname: "pub.example".into(),
            device: DeviceProfile::Desktop,
            status: ScanStatus::Completed,
            started_at: 1.0,
            finished_at: None,
            duration_ms: None,
            cost_usd: 0.0,
            finding_counts: SeverityCounts::default(),
            health_score: None,
            prebid_version: None,
            ai_used: false,
            error: None,
            configuration: None,
            proxy: None,
            remote: None,
        };
        store
            .create(&summary, &ScanOptions::for_url("https://pub.example/"))
            .await
            .unwrap();
        // What a build from before this change stored: no `browser` in the
        // options and none in the environment.
        store
            .call(|connection| {
                connection.execute(
                    r#"UPDATE scans SET
                        options = '{"url":"https://pub.example/","device":"desktop","observeSeconds":10,"autoScroll":true,"acceptConsent":true,"aiEnabled":false}',
                        environment = '{"visitor":"scroll","seed":1,"headless":true,"persistentProfile":false,"webdriver":true,"userAgentOverridden":false,"viewportOverridden":false,"emulatedMobile":false,"detectors":[],"proxy":{"kind":"system","name":null,"host":null}}'
                       WHERE id = 'old'"#,
                    [],
                )
            })
            .await
            .unwrap();

        let detail = store.detail("old").await.unwrap().unwrap();
        assert_eq!(detail.options.browser, None);
        assert_eq!(detail.summary.remote, None);
        assert_eq!(detail.environment.unwrap().browser, BrowserUse::Local);
    }

    #[test]
    fn every_database_gets_its_own_install_id() {
        let ids: Vec<String> = (0..2)
            .map(|_| {
                let mut connection = database_before();
                migrate(&mut connection).unwrap();
                connection
                    .query_row("SELECT install_id FROM browserbase_install", [], |row| {
                        row.get(0)
                    })
                    .unwrap()
            })
            .collect();
        assert_ne!(ids[0], ids[1]);
        for id in &ids {
            assert_eq!(id.len(), 32);
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            );
        }
    }

    #[tokio::test]
    async fn the_install_id_is_stable_for_one_database() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(
            store.install_id().await.unwrap(),
            store.install_id().await.unwrap()
        );
    }

    #[tokio::test]
    async fn the_ledger_lists_sessions_until_their_end_is_recorded() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.session_created("s2", 20.0).await.unwrap();
        store.session_created("s1", 10.0).await.unwrap();
        // Told twice about the same session: still one row.
        store.session_created("s1", 99.0).await.unwrap();
        let open = |store: SqliteStore| async move {
            store
                .open_sessions()
                .await
                .unwrap()
                .into_iter()
                .map(|session| (session.session_id, session.created_at))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            open(store.clone()).await,
            [("s1".to_string(), 10.0), ("s2".to_string(), 20.0)]
        );

        store.session_released("s1", 30.0).await.unwrap();
        // The end of a session the ledger never saw start is ignored.
        store.session_released("unknown", 30.0).await.unwrap();
        assert_eq!(open(store.clone()).await, [("s2".to_string(), 20.0)]);

        store.forget_released_sessions().await.unwrap();
        let rows: u32 = store
            .call(|connection| {
                connection.query_row("SELECT count(*) FROM browserbase_sessions", [], |row| {
                    row.get(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(rows, 1, "only the open session is kept");
    }

    #[tokio::test]
    async fn one_context_is_kept_per_device_and_all_are_forgotten_together() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert!(store.contexts().await.unwrap().is_empty());

        store
            .save_context(DeviceProfile::Desktop, "ctx-desktop", 1.0)
            .await
            .unwrap();
        store
            .save_context(DeviceProfile::Mobile, "ctx-mobile", 2.0)
            .await
            .unwrap();
        store
            .context_used(DeviceProfile::Mobile, 50.0)
            .await
            .unwrap();
        assert_eq!(
            store.contexts().await.unwrap(),
            [
                StoredContext {
                    device: DeviceProfile::Desktop,
                    context_id: "ctx-desktop".into(),
                    last_used_at: None,
                },
                StoredContext {
                    device: DeviceProfile::Mobile,
                    context_id: "ctx-mobile".into(),
                    last_used_at: Some(50.0),
                },
            ]
        );

        // A context that replaces one starts unused.
        store
            .save_context(DeviceProfile::Mobile, "ctx-mobile-2", 60.0)
            .await
            .unwrap();
        let contexts = store.contexts().await.unwrap();
        assert_eq!(contexts[1].context_id, "ctx-mobile-2");
        assert_eq!(contexts[1].last_used_at, None);

        store.forget_contexts().await.unwrap();
        assert!(store.contexts().await.unwrap().is_empty());
    }
}
