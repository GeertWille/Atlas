//! The sellers.json files of ad exchanges on the local disk, under
//! `<data dir>/cache/sellers-json/`.
//!
//! One entry is two files named by a hash of the exchange's domain: the file
//! as it was received, compressed (`<key>.json.zst`), and what is known about
//! it (`<key>.meta.json`). The entry exists once its meta file does, and the
//! meta file names the size and hash of its content: a write that was cut
//! short, or a file that was changed on disk, is no entry. Every file is
//! written under a temporary name, synced and renamed.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use adscan_model::SellersJsonCacheInfo;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::application::ports::{CachedSellersJson, FileVersion, SellersJsonCache};
use crate::domain::archive::codec;
use crate::domain::archive::index::sha256_hex;
use crate::domain::diagnostics::sellers_cache::{MAX_CACHE_BYTES, MAX_FILE_BYTES};

const CACHE_DIR: &str = "cache";
const SELLERS_DIR: &str = "sellers-json";
const BODY_SUFFIX: &str = ".json.zst";
const META_SUFFIX: &str = ".meta.json";
const TEMP_PREFIX: &str = ".tmp-";
/// Hex characters of the domain's sha-256 that name its files.
const KEY_LENGTH: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    domain: String,
    fetched_at: f64,
    final_url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    content: StoredContent,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
enum StoredContent {
    File {
        sha256: String,
        raw_bytes: u64,
        stored_bytes: u64,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone)]
pub struct LocalSellersJsonCache {
    directory: PathBuf,
    max_bytes: u64,
}

impl LocalSellersJsonCache {
    pub fn new(data_dir: &Path) -> Self {
        Self::with_limit(data_dir, MAX_CACHE_BYTES)
    }

    pub fn with_limit(data_dir: &Path, max_bytes: u64) -> Self {
        Self {
            directory: data_dir.join(CACHE_DIR).join(SELLERS_DIR),
            max_bytes,
        }
    }

    /// The name of a domain's files. A hash, so that nothing a domain could
    /// contain (a separator, `..`, a name the file system treats specially)
    /// ever becomes part of a path.
    fn key(domain: &str) -> String {
        let digest: [u8; 32] = Sha256::digest(domain.as_bytes()).into();
        sha256_hex(&digest)[..KEY_LENGTH].to_string()
    }

    fn body_path(&self, key: &str) -> PathBuf {
        self.directory.join(format!("{key}{BODY_SUFFIX}"))
    }

    fn meta_path(&self, key: &str) -> PathBuf {
        self.directory.join(format!("{key}{META_SUFFIX}"))
    }

