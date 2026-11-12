//! Pack files on the local disk, under `<data dir>/archive/`.

use std::io::{ErrorKind, SeekFrom};
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::application::ports::{PackError, PackInfo, PackStore, PackWriter};
use crate::domain::archive::Location;

const ARCHIVE_DIR: &str = "archive";
const PACK_SUFFIX: &str = ".pack.zst";

pub struct LocalPackStore {
    root: PathBuf,
}

impl LocalPackStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join(ARCHIVE_DIR),
        }
    }

    /// File names come from the app itself; this keeps a damaged index row
    /// from pointing outside the archive directory, or at a file that is no
    /// pack, all the same.
    fn path(&self, file: &str) -> Result<PathBuf, PackError> {
        let relative = Path::new(file);
        let plain = relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if !plain || !file.ends_with(PACK_SUFFIX) {
            return Err(PackError::Io(format!("not a pack file name: {file}")));
        }
        Ok(self.root.join(relative))
    }

    /// Creates the directory of `path` below the archive root. Packs hold
    /// what a browser sent and received: on Unix only the user may read
    /// them, whatever the umask says.
    async fn create_directory_of(&self, path: &Path) -> std::io::Result<()> {
        let Some(directory) = path.parent() else {
            return Ok(());
        };
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(directory).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A directory that was already there keeps the mode it had.
            for existing in [self.root.as_path(), directory] {
                fs::set_permissions(existing, std::fs::Permissions::from_mode(0o700)).await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl PackStore for LocalPackStore {
    async fn create(&self, file: &str) -> Result<Box<dyn PackWriter>, PackError> {
        let path = self.path(file)?;
        self.create_directory_of(&path)
            .await
            .map_err(|error| io_error(file, &error))?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let handle = options
            .open(&path)
            .await
            .map_err(|error| io_error(file, &error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // The mode above only applies to a file that did not exist.
            handle
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(|error| io_error(file, &error))?;
        }
        Ok(Box::new(LocalPackWriter {
            file: file.to_string(),
            handle,
            length: 0,
        }))
    }

    async fn read(&self, location: &Location) -> Result<Vec<u8>, PackError> {
        let path = self.path(&location.file)?;
        let mut handle = File::open(&path).await.map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                PackError::Missing(location.file.clone())
            } else {
                io_error(&location.file, &error)
            }
        })?;
        // The range is checked against the file before a byte is reserved
        // for it: a wrong length in the index must not become an allocation.
        let size = handle
            .metadata()
            .await
            .map_err(|error| io_error(&location.file, &error))?
            .len();
        let end = location.offset.checked_add(u64::from(location.length));
        if end.is_none_or(|end| end > size) {
            return Err(PackError::OutOfRange(format!(
                "{} has {size} bytes, not the {} at byte {} that were asked for",
                location.file, location.length, location.offset
            )));
        }
        handle
            .seek(SeekFrom::Start(location.offset))
            .await
            .map_err(|error| io_error(&location.file, &error))?;
        let mut bytes = vec![0u8; location.length as usize];
        handle.read_exact(&mut bytes).await.map_err(|error| {
            if error.kind() == ErrorKind::UnexpectedEof {
                // The file shrank between the check and the read.
                PackError::OutOfRange(format!("{} ended early", location.file))
            } else {
                io_error(&location.file, &error)
            }
        })?;
        Ok(bytes)
    }

    async fn delete(&self, file: &str) -> Result<(), PackError> {
        match fs::remove_file(self.path(file)?).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(file, &error)),
        }
    }

    async fn list(&self) -> Result<Vec<PackInfo>, PackError> {
        let mut packs = Vec::new();
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            let mut entries = match fs::read_dir(&directory).await {
                Ok(entries) => entries,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(ARCHIVE_DIR, &error)),
            };
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|error| io_error(ARCHIVE_DIR, &error))?
            {
                let path = entry.path();
                let metadata = entry
                    .metadata()
                    .await
                    .map_err(|error| io_error(ARCHIVE_DIR, &error))?;
                if metadata.is_dir() {
                    pending.push(path);
                    continue;
                }
                let name = path
                    .strip_prefix(&self.root)
                    .ok()
                    .and_then(|relative| relative.to_str())
                    .map(|relative| relative.replace('\\', "/"));
                if let Some(file) = name.filter(|name| name.ends_with(PACK_SUFFIX)) {
                    packs.push(PackInfo {
                        file,
                        bytes: metadata.len(),
                    });
                }
            }
        }
        packs.sort_by(|a, b| a.file.cmp(&b.file));
        Ok(packs)
    }
}

