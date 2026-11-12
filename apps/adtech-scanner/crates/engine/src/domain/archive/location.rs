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
}

/// Where the pack of a scan lives, relative to the archive directory.
pub fn pack_file(scan_id: &str) -> String {
    format!("scans/{scan_id}.pack.zst")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_location_is_composed_in_the_data_lakes_syntax() {
        let location = Location {
            file: pack_file("abc"),
            offset: 1_048_576,
            length: 1_433,
        };
        assert_eq!(location.compose(), "scans/abc.pack.zst#o=1048576&l=1433");
        let first_byte = Location {
            file: "requests/o/a/s/b.batch.zst".into(),
            offset: 0,
            length: 1,
        };
        assert_eq!(first_byte.compose(), "requests/o/a/s/b.batch.zst#o=0&l=1");
    }
}