    /// The directory, readable by the user only: the files say which ad
    /// exchanges the scanned sites work with.
    fn create_directory(&self) -> std::io::Result<()> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A directory that was already there keeps the mode it had.
            let own = [self.directory.parent(), Some(self.directory.as_path())];
            for existing in own.into_iter().flatten() {
                fs::set_permissions(existing, fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(())
    }

    /// `bytes` under `path`, whole or not at all.
    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        static WRITES: AtomicU64 = AtomicU64::new(0);
        let temporary = self.directory.join(format!(
            "{TEMP_PREFIX}{}-{}",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let written = options.open(&temporary).and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
        let renamed = written.and_then(|()| fs::rename(&temporary, path));
        if renamed.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        renamed
    }

    fn read_meta(&self, key: &str) -> Option<Meta> {
        serde_json::from_slice(&fs::read(self.meta_path(key)).ok()?).ok()
    }

    fn discard(&self, key: &str) {
        let _ = fs::remove_file(self.meta_path(key));
        let _ = fs::remove_file(self.body_path(key));
    }

    /// The entry as it was stored. `None` for anything else: a file of
    /// another domain, another size or another content than its meta says.
    fn read_entry(&self, domain: &str, key: &str) -> Option<CachedSellersJson> {
        let meta = self.read_meta(key).filter(|meta| meta.domain == domain)?;
        let content = match meta.content {
            StoredContent::Unavailable { reason } => Err(reason),
            StoredContent::File {
                sha256,
                raw_bytes,
                stored_bytes,
            } => {
                let path = self.body_path(key);
                // Checked before the file is read: a file that grew is not
                // loaded into memory to find that out.
                if fs::metadata(&path).ok()?.len() != stored_bytes {
                    return None;
                }
                let raw = codec::decompress(&fs::read(&path).ok()?, MAX_FILE_BYTES).ok()?;
                let digest: [u8; 32] = Sha256::digest(&raw).into();
                if raw.len() as u64 != raw_bytes || sha256_hex(&digest) != sha256 {
                    return None;
                }
                Ok(raw)
            }
        };
        Some(CachedSellersJson {
            fetched_at: meta.fetched_at,
            final_url: meta.final_url,
            version: FileVersion {
                etag: meta.etag,
                last_modified: meta.last_modified,
            },
            content,
        })
    }

    fn load_now(&self, domain: &str) -> Option<CachedSellersJson> {
        let key = Self::key(domain);
        if !self.meta_path(&key).exists() {
            return None;
        }
        let entry = self.read_entry(domain, &key);
        match &entry {
            // Used just now: the last to go when room is needed.
            Some(_) => {
                let _ = OpenOptions::new()
                    .write(true)
                    .open(self.meta_path(&key))
                    .and_then(|meta| meta.set_modified(SystemTime::now()));
            }
            None => self.discard(&key),
        }
        entry
    }

    fn store_now(&self, domain: &str, entry: CachedSellersJson) -> std::io::Result<()> {
        self.create_directory()?;
        let key = Self::key(domain);
        let content = match &entry.content {
            Ok(raw) => {
                let stored = codec::compress(raw)?;
                // The file before its meta: until the meta is replaced, the
                // old one does not match the new file, and that is no entry.
                self.write(&self.body_path(&key), &stored)?;
                let digest: [u8; 32] = Sha256::digest(raw).into();
                StoredContent::File {
                    sha256: sha256_hex(&digest),
                    raw_bytes: raw.len() as u64,
                    stored_bytes: stored.len() as u64,
                }
            }
            Err(reason) => StoredContent::Unavailable {
                reason: reason.clone(),
            },
        };
        let has_file = matches!(content, StoredContent::File { .. });
        let meta = Meta {
            domain: domain.to_string(),
            fetched_at: entry.fetched_at,
            final_url: entry.final_url,
            etag: entry.version.etag,
            last_modified: entry.version.last_modified,
            content,
        };
        self.write(&self.meta_path(&key), &serde_json::to_vec(&meta)?)?;
        if !has_file {
            let _ = fs::remove_file(self.body_path(&key));
        }
        self.make_room(&key);
        Ok(())
    }

    fn confirm_now(&self, domain: &str, fetched_at: f64) -> std::io::Result<()> {
        let key = Self::key(domain);
        let meta = self
            .read_meta(&key)
            .filter(|meta| meta.domain == domain)
            .ok_or_else(|| std::io::Error::other(format!("no kept sellers.json of {domain}")))?;
        let meta = Meta { fetched_at, ..meta };
        self.write(&self.meta_path(&key), &serde_json::to_vec(&meta)?)
    }

    /// Every file of the directory with its size.
    fn files(&self) -> Vec<(String, u64, SystemTime)> {
        let Ok(listing) = fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        listing
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let metadata = entry.metadata().ok().filter(fs::Metadata::is_file)?;
                Some((
                    entry.file_name().to_str()?.to_string(),
                    metadata.len(),
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                ))
            })
            .collect()
    }

    /// Removes the entries that were used longest ago until the rest fits.
    /// The entry that was just stored stays.
    fn make_room(&self, stored_key: &str) {
        let files = self.files();
        let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
        let size_of = |name: String| {
            files
                .iter()
                .find(|(file, _, _)| *file == name)
                .map_or(0, |(_, size, _)| *size)
        };
        let mut entries: Vec<(&str, SystemTime, u64)> = files
            .iter()
            .filter_map(|(name, size, used)| {
                let key = name.strip_suffix(META_SUFFIX)?;
                Some((key, *used, size + size_of(format!("{key}{BODY_SUFFIX}"))))
            })
            .filter(|(key, _, _)| *key != stored_key)
            .collect();
        entries.sort_by_key(|(_, used, _)| *used);
        for (key, _, size) in entries {
            if total <= self.max_bytes {
                break;
            }
            self.discard(key);
            total -= size;
        }
    }

    fn info_now(&self) -> SellersJsonCacheInfo {
        let files = self.files();
        SellersJsonCacheInfo {
            entries: files
                .iter()
                .filter(|(name, _, _)| name.ends_with(META_SUFFIX))
                .count() as u32,
            stored_bytes: files.iter().map(|(_, size, _)| size).sum::<u64>() as f64,
        }
    }