struct LocalPackWriter {
    file: String,
    handle: File,
    length: u64,
}

#[async_trait]
impl PackWriter for LocalPackWriter {
    async fn append(&mut self, frame: Vec<u8>) -> Result<Location, PackError> {
        let length = u32::try_from(frame.len())
            .ok()
            .filter(|length| *length > 0)
            .ok_or_else(|| {
                PackError::Io(format!(
                    "a frame of {} bytes cannot be addressed",
                    frame.len()
                ))
            })?;
        self.handle
            .write_all(&frame)
            .await
            .map_err(|error| io_error(&self.file, &error))?;
        let location = Location {
            file: self.file.clone(),
            offset: self.length,
            length,
        };
        self.length += u64::from(length);
        Ok(location)
    }

    async fn sync(&mut self) -> Result<(), PackError> {
        self.handle
            .flush()
            .await
            .map_err(|error| io_error(&self.file, &error))?;
        self.handle
            .sync_data()
            .await
            .map_err(|error| io_error(&self.file, &error))
    }

    async fn seal(mut self: Box<Self>) -> Result<PackInfo, PackError> {
        self.handle
            .flush()
            .await
            .map_err(|error| io_error(&self.file, &error))?;
        self.handle
            .sync_all()
            .await
            .map_err(|error| io_error(&self.file, &error))?;
        Ok(PackInfo {
            file: self.file,
            bytes: self.length,
        })
    }
}

