//! `scans/<scan id>.pack.zst#o=<offset>&l=<length>`: a byte range of a pack
//! file, in the syntax of the owner's data lake so that a pack can later be
//! read there as it is.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub offset: u64,
    pub length: u32,
}

impl Location {
    pub fn compose(&self) -> String {
        format!("{}#o={}&l={}", self.file, self.offset, self.length)
    }

    /// `None` for anything that is not a file with a non-empty byte range.
    pub fn parse(text: &str) -> Option<Self> {
        let (file, fragment) = text.split_once('#')?;
        let (offset, length) = fragment.strip_prefix("o=")?.split_once("&l=")?;
        let location = Self {
            file: file.to_string(),
            offset: offset.parse().ok()?,
            length: length.parse().ok()?,
        };
        (!location.file.is_empty() && location.length > 0).then_some(location)
    }
}

/// Where the pack of a scan lives, relative to the archive directory.
pub fn pack_file(scan_id: &str) -> String {
    format!("scans/{scan_id}.pack.zst")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_location_reads_back_as_it_was_composed() {
        let location = Location {
            file: pack_file("abc"),
            offset: 1_048_576,
            length: 1_433,
        };
        assert_eq!(
            location.compose(),
            "scans/abc.pack.zst#o=1048576&l=1433"
        );
        assert_eq!(Location::parse(&location.compose()), Some(location));
    }

    #[test]
    fn the_first_byte_alone_is_a_valid_range() {
        let parsed = Location::parse("requests/o/a/s/b.batch.zst#o=0&l=1").unwrap();
        assert_eq!((parsed.offset, parsed.length), (0, 1));
    }

    #[test]
    fn an_empty_range_a_missing_range_and_garbage_are_refused() {
        for text in [
            "scans/a.pack.zst#o=10&l=0",
            "scans/a.pack.zst",
            "#o=1&l=2",
            "scans/a.pack.zst#l=2&o=1",
            "scans/a.pack.zst#o=-1&l=2",
            "scans/a.pack.zst#o=1&l=two",
        ] {
            assert_eq!(Location::parse(text), None, "{text}");
        }
    }
}
