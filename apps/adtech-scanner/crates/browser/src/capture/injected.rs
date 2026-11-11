//! The scripts evaluated inside the scanned page and the names they share with Rust.

/// Runs before any page script in the top frame and reports ad-stack activity.
pub(crate) const TRACKER_SOURCE: &str = include_str!("../../injected/tracker.js");

/// Finds and clicks the "accept all" control of a consent banner. Evaluates to
/// the name of the banner it accepted, or `null`.
pub(crate) const CONSENT_SOURCE: &str = include_str!("../../injected/consent.js");

/// CDP binding the tracker sends its batches through.
pub(crate) const TRACKER_BINDING: &str = "__adscan_emit";

/// DOM event that makes the tracker send what it has buffered.
pub(crate) const TRACKER_FLUSH_EVENT: &str = "__adscan_flush";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_uses_the_names_rust_listens_for() {
        assert!(TRACKER_SOURCE.contains(&format!("'{TRACKER_BINDING}'")));
        assert!(TRACKER_SOURCE.contains(&format!("'{TRACKER_FLUSH_EVENT}'")));
    }
}
