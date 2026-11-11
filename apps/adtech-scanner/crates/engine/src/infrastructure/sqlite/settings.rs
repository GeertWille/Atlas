use adscan_model::Settings;
use async_trait::async_trait;
use rusqlite::OptionalExtension;

use super::{SqliteStore, from_json, to_json};
use crate::application::ports::{RepoResult, SettingsRepository};

#[async_trait]
impl SettingsRepository for SqliteStore {
    async fn load(&self) -> RepoResult<Option<Settings>> {
        self.call(|connection| {
            connection
                .query_row("SELECT data FROM settings WHERE id = 1", [], |row| row.get::<_, String>(0))
                .optional()?
                .map(|data| from_json(&data))
                .transpose()
        })
        .await
    }

    async fn save(&self, settings: &Settings) -> RepoResult<()> {
        let settings = settings.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO settings (id, data) VALUES (1, ?1) ON CONFLICT (id) DO UPDATE SET data = excluded.data",
                [to_json(&settings)?],
            )?;
            Ok(())
        })
        .await
    }
}
