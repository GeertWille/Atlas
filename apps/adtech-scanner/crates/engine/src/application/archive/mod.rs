//! The scan archive: writing it during a capture, reading it back for the
//! network view and the script viewer, and keeping it within its limit.

pub mod analysis;
pub mod network;
pub mod recording;
pub mod source;
pub mod storage;

use std::sync::Arc;

use super::AppError;
use super::ports::{ArchiveRepository, BodyIndexRow, PackStore};
use crate::domain::archive::kinds::MAX_STORED_BODY_BYTES;
use crate::domain::archive::{Location, codec};

/// The two halves of every archive: the pack files and their index.
#[derive(Clone)]
pub struct Archive {
    pub packs: Arc<dyn PackStore>,
    pub index: Arc<dyn ArchiveRepository>,
}

impl Archive {
    /// The content of one frame of a pack, which may be at most
    /// `max_content` bytes. Where a frame is and how long it is comes from
    /// the index, and the pack is a file anyone can replace: neither is
    /// trusted to stay within what this app would have written.
    async fn frame(&self, location: Location, max_content: usize) -> Result<Vec<u8>, AppError> {
        // zstd adds less than this to content it cannot compress.
        let max_frame = max_content + max_content / 128 + 1024;
        if location.length as usize > max_frame {
            return Err(AppError::Storage(format!(
                "a frame of {} bytes in {} is larger than anything this app stores",
                location.length, location.file
            )));
        }
        let frame = self
            .packs
            .read(&location)
            .await
            .map_err(storage::pack_error)?;
        tokio::task::spawn_blocking(move || codec::decompress(&frame, max_content))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .map_err(|error| AppError::Storage(format!("stored data could not be read: {error}")))
    }

    /// The stored bytes of a body.
    async fn body(&self, pack_file: String, body: &BodyIndexRow) -> Result<Vec<u8>, AppError> {
        let max_content = body.raw_len.min(MAX_STORED_BODY_BYTES as u64) as usize;
        self.frame(
            Location {
                file: pack_file,
                offset: body.pack_offset,
                length: body.pack_length,
            },
            max_content,
        )
        .await
    }
}
