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
//! answers. What the page asked for does not settle it alone: Chrome
//! answers from the resource it still holds, where an image is always
//! base64 (measured on Chromium 1243: so were images answered as
//! `image/svg+xml`, `application/json` and `application/octet-stream`),
//! or from its own copy of the bytes, where the MIME type and charset of
//! the response decide and SVG or `text/plain` would come as text (that
//! second path is from reading Chrome's source, not from a measurement).
//! So only a response that was both asked for as an image, media or a
//! font and answered with a MIME type of that kind counts as never text;
//! anything else may be text.
//!
//! The size of what a response carries is counted as it arrives
//! (`Network.dataReceived`), decoded. A response that finished without
//! any of it counted has only its length on the wire, which says little:
//! a compressed body can decode to a thousand times that. Measured on
//! Chromium 1243, a response of any size reports what it received, from
//! the network, the disk cache and the memory cache alike; only one
//! without a body reports nothing. So a response of unknown decoded size
//! is asked for only when its length on the wire is small enough that even
//! the best compression could not have made it too large.

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
/// How many times its length on the wire a body can be once decoded:
/// deflate, and gzip with it, tops out at 1032 to 1. Brotli and zstd can
/// do better on bodies made for it; those are caught by the count of what
/// arrived, which this only stands in for when nothing was counted.
const MOST_A_BODY_GROWS_DECODED: u64 = 1_032;

/// How large the body of a finished response is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodySize {
    /// As counted while it arrived.
    Decoded(u64),
    /// Nothing was counted while it arrived; this is the length of the
    /// whole response on the wire, headers included.
    OnTheWire(u64),
}

/// The largest answer the connection to the browser carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyLimit {
    /// The debugging pipe of a local browser.
    Unlimited,
    /// The WebSocket to a browser at Browserbase.
    Remote,
}

impl ReplyLimit {
    /// The largest body that may be text, decoded, whose answer is sure
    /// to fit.
    fn largest_text_body(self) -> Option<u64> {
        match self {
            Self::Unlimited => None,
            Self::Remote => Some(MAX_REMOTE_REPLY_BYTES / WORST_BYTES_PER_TEXT_UNIT),
        }
    }

    /// The largest body, decoded, whose answer is sure to fit.
    fn largest_body(self, kind: ResourceKind, mime_type: Option<&str>) -> Option<u64> {
        match self {
            Self::Unlimited => None,
            Self::Remote if is_never_text(kind, mime_type) => Some(MAX_REMOTE_BASE64_BODY_BYTES),
            Self::Remote => self.largest_text_body(),
        }
    }

    /// Whether the body of a response may be asked for with
    /// `Network.getResponseBody`. `mime_type` is the response's own.
    pub(crate) fn admits_body(
        self,
        kind: ResourceKind,
        mime_type: Option<&str>,
        size: BodySize,
    ) -> bool {
        let Some(largest) = self.largest_body(kind, mime_type) else {
            return true;
        };
        match size {
            BodySize::Decoded(bytes) => bytes <= largest,
            BodySize::OnTheWire(bytes) => bytes <= largest / MOST_A_BODY_GROWS_DECODED,
        }
    }

    /// Whether a script source of `utf16_units` may be asked for with
    /// `Debugger.getScriptSource`.
    pub(crate) fn admits_script_source(self, utf16_units: u64) -> bool {
        self.largest_text_body()
            .is_none_or(|largest| utf16_units <= largest)
    }

    /// From how many received bytes on a response that may be text is
    /// asked to be sent along while it loads
    /// (`Network.streamResourceContent`): were it to grow past what can be
    /// asked for, the copy is all there is. Half of that size, so that the
    /// question is on its way in time. The pieces come as base64, each far
    /// below the limit.
    pub(crate) fn copies_bodies_from(self) -> Option<u64> {
        self.largest_text_body().map(|largest| largest / 2)
    }
}

/// Whether the browser answers for the body of such a response in base64
/// whatever is in it: it was asked for as an image, media or a font, and
/// its MIME type is one the browser never reads as text. A response
/// without a MIME type may be text.
fn is_never_text(kind: ResourceKind, mime_type: Option<&str>) -> bool {
    let asked_for_as_binary = matches!(
        kind,
        ResourceKind::Image | ResourceKind::Media | ResourceKind::Font
    );
    asked_for_as_binary && mime_type.is_some_and(is_a_binary_mime_type)
}

