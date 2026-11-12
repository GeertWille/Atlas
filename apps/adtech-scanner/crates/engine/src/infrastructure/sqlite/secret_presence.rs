use std::collections::HashSet;

use async_trait::async_trait;

use super::SqliteStore;
use crate::application::ports::{RepoResult, SecretName, SecretPresenceRepository};
use crate::infrastructure::secrets::{secret_key, secret_name};

/// Earlier versions asked the keychain what exists and kept no record. For a
/// database they wrote, the record starts from what its data implies: an
/// Anthropic key when a scan used AI, a TypeSafe key when Jev is on, and a
/// password for every proxy profile, which is also where the secret store
/// learns which old keychain items to look for. The first read of the
/// keychain replaces these guesses with what is really there.
pub(super) const MIGRATION: &str = "
    CREATE TABLE secret_presence (name TEXT PRIMARY KEY) WITHOUT ROWID;

    INSERT INTO secret_presence (name)
    SELECT 'anthropic' WHERE EXISTS (SELECT 1 FROM scans WHERE ai_used = 1);

    INSERT INTO secret_presence (name)
    SELECT 'typesafe' FROM settings WHERE json_extract(data, '$.jevEnabled');

    INSERT OR IGNORE INTO secret_presence (name)
    SELECT 'proxy:' || json_extract(profile.value, '$.id')
    FROM settings, json_each(settings.data, '$.proxies') AS profile;
    ";

#[async_trait]
impl SecretPresenceRepository for SqliteStore {
    async fn load_presence(&self) -> RepoResult<HashSet<SecretName>> {
        self.call(|connection| {
            let mut statement = connection.prepare("SELECT name FROM secret_presence")?;
            let keys = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(keys.iter().filter_map(|key| secret_name(key)).collect())
        })
        .await
    }

    async fn save_presence(&self, names: &HashSet<SecretName>) -> RepoResult<()> {
        let keys: Vec<String> = names.iter().map(secret_key).collect();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute("DELETE FROM secret_presence", [])?;
            for key in keys {
                transaction.execute("INSERT INTO secret_presence (name) VALUES (?1)", [key])?;
            }
            transaction.commit()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use adscan_model::ApiProvider;
    use rusqlite::Connection;

    use super::super::{MIGRATIONS, migrate};
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
            .pragma_update(None, "user_version", earlier)
            .unwrap();
        connection
    }

    fn recorded(connection: &Connection) -> Vec<String> {
        connection
            .prepare("SELECT name FROM secret_presence ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn a_new_database_records_no_secrets() {
        let mut connection = database_before();
        migrate(&mut connection).unwrap();
        assert!(recorded(&connection).is_empty());
    }

    #[test]
    fn an_earlier_database_starts_from_what_its_data_implies() {
        let mut connection = database_before();
        connection
            .execute_batch(
                r#"
                INSERT INTO scans (id, url, hostname, device, status, started_at, ai_used, options)
                VALUES ('s1', 'https://pub.example/', 'pub.example', 'desktop', 'completed', 1, 1, '{}');
                INSERT INTO settings (id, data)
                VALUES (1, '{"jevEnabled": true, "proxies": [{"id": "p1"}, {"id": "p2"}]}');
                "#,
            )
            .unwrap();
        migrate(&mut connection).unwrap();
        assert_eq!(
            recorded(&connection),
            ["anthropic", "proxy:p1", "proxy:p2", "typesafe"]
        );
    }

    #[test]
    fn an_earlier_database_without_ai_scans_jev_or_proxies_records_nothing() {
        let mut connection = database_before();
        connection
            .execute_batch(
                r#"
                INSERT INTO scans (id, url, hostname, device, status, started_at, ai_used, options)
                VALUES ('s1', 'https://pub.example/', 'pub.example', 'desktop', 'completed', 1, 0, '{}');
                INSERT INTO settings (id, data) VALUES (1, '{"jevEnabled": false, "proxies": []}');
                "#,
            )
            .unwrap();
        migrate(&mut connection).unwrap();
        assert!(recorded(&connection).is_empty());
    }

    #[tokio::test]
    async fn the_record_is_replaced_as_a_whole() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert!(store.load_presence().await.unwrap().is_empty());

        let first = HashSet::from([
            SecretName::ApiKey(ApiProvider::Anthropic),
            SecretName::ProxyPassword("p1".into()),
        ]);
        store.save_presence(&first).await.unwrap();
        assert_eq!(store.load_presence().await.unwrap(), first);

        let second = HashSet::from([SecretName::ApiKey(ApiProvider::Typesafe)]);
        store.save_presence(&second).await.unwrap();
        assert_eq!(store.load_presence().await.unwrap(), second);
    }
}
