//! Pack format, version 1. A pack is a sequence of zstd frames in capture
//! order: one frame per distinct body, record frames of JSON lines, then a
//! manifest frame and a trailer. The trailer is a zstd skippable frame, so
//! the whole file is a valid `.zst` and any zstd tool reads past it.

use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
/// zstd level of every frame: the data lake's level.
pub const ZSTD_LEVEL: i32 = 3;
/// A record frame is closed at this many documents, or this much JSON.
pub const RECORDS_PER_FRAME: usize = 128;
pub const MAX_RECORD_FRAME_BYTES: usize = 256 * 1024;

const SKIPPABLE_MAGIC: u32 = 0x184D_2A50;
const TAG: &[u8; 8] = b"ADSCANPK";
/// Magic, payload length, tag, format version, manifest offset and length.
pub const TRAILER_LEN: usize = 4 + 4 + 8 + 4 + 8 + 4;

/// The last frame before the trailer: enough to rebuild the index from the
/// file alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    pub app_version: String,
    pub scan_id: String,
    pub url: String,
    /// Unix epoch ms.
    pub started_at: f64,
    pub headers_redacted: bool,
    pub request_count: u32,
    pub bodies: Vec<ManifestBody>,
    pub records: Vec<ManifestRecords>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestBody {
    pub sha256: String,
    pub offset: u64,
    pub length: u32,
    pub raw_length: u64,
    pub kind: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRecords {
    pub offset: u64,
    pub length: u32,
    pub count: u32,
}

pub fn trailer(manifest_offset: u64, manifest_length: u32) -> [u8; TRAILER_LEN] {
    let mut bytes = [0u8; TRAILER_LEN];
    bytes[0..4].copy_from_slice(&SKIPPABLE_MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&((TRAILER_LEN - 8) as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(TAG);
    bytes[16..20].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[20..28].copy_from_slice(&manifest_offset.to_le_bytes());
    bytes[28..32].copy_from_slice(&manifest_length.to_le_bytes());
    bytes
}

/// Offset and length of the manifest frame, read from the last
/// [`TRAILER_LEN`] bytes of a sealed pack. `None` for anything else: a pack
/// that was never sealed, a truncated one, another format version.
pub fn manifest_range(trailer: &[u8]) -> Option<(u64, u32)> {
    let bytes: &[u8; TRAILER_LEN] = trailer.try_into().ok()?;
    let magic = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let payload = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let version = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    if magic != SKIPPABLE_MAGIC
        || payload as usize != TRAILER_LEN - 8
        || &bytes[8..16] != TAG
        || version != FORMAT_VERSION
    {
        return None;
    }
    Some((
        u64::from_le_bytes(bytes[20..28].try_into().ok()?),
        u32::from_le_bytes(bytes[28..32].try_into().ok()?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trailer_names_the_manifest() {
        let bytes = trailer(9_876_543_210, 4_321);
        assert_eq!(manifest_range(&bytes), Some((9_876_543_210, 4_321)));
    }

    #[test]
    fn the_trailer_is_a_zstd_skippable_frame() {
        let bytes = trailer(1, 2);
        // Magic 0x184D2A50, little endian, then the length of what follows.
        assert_eq!(bytes[0..4], [0x50, 0x2A, 0x4D, 0x18]);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 24);
    }

    #[test]
    fn damaged_short_and_foreign_trailers_are_not_read() {
        let good = trailer(100, 50);
        assert_eq!(manifest_range(&good[1..]), None, "too short");
        let mut wrong_tag = good;
        wrong_tag[9] ^= 0xFF;
        assert_eq!(manifest_range(&wrong_tag), None);
        let mut newer = good;
        newer[16] = 2;
        assert_eq!(manifest_range(&newer), None, "another format version");
        assert_eq!(manifest_range(&[0u8; TRAILER_LEN]), None);
    }
}