fn is_a_binary_mime_type(mime_type: &str) -> bool {
    let essence = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let Some((top_level, subtype)) = essence.split_once('/') else {
        return false;
    };
    // SVG is XML, and anything `+xml` or `+json` is read as text.
    let textual_subtype = subtype.ends_with("+xml") || subtype.ends_with("+json");
    matches!(top_level, "image" | "video" | "audio" | "font") && !textual_subtype
}

#[cfg(test)]
mod tests {
    use super::*;

    use BodySize::{Decoded, OnTheWire};

    #[test]
    fn a_local_browser_is_asked_for_a_body_of_any_size() {
        let pipe = ReplyLimit::Unlimited;
        assert!(pipe.admits_body(ResourceKind::Script, None, Decoded(500 * MIB)));
        assert!(pipe.admits_body(ResourceKind::Image, Some("image/png"), Decoded(u64::MAX)));
        assert!(pipe.admits_body(ResourceKind::Script, None, OnTheWire(u64::MAX)));
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
            // Whatever the response calls itself: a fetch of an image may
            // be answered as text.
            for mime_type in [None, Some("text/javascript"), Some("image/png")] {
                let admits = |bytes| remote.admits_body(kind, mime_type, Decoded(bytes));
                assert!(admits(0), "{kind:?} {mime_type:?}");
                assert!(admits(8 * MIB), "{kind:?} {mime_type:?}");
                assert!(!admits(8 * MIB + 1), "{kind:?} {mime_type:?}");
                assert!(!admits(39 * MIB), "{kind:?} {mime_type:?}");
            }
        }
        assert!(remote.admits_script_source(8 * MIB));
        assert!(!remote.admits_script_source(8 * MIB + 1));
    }

    #[test]
    fn what_is_never_text_is_asked_for_up_to_the_32_mib_that_fit_as_base64() {
        let remote = ReplyLimit::Remote;
        for (kind, mime_type) in [
            (ResourceKind::Image, "image/png"),
            (ResourceKind::Image, "IMAGE/WebP"),
            (ResourceKind::Image, "image/jpeg; q=1"),
            (ResourceKind::Media, "video/mp4"),
            (ResourceKind::Media, "audio/mpeg"),
            (ResourceKind::Font, "font/woff2"),
        ] {
            let admits = |bytes| remote.admits_body(kind, Some(mime_type), Decoded(bytes));
            assert!(admits(32 * MIB), "{kind:?} {mime_type}");
            assert!(!admits(32 * MIB + 1), "{kind:?} {mime_type}");
        }
    }

    /// An `<img>` decides what a request is called, not what answers it:
    /// a page can have 30 MiB of text served to an image element.
    #[test]
    fn an_image_media_or_font_request_answered_with_what_may_be_text_gets_the_text_limit() {
        let remote = ReplyLimit::Remote;
        for kind in [ResourceKind::Image, ResourceKind::Media, ResourceKind::Font] {
            for mime_type in [
                None,
                Some(""),
                Some("image/svg+xml"),
                Some("image/svg+xml; charset=utf-8"),
                Some("text/plain"),
                Some("text/html"),
                Some("application/json"),
                Some("application/octet-stream"),
                Some("application/font-woff"),
                Some("video"),
            ] {
                let admits = |bytes| remote.admits_body(kind, mime_type, Decoded(bytes));
                assert!(admits(8 * MIB), "{kind:?} {mime_type:?}");
                assert!(!admits(8 * MIB + 1), "{kind:?} {mime_type:?}");
                assert!(!admits(32 * MIB), "{kind:?} {mime_type:?}");
            }
        }
    }

    /// 5 MiB of gzip can be 100 MiB of text. Without a count of what
    /// arrived, only a response too short on the wire to decode to more
    /// than the limit is asked for.
    #[test]
    fn a_body_of_unknown_decoded_size_is_asked_for_only_when_it_is_small_on_the_wire() {
        let remote = ReplyLimit::Remote;
        let text = |bytes| remote.admits_body(ResourceKind::Script, None, OnTheWire(bytes));
        // No body at all: a 204, a redirect, a cache hit without content.
        assert!(text(0));
        assert!(text(600));
        assert!(text(8 * MIB / 1_032));
        assert!(!text(8 * MIB / 1_032 + 1));
        assert!(!text(5 * MIB));
        // The same number is fine when it is what was counted.
        assert!(remote.admits_body(ResourceKind::Script, None, Decoded(5 * MIB)));

        let image =
            |bytes| remote.admits_body(ResourceKind::Image, Some("image/png"), OnTheWire(bytes));
        assert!(image(32 * MIB / 1_032));
        assert!(!image(32 * MIB / 1_032 + 1));
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