    fn clear_now(&self) -> std::io::Result<()> {
        for (name, _, _) in self.files() {
            fs::remove_file(self.directory.join(name))?;
        }
        Ok(())
    }

    fn sweep_now(&self) {
        let files = self.files();
        let has_meta = |key: &str| {
            let meta = format!("{key}{META_SUFFIX}");
            files.iter().any(|(name, _, _)| *name == meta)
        };
        for (name, _, _) in &files {
            let abandoned = name.starts_with(TEMP_PREFIX)
                || name
                    .strip_suffix(BODY_SUFFIX)
                    .is_some_and(|key| !has_meta(key));
            if abandoned {
                let _ = fs::remove_file(self.directory.join(name));
            }
        }
    }

    /// Megabytes are compressed, hashed and synced: not on an async worker.
    async fn off_the_runtime<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Self) -> T + Send + 'static,
    ) -> Result<T, String> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || work(&cache))
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl SellersJsonCache for LocalSellersJsonCache {
    async fn load(&self, domain: &str) -> Option<CachedSellersJson> {
        let domain = domain.to_string();
        self.off_the_runtime(move |cache| cache.load_now(&domain))
            .await
            .ok()
            .flatten()
    }

    async fn store(&self, domain: &str, entry: CachedSellersJson) -> Result<(), String> {
        let domain = domain.to_string();
        self.off_the_runtime(move |cache| cache.store_now(&domain, entry))
            .await?
            .map_err(|error| error.to_string())
    }

    async fn confirm(&self, domain: &str, fetched_at: f64) -> Result<(), String> {
        let domain = domain.to_string();
        self.off_the_runtime(move |cache| cache.confirm_now(&domain, fetched_at))
            .await?
            .map_err(|error| error.to_string())
    }

    async fn info(&self) -> SellersJsonCacheInfo {
        self.off_the_runtime(Self::info_now)
            .await
            .unwrap_or(SellersJsonCacheInfo {
                entries: 0,
                stored_bytes: 0.0,
            })
    }

    async fn clear(&self) -> Result<(), String> {
        self.off_the_runtime(Self::clear_now)
            .await?
            .map_err(|error| error.to_string())
    }

    async fn sweep(&self) {
        let _ = self.off_the_runtime(Self::sweep_now).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(fetched_at: f64, text: &str) -> CachedSellersJson {
        CachedSellersJson {
            fetched_at,
            final_url: "https://exchange.example/sellers.json".into(),
            version: FileVersion {
                etag: Some("\"v1\"".into()),
                last_modified: Some("Wed, 11 Nov 2026 09:00:00 GMT".into()),
            },
            content: Ok(text.as_bytes().to_vec()),
        }
    }

    fn names(cache: &LocalSellersJsonCache) -> Vec<String> {
        let mut names: Vec<String> = cache.files().into_iter().map(|(name, _, _)| name).collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn an_entry_comes_back_as_it_was_stored_and_a_failed_lookup_is_an_entry_too() {
        let directory = tempfile::tempdir().unwrap();
        let cache = LocalSellersJsonCache::new(directory.path());
        assert_eq!(cache.load("exchange.example").await, None);
        assert_eq!(
            cache.info().await,
            SellersJsonCacheInfo {
                entries: 0,
                stored_bytes: 0.0
            }
        );

        let stored = file(1_000.0, r#"{"sellers":[{"seller_id":"958"}]}"#);
        cache
            .store("exchange.example", stored.clone())
            .await
            .unwrap();
        assert_eq!(cache.load("exchange.example").await, Some(stored));
        assert_eq!(cache.load("other.example").await, None);

        let unavailable = CachedSellersJson {
            content: Err("the exchange answered HTTP 404".into()),
            version: FileVersion::default(),
            ..file(2_000.0, "")
        };
        cache
            .store("gone.example", unavailable.clone())
            .await
            .unwrap();
        assert_eq!(cache.load("gone.example").await, Some(unavailable.clone()));
        let info = cache.info().await;
        assert_eq!(info.entries, 2);
        assert!(info.stored_bytes > 0.0);

        // A file that is replaced by a failed lookup leaves no file behind.
        cache
            .store("exchange.example", unavailable.clone())
            .await
            .unwrap();
        assert_eq!(cache.load("exchange.example").await, Some(unavailable));
        assert_eq!(
            names(&cache)
                .iter()
                .filter(|name| name.ends_with(BODY_SUFFIX))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn the_exchange_confirming_its_file_only_moves_the_date() {
        let directory = tempfile::tempdir().unwrap();
        let cache = LocalSellersJsonCache::new(directory.path());
        cache
            .store("exchange.example", file(1_000.0, "{}"))
            .await
            .unwrap();
        cache.confirm("exchange.example", 9_000.0).await.unwrap();
        assert_eq!(
            cache.load("exchange.example").await,
            Some(file(9_000.0, "{}"))
        );
        assert!(cache.confirm("unknown.example", 9_000.0).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn only_the_user_can_read_the_cache_whatever_the_directory_was_before() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join(CACHE_DIR);
        fs::create_dir(&existing).unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
        let cache = LocalSellersJsonCache::new(directory.path());
        cache
            .store("exchange.example", file(1_000.0, "{}"))
            .await
            .unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&existing), 0o700);
        assert_eq!(mode(&cache.directory), 0o700);
        let files = names(&cache);
        assert_eq!(files.len(), 2);
        for name in files {
            assert_eq!(mode(&cache.directory.join(&name)), 0o600, "{name}");
        }
    }

    #[tokio::test]
    async fn no_domain_can_name_a_file_outside_the_cache_directory() {
        let directory = tempfile::tempdir().unwrap();
        let cache = LocalSellersJsonCache::new(directory.path());
        for hostile in [
            "../../../outside",
            "/etc/passwd",
            "..",
            "a/b",
            "a\\b",
            "nul",
            "exchange.example\0.json",
            "",
        ] {
            cache.store(hostile, file(1_000.0, "{}")).await.unwrap();
            assert_eq!(
                cache.load(hostile).await,
                Some(file(1_000.0, "{}")),
                "{hostile:?}"
            );
        }
        let outside: Vec<_> = walk(directory.path())
            .into_iter()
            .filter(|path| path.parent() != Some(cache.directory.as_path()))
            .collect();
        assert_eq!(outside, Vec::<PathBuf>::new());
        for name in names(&cache) {
            let (key, _) = name.split_once('.').unwrap();
            assert!(
                key.len() == KEY_LENGTH && key.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{name}"
            );
        }
    }

    fn walk(directory: &Path) -> Vec<PathBuf> {
        fs::read_dir(directory)
            .unwrap()
            .flat_map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path)
                } else {
                    vec![path]
                }
            })
            .collect()
    }

    #[tokio::test]
    async fn a_damaged_entry_is_none_and_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let cache = LocalSellersJsonCache::new(directory.path());
        let key = LocalSellersJsonCache::key("exchange.example");
        type Damage = Box<dyn Fn(&LocalSellersJsonCache)>;
        let damage: [(&str, Damage); 6] = [
            (
                "a cut-off file",
                Box::new(|cache| {
                    let path = cache.body_path(&LocalSellersJsonCache::key("exchange.example"));
                    let bytes = fs::read(&path).unwrap();
                    fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
                }),
            ),
            (
                "other content of the same size",
                Box::new(|cache| {
                    let path = cache.body_path(&LocalSellersJsonCache::key("exchange.example"));
                    let mut bytes = fs::read(&path).unwrap();
                    let last = bytes.len() - 1;
                    bytes[last / 2] ^= 0xff;
                    fs::write(&path, bytes).unwrap();
                }),
            ),
            (
                "a missing file",
                Box::new(|cache| {
                    fs::remove_file(
                        cache.body_path(&LocalSellersJsonCache::key("exchange.example")),
                    )
                    .unwrap();
                }),
            ),
            (
                "a meta file that is not JSON",
                Box::new(|cache| {
                    fs::write(
                        cache.meta_path(&LocalSellersJsonCache::key("exchange.example")),
                        b"{",
                    )
                    .unwrap();
                }),
            ),
            (
                "the entry of another domain under this name",
                Box::new(|cache| {
                    let own = LocalSellersJsonCache::key("exchange.example");
                    let other = LocalSellersJsonCache::key("other.example");
                    cache
                        .store_now("other.example", file(1_000.0, r#"{"sellers":[]}"#))
                        .unwrap();
                    fs::rename(cache.meta_path(&other), cache.meta_path(&own)).unwrap();
                    fs::rename(cache.body_path(&other), cache.body_path(&own)).unwrap();
                }),
            ),
            (
                "a file that unpacks to more than a sellers.json may hold",
                Box::new(|cache| {
                    let key = LocalSellersJsonCache::key("exchange.example");
                    let bomb = codec::compress(&vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
                    let mut meta: serde_json::Value =
                        serde_json::from_slice(&fs::read(cache.meta_path(&key)).unwrap()).unwrap();
                    meta["content"]["file"]["storedBytes"] = bomb.len().into();
                    meta["content"]["file"]["rawBytes"] = (MAX_FILE_BYTES + 1).into();
                    fs::write(cache.body_path(&key), bomb).unwrap();
                    fs::write(cache.meta_path(&key), meta.to_string()).unwrap();
                }),
            ),
        ];
        for (what, damage) in damage {
            cache
                .store("exchange.example", file(1_000.0, r#"{"sellers":[]}"#))
                .await
                .unwrap();
            damage(&cache);
            assert_eq!(cache.load("exchange.example").await, None, "{what}");
            assert!(
                !cache.meta_path(&key).exists() && !cache.body_path(&key).exists(),
                "{what}"
            );
        }
    }

    #[tokio::test]
    async fn over_the_limit_the_entries_used_longest_ago_go_first() {
        let directory = tempfile::tempdir().unwrap();
        // Incompressible content, so that sizes are what they look like.
        let content = |seed: u8| -> String {
            (0..3_000u32)
                .map(|index| {
                    let digest = Sha256::digest([seed, index as u8, (index >> 8) as u8]);
                    format!("{:02x}", digest[0])
                })
                .collect()
        };
        let cache = LocalSellersJsonCache::with_limit(directory.path(), 10_000);
        let used = |cache: &LocalSellersJsonCache, domain: &str, seconds: u64| {
            let meta = cache.meta_path(&LocalSellersJsonCache::key(domain));
            let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
            OpenOptions::new()
                .write(true)
                .open(meta)
                .unwrap()
                .set_modified(at)
                .unwrap();
        };
        cache
            .store("a.example", file(1.0, &content(1)))
            .await
            .unwrap();
        cache
            .store("b.example", file(1.0, &content(2)))
            .await
            .unwrap();
        cache
            .store("c.example", file(1.0, &content(3)))
            .await
            .unwrap();
        assert_eq!(cache.info().await.entries, 3, "three fit");
        used(&cache, "a.example", 300);
        used(&cache, "b.example", 100);
        used(&cache, "c.example", 200);

        cache
            .store("d.example", file(1.0, &content(4)))
            .await
            .unwrap();
        assert_eq!(cache.load("b.example").await, None, "used longest ago");
        assert!(cache.load("c.example").await.is_some());
        assert!(cache.load("a.example").await.is_some());
        assert!(cache.load("d.example").await.is_some());
        assert!(cache.info().await.stored_bytes <= 10_000.0);

        // Reading an entry is using it: `c` was read before `a` just now.
        cache
            .store("e.example", file(1.0, &content(5)))
            .await
            .unwrap();
        assert_eq!(cache.load("c.example").await, None);
        assert!(cache.load("a.example").await.is_some());

        // One file larger than the whole limit is still kept: it was asked for.
        let tiny = LocalSellersJsonCache::with_limit(directory.path(), 100);
        tiny.store("f.example", file(1.0, &content(6)))
            .await
            .unwrap();
        assert!(tiny.load("f.example").await.is_some());
        assert_eq!(tiny.info().await.entries, 1);
    }

    #[tokio::test]
    async fn clearing_empties_the_cache_and_the_sweep_removes_only_leftovers() {
        let directory = tempfile::tempdir().unwrap();
        let cache = LocalSellersJsonCache::new(directory.path());
        // Nothing to do, nothing to fail on, before the directory exists.
        cache.sweep().await;
        cache.clear().await.unwrap();

        cache
            .store("exchange.example", file(1_000.0, "{}"))
            .await
            .unwrap();
        // A download that stopped half-way, and a file whose meta never came.
        fs::write(cache.directory.join(".tmp-4711-0"), b"{\"sellers\":[").unwrap();
        fs::write(
            cache.body_path(&LocalSellersJsonCache::key("half.example")),
            b"x",
        )
        .unwrap();
        assert_eq!(
            cache.load("half.example").await,
            None,
            "never an entry without its meta"
        );
        assert_eq!(cache.info().await.entries, 1);

        cache.sweep().await;
        assert_eq!(names(&cache).len(), 2);
        assert!(cache.load("exchange.example").await.is_some());

        fs::write(cache.directory.join(".tmp-4711-1"), b"x").unwrap();
        cache.clear().await.unwrap();
        assert_eq!(names(&cache), Vec::<String>::new());
        assert_eq!(cache.load("exchange.example").await, None);
    }
}
