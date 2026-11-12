//! Which bodies a scan keeps, and the caps on them.

use adscan_model::ResourceKind;

/// A response body is stored up to this many bytes.
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
/// Kept bodies of one scan, before compression. After that requests keep
/// their metadata only.
pub const MAX_SCAN_BODY_BYTES: u64 = 256 * 1024 * 1024;
/// Body bytes that may be admitted but not yet written.
pub const MAX_IN_FLIGHT_BODY_BYTES: u64 = 64 * 1024 * 1024;
/// What an admission reserves when the browser cannot say how large the body is.
pub const UNKNOWN_BODY_RESERVATION: u64 = 256 * 1024;

/// The text classes a scan keeps bodies of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyKind {
    Js,
    Html,
    Json,
    Text,
    Css,
}

impl BodyKind {
    pub fn code(self) -> u8 {
        match self {
            Self::Js => 0,
            Self::Html => 1,
            Self::Json => 2,
            Self::Text => 3,
            Self::Css => 4,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        [Self::Js, Self::Html, Self::Json, Self::Text, Self::Css]
            .into_iter()
            .find(|kind| kind.code() == code)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Js => "js",
            Self::Html => "html",
            Self::Json => "json",
            Self::Text => "text",
            Self::Css => "css",
        }
    }
}

/// The kind a response body is kept as, or `None` when only its metadata is:
/// images, media, fonts and other binary content, and requests that carry
/// no content of their own.
pub fn kept_kind(kind: ResourceKind, mime_type: Option<&str>) -> Option<BodyKind> {
    use ResourceKind::*;
    if matches!(
        kind,
        Image | Media | Font | Ping | Preflight | WebSocket | EventSource
    ) {
        return None;
    }
    let mime = mime_type
        .map(|mime| mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
        .filter(|mime| !mime.is_empty());
    match mime.as_deref() {
        Some(mime) => kind_of_mime(mime),
        // No declared type: trust what the browser loaded it as.
        None => match kind {
            Script => Some(BodyKind::Js),
            Document => Some(BodyKind::Html),
            Stylesheet => Some(BodyKind::Css),
            Xhr | Fetch => Some(BodyKind::Text),
            _ => None,
        },
    }
}

fn kind_of_mime(mime: &str) -> Option<BodyKind> {
    let (family, subtype) = mime.split_once('/')?;
    if matches!(family, "image" | "video" | "audio" | "font") {
        return None;
    }
    if subtype.contains("javascript") || subtype.contains("ecmascript") {
        return Some(BodyKind::Js);
    }
    if subtype == "json" || subtype.ends_with("+json") {
        return Some(BodyKind::Json);
    }
    if subtype == "html" || subtype == "xhtml+xml" {
        return Some(BodyKind::Html);
    }
    if subtype == "css" {
        return Some(BodyKind::Css);
    }
    let textual = family == "text"
        || subtype == "xml"
        || subtype.ends_with("+xml")
        || subtype == "x-www-form-urlencoded";
    textual.then_some(BodyKind::Text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ResourceKind::*;

    #[test]
    fn text_classes_are_kept_by_their_mime_type() {
        for (kind, mime, expected) in [
            (Script, "application/javascript; charset=utf-8", BodyKind::Js),
            (Script, "text/javascript", BodyKind::Js),
            (Other, "application/x-javascript", BodyKind::Js),
            (Document, "text/html", BodyKind::Html),
            (Xhr, "application/json", BodyKind::Json),
            (Fetch, "application/vnd.api+json", BodyKind::Json),
            (Stylesheet, "text/css", BodyKind::Css),
            (Xhr, "text/plain", BodyKind::Text),
            (Fetch, "application/xml", BodyKind::Text),
            (Xhr, "application/vast+xml", BodyKind::Text),
            (Document, "application/xhtml+xml", BodyKind::Html),
        ] {
            assert_eq!(kept_kind(kind, Some(mime)), Some(expected), "{kind:?} {mime}");
        }
    }

    #[test]
    fn images_media_fonts_and_binary_content_are_metadata_only() {
        for (kind, mime) in [
            (Image, Some("image/png")),
            (Image, Some("image/svg+xml")),
            (Media, Some("video/mp4")),
            (Font, Some("font/woff2")),
            (Font, Some("application/octet-stream")),
            // What it was loaded as does not make binary content text.
            (Xhr, Some("image/gif")),
            (Fetch, Some("application/octet-stream")),
            (Fetch, Some("application/wasm")),
            (Other, Some("application/pdf")),
            (Other, Some("audio/mpeg")),
            (Ping, Some("text/plain")),
            (Preflight, Some("text/html")),
            (WebSocket, None),
            (EventSource, Some("text/event-stream")),
            (Other, None),
            (Image, None),
        ] {
            assert_eq!(kept_kind(kind, mime), None, "{kind:?} {mime:?}");
        }
    }

    #[test]
    fn a_response_without_a_type_is_kept_as_what_the_browser_loaded_it_as() {
        assert_eq!(kept_kind(Script, None), Some(BodyKind::Js));
        assert_eq!(kept_kind(Document, Some("")), Some(BodyKind::Html));
        assert_eq!(kept_kind(Xhr, None), Some(BodyKind::Text));
    }

    #[test]
    fn body_kind_codes_round_trip() {
        for kind in [BodyKind::Js, BodyKind::Html, BodyKind::Json, BodyKind::Text, BodyKind::Css] {
            assert_eq!(BodyKind::from_code(kind.code()), Some(kind));
        }
        assert_eq!(BodyKind::from_code(9), None);
    }
}
