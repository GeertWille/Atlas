//! zstd frames: every body and every record frame is compressed alone, so
//! any of them can be read by its byte range without the rest of the pack.

use std::io::{Read, Write};

use super::pack::ZSTD_LEVEL;

/// One complete zstd frame of `bytes`, with the content checksum on so a
/// damaged range is noticed when it is read.
pub fn compress(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), ZSTD_LEVEL)?;
    encoder.include_checksum(true)?;
    encoder.write_all(bytes)?;
    encoder.finish()
}

/// The content of one frame. Fails on a damaged frame and on content longer
/// than `max_len`, so a wrong index row cannot make the app allocate without
/// bound.
pub fn decompress(frame: &[u8], max_len: usize) -> std::io::Result<Vec<u8>> {
    let mut content = Vec::new();
    zstd::stream::Decoder::new(frame)?
        .take(max_len as u64 + 1)
        .read_to_end(&mut content)?;
    if content.len() > max_len {
        return Err(std::io::Error::other(format!(
            "the frame holds more than the expected {max_len} bytes"
        )));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_gives_back_its_content() {
        let content = b"function a(){return 1}".repeat(500);
        let frame = compress(&content).unwrap();
        assert!(frame.len() < content.len() / 10);
        assert_eq!(decompress(&frame, content.len()).unwrap(), content);
    }

    #[test]
    fn a_frame_is_a_plain_zstd_file() {
        let frame = compress(b"hello").unwrap();
        assert_eq!(frame[0..4], [0x28, 0xB5, 0x2F, 0xFD], "zstd magic");
        assert_eq!(
            zstd::stream::decode_all(frame.as_slice()).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn frames_written_one_after_another_still_decode_alone() {
        let first = compress(b"first body").unwrap();
        let second = compress(b"second body").unwrap();
        let pack = [first.clone(), second.clone()].concat();
        assert_eq!(decompress(&pack[..first.len()], 64).unwrap(), b"first body");
        assert_eq!(
            decompress(&pack[first.len()..], 64).unwrap(),
            b"second body"
        );
    }

    #[test]
    fn a_flipped_bit_is_caught_by_the_checksum() {
        let content = b"0123456789abcdef".repeat(64);
        let mut frame = compress(&content).unwrap();
        let middle = frame.len() / 2;
        frame[middle] ^= 0x01;
        assert!(decompress(&frame, content.len()).is_err());
    }

    #[test]
    fn a_cut_off_frame_is_an_error_not_a_short_body() {
        let content = b"abcdefgh".repeat(1_000);
        let frame = compress(&content).unwrap();
        assert!(decompress(&frame[..frame.len() - 5], content.len()).is_err());
        assert!(decompress(&frame[..3], content.len()).is_err());
        assert!(decompress(&[], 10).is_err());
    }

    #[test]
    fn content_longer_than_expected_is_refused() {
        let frame = compress(&[7u8; 4_096]).unwrap();
        assert!(decompress(&frame, 4_095).is_err());
        assert!(decompress(&frame, 4_096).is_ok());
    }
}
