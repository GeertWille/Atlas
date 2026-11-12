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
    /// from pointing outside the archive directory all the same.
    fn path(&self, file: &str) -> Result<PathBuf, PackError> {
        let relative = Path::new(file);
        let plain = relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if !plain || file.is_empty() {
            return Err(PackError::Io(format!("not a pack file name: {file}")));
        }
        Ok(self.root.join(relative))
    }
}

#[async_trait]
impl PackStore for LocalPackStore {
    async fn create(&self, file: &str) -> Result<Box<dyn PackWriter>, PackError> {
        let path = self.path(file)?;
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)
                .await
                .map_err(|error| io_error(file, &error))?;
        }
        let handle = File::create(&path)
            .await
            .map_err(|error| io_error(file, &error))?;
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
        handle
            .seek(SeekFrom::Start(location.offset))
            .await
            .map_err(|error| io_error(&location.file, &error))?;
        let mut bytes = vec![0u8; location.length as usize];
        handle.read_exact(&mut bytes).await.map_err(|error| {
            if error.kind() == ErrorKind::UnexpectedEof {
                PackError::OutOfRange(format!(
                    "{} ends before byte {}",
                    location.file,
                    location.offset + u64::from(location.length)
                ))
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
            .ok_or_else(|| PackError::Io(format!("a frame of {} bytes cannot be addressed", frame.len())))?;
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