fn io_error(file: &str, error: &std::io::Error) -> PackError {
    PackError::Io(format!("{file}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_are_appended_one_after_another_and_read_back_by_location() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalPackStore::new(directory.path());
        let mut writer = store.create("scans/a.pack.zst").await.unwrap();
        let first = writer.append(b"first".to_vec()).await.unwrap();
        let second = writer.append(b"second frame".to_vec()).await.unwrap();
        assert_eq!((first.offset, first.length), (0, 5));
        assert_eq!((second.offset, second.length), (5, 12));
        assert_eq!(second.compose(), "scans/a.pack.zst#o=5&l=12");
        writer.sync().await.unwrap();
        // Readable while the pack is still being written.
        assert_eq!(store.read(&second).await.unwrap(), b"second frame");
        let sealed = writer.seal().await.unwrap();
        assert_eq!(
            sealed,
            PackInfo {
                file: "scans/a.pack.zst".into(),
                bytes: 17
            }
        );
        assert_eq!(store.read(&first).await.unwrap(), b"first");
        assert!(directory.path().join("archive/scans/a.pack.zst").is_file());
    }

    #[tokio::test]
    async fn a_range_past_the_end_fails_instead_of_reading_short() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalPackStore::new(directory.path());
        let mut writer = store.create("scans/a.pack.zst").await.unwrap();
        writer.append(b"0123456789".to_vec()).await.unwrap();
        writer.seal().await.unwrap();
        let range = |offset, length| Location {
            file: "scans/a.pack.zst".into(),
            offset,
            length,
        };
        assert_eq!(store.read(&range(8, 2)).await.unwrap(), b"89");
        assert!(matches!(
            store.read(&range(8, 3)).await,
            Err(PackError::OutOfRange(_))
        ));
        assert!(matches!(
            store.read(&range(50, 1)).await,
            Err(PackError::OutOfRange(_))
        ));
        let missing = Location {
            file: "scans/none.pack.zst".into(),
            offset: 0,
            length: 1,
        };
        assert_eq!(
            store.read(&missing).await,
            Err(PackError::Missing("scans/none.pack.zst".into()))
        );
    }

    #[tokio::test]
    async fn packs_are_listed_and_deleted_and_deleting_twice_is_fine() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalPackStore::new(directory.path());
        assert!(
            store.list().await.unwrap().is_empty(),
            "no archive directory yet"
        );
        for name in ["scans/b.pack.zst", "scans/a.pack.zst"] {
            let mut writer = store.create(name).await.unwrap();
            writer.append(name.as_bytes().to_vec()).await.unwrap();
            writer.seal().await.unwrap();
        }
        std::fs::write(
            directory.path().join("archive/scans/notes.txt"),
            "not a pack",
        )
        .unwrap();
        let listed: Vec<String> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|pack| pack.file)
            .collect();
        assert_eq!(listed, ["scans/a.pack.zst", "scans/b.pack.zst"]);
        store.delete("scans/a.pack.zst").await.unwrap();
        store.delete("scans/a.pack.zst").await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn names_that_leave_the_archive_directory_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalPackStore::new(directory.path());
        for name in [
            "../outside.pack.zst",
            "/etc/passwd",
            "/tmp/x.pack.zst",
            "scans/../../x.pack.zst",
            "scans/notes.txt",
            "adscan.sqlite",
            "",
        ] {
            assert!(store.create(name).await.is_err(), "{name}");
            assert!(store.delete(name).await.is_err(), "{name}");
            let location = Location {
                file: name.into(),
                offset: 0,
                length: 1,
            };
            assert!(store.read(&location).await.is_err(), "{name}");
        }
    }

    #[tokio::test]
    async fn an_empty_frame_cannot_be_appended() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalPackStore::new(directory.path());
        let mut writer = store.create("scans/a.pack.zst").await.unwrap();
        assert!(writer.append(Vec::new()).await.is_err());
    }

    #[tokio::test]
    async fn a_range_the_file_does_not_have_is_refused_before_it_is_allocated() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalPackStore::new(directory.path());
        let mut writer = store.create("scans/a.pack.zst").await.unwrap();
        writer.append(b"0123456789".to_vec()).await.unwrap();
        writer.seal().await.unwrap();
        // Four gigabytes "at" the start, and offsets whose end does not fit
        // into 64 bits: answered at once, without reserving the memory.
        for (offset, length) in [
            (0, u32::MAX),
            (u64::MAX, 1),
            (u64::MAX - 3, u32::MAX),
            (10, 1),
        ] {
            let location = Location {
                file: "scans/a.pack.zst".into(),
                offset,
                length,
            };
            assert!(
                matches!(store.read(&location).await, Err(PackError::OutOfRange(_))),
                "{offset} {length}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn only_the_user_can_read_the_archive_directory_and_its_packs() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        // Left behind by something with a wider umask.
        let archive = directory.path().join("archive");
        std::fs::create_dir(&archive).unwrap();
        std::fs::set_permissions(&archive, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir(archive.join("scans")).unwrap();
        std::fs::write(archive.join("scans/b.pack.zst"), "old").unwrap();
        std::fs::set_permissions(
            archive.join("scans/b.pack.zst"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();

        let store = LocalPackStore::new(directory.path());
        for name in ["scans/a.pack.zst", "scans/b.pack.zst"] {
            let mut writer = store.create(name).await.unwrap();
            writer.append(b"frame".to_vec()).await.unwrap();
            writer.seal().await.unwrap();
        }
        let mode = |path: &str| {
            std::fs::metadata(directory.path().join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("archive"), 0o700);
        assert_eq!(mode("archive/scans"), 0o700);
        assert_eq!(mode("archive/scans/a.pack.zst"), 0o600);
        assert_eq!(mode("archive/scans/b.pack.zst"), 0o600);
    }
}
