//! How large the answer to a question may be. Chrome's debugging pipe
//! carries a message of any size. The connection to a browser at
//! Browserbase does not: a message of more than about 50 MiB is not
//! refused, the socket is closed and the browser with it, so what could
//! be larger must never be asked for.
//!
//! The size of an answer is not known before it is asked for, only the
//! size of what it carries, so the worst case decides:
//!
//! - A body that is not text comes as base64: four bytes for three.
//! - A body that is text comes as a JSON string, in which the browser
//!   writes every character outside ASCII as `\uXXXX` and every control
//!   character likewise: six bytes for a character that took one to three
//!   (measured on Chromium 153: 6.0 bytes per `é`, per `中` and per U+0001,
//!   12 per emoji, 2 per `"`). So text can take six times its size.
//!
//! Which of the two a body will be is the browser's choice when it
//! answers. Images, media and fonts are never text; anything else may be.

use adscan_model::ResourceKind;

const MIB: u64 = 1024 * 1024;

/// The most one message may hold. The limit was found between 52.00 and
/// 52.75 million bytes; this leaves room below it.
const MAX_REMOTE_REPLY_BYTES: u64 = 48 * MIB;
/// Chrome keeps at most this much of one response for the scanner
/// (`maxResourceBufferSize`), which as base64 is under the limit.
const MAX_REMOTE_BASE64_BODY_BYTES: u64 = 32 * MIB;
/// Bytes one byte, or one UTF-16 unit, of text can take in an answer.
const WORST_BYTES_PER_TEXT_UNIT: u64 = 6;

/// The largest answer the connection to the browser carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyLimit {
    /// The debugging pipe of a local browser.
    Unlimited,
    /// The WebSocket to a browser at Browserbase.
    Remote,
}

impl ReplyLimit {
    /// The largest body, decoded, whose answer is sure to fit.
    fn largest_body(self, kind: ResourceKind) -> Option<u64> {
        match self {
            Self::Unlimited => None,
            Self::Remote if is_never_text(kind) => Some(MAX_REMOTE_BASE64_BODY_BYTES),
            Self::Remote => Some(MAX_REMOTE_REPLY_BYTES / WORST_BYTES_PER_TEXT_UNIT),
        }
    }

    /// Whether the body of a response of `decoded_bytes` may be asked for
    /// with `Network.getResponseBody`.
    pub(crate) fn admits_body(self, kind: ResourceKind, decoded_bytes: u64) -> bool {
        self.largest_body(kind)
            .is_none_or(|largest| decoded_bytes <= largest)
    }

    /// Whether a script source of `utf16_units` may be asked for with
    /// `Debugger.getScriptSource`.
    pub(crate) fn admits_script_source(self, utf16_units: u64) -> bool {
        match self {
            Self::Unlimited => true,
            Self::Remote => utf16_units <= MAX_REMOTE_REPLY_BYTES / WORST_BYTES_PER_TEXT_UNIT,
        }
    }

    /// From how many received bytes on a response that may be text is
    /// asked to be sent along while it loads
    /// (`Network.streamResourceContent`): were it to grow past what can be
    /// asked for, the copy is all there is. Half of that size, so that the
    /// question is on its way in time. The pieces come as base64, each far
    /// below the limit.
    pub(crate) fn copies_bodies_from(self) -> Option<u64> {
        self.largest_body(ResourceKind::Other)
            .map(|largest| largest / 2)
    }
}

fn is_never_text(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Image | ResourceKind::Media | ResourceKind::Font
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_browser_is_asked_for_a_body_of_any_size() {
        let pipe = ReplyLimit::Unlimited;
        assert!(pipe.admits_body(ResourceKind::Script, 500 * MIB));
        assert!(pipe.admits_body(ResourceKind::Image, u64::MAX));
        assert!(pipe.admits_script_source(u64::MAX));
        assert_eq!(pipe.copies_bodies_from(), None);
    }

    #[test]
    fn what_may_be_text_is_asked_for_up_to_a_sixth_of_the_limit() {
        let remote = ReplyLimit::Remote;
        for kind in [
            ResourceKind::Document,
            ResourceKind::Script,
            ResourceKind::Stylesheet,
            ResourceKind::Xhr,
            ResourceKind::Fetch,
            ResourceKind::Other,
        ] {
            assert!(remote.admits_body(kind, 0), "{kind:?}");
            assert!(remote.admits_body(kind, 8 * MIB), "{kind:?}");
            assert!(!remote.admits_body(kind, 8 * MIB + 1), "{kind:?}");
            assert!(!remote.admits_body(kind, 39 * MIB), "{kind:?}");
        }
        assert!(remote.admits_script_source(8 * MIB));
        assert!(!remote.admits_script_source(8 * MIB + 1));
    }

    #[test]
    fn what_is_never_text_is_asked_for_up_to_the_32_mib_that_fit_as_base64() {
        let remote = ReplyLimit::Remote;
        for kind in [ResourceKind::Image, ResourceKind::Media, ResourceKind::Font] {
            assert!(remote.admits_body(kind, 32 * MIB), "{kind:?}");
            assert!(!remote.admits_body(kind, 32 * MIB + 1), "{kind:?}");
        }
    }

    /// The numbers the limits rest on: nothing admitted can make an answer
    /// that reaches the smallest message that was seen to close the socket.
    #[test]
    fn the_worst_answer_to_anything_admitted_stays_under_the_measured_limit() {
        const LARGEST_MESSAGE_THAT_ARRIVED: u64 = 52_000_097;
        const ENVELOPE: u64 = 1_024;
        let worst_text = (MAX_REMOTE_REPLY_BYTES / WORST_BYTES_PER_TEXT_UNIT)
            * WORST_BYTES_PER_TEXT_UNIT
            + ENVELOPE;
        let worst_base64 = MAX_REMOTE_BASE64_BODY_BYTES.div_ceil(3) * 4 + ENVELOPE;

        assert!(worst_text < LARGEST_MESSAGE_THAT_ARRIVED);
        assert!(worst_base64 < LARGEST_MESSAGE_THAT_ARRIVED);
        assert!(MAX_REMOTE_REPLY_BYTES < (crate::cdp::MAX_MESSAGE_BYTES as u64));
        assert_eq!(ReplyLimit::Remote.copies_bodies_from(), Some(4 * MIB));
    }
}
