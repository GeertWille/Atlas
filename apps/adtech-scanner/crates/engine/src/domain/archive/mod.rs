//! What a scan archive holds and how it is laid out: one pack file per scan
//! (independent zstd frames addressed by location) plus an index of request
//! rows. Pure: the bytes are moved by the application layer.

pub mod document;
pub mod index;
pub mod kinds;
pub mod location;
pub mod pack;
pub mod redact;

pub use document::{RequestDocument, StoredBody};
pub use kinds::BodyKind;
pub use location::Location;
