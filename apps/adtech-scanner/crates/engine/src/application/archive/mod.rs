//! The scan archive: writing it during a capture, reading it back for the
//! network view and the script viewer, and keeping it within its limit.

pub mod analysis;
pub mod codec;
pub mod network;
pub mod recording;
pub mod source;
pub mod storage;

use std::sync::Arc;

use super::ports::{ArchiveRepository, PackStore};

/// The two halves of every archive: the pack files and their index.
#[derive(Clone)]
pub struct Archive {
    pub packs: Arc<dyn PackStore>,
    pub index: Arc<dyn ArchiveRepository>,
}
