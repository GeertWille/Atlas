//! Credentials in headers never reach the disk unless the user asked for
//! full headers: the values of `Cookie`, `Set-Cookie`, `Authorization` and
//! `Proxy-Authorization` are replaced by their length. Cookie names, cookie
//! attributes and the authorization scheme stay, because "was the id cookie
//! sent, and how large was it" is what ad debugging asks.

use adscan_model::{Header, HeaderRedaction, RawHeader};

/// Why a header's value is not stored, if it is not.
pub fn redaction_of(name: &str) -> Option<HeaderRedaction> {
    match name.to_ascii_lowercase().as_str() {
        "cookie" | "set-cookie" => Some(HeaderRedaction::Cookie),
        "authorization" | "proxy-authorization" => Some(HeaderRedaction::Credential),
        _ => None,
    }
}

/// The headers as they are written to the archive: one entry per value
/// (CDP joins repeated headers with a line feed), credentials replaced
/// unless `full`.
pub fn stored_headers(headers: &[RawHeader], full: bool) -> Vec<(String, String)> {
    headers
        .iter()
        .flat_map(|header| {
            header
                .value
                .split('\n')
                .map(move |value| (header.name.as_str(), value))
        })
        .map(|(name, value)| {
            let stored = match redaction_of(name) {
                Some(redaction) if !full => redact(name, value, redaction),
                _ => value.to_string(),
            };
            (name.to_string(), stored)
        })
        .collect()
}

/// Stored headers as the frontend gets them. `redacted` is the mode the
/// archive was written in.
pub fn displayed_headers(stored: &[(String, String)], redacted: bool) -> Vec<Header> {
    stored
        .iter()
        .map(|(name, value)| Header {
            name: name.clone(),
            value: value.clone(),
            redaction: redaction_of(name).filter(|_| redacted),
        })
        .collect()
}

fn redact(name: &str, value: &str, redaction: HeaderRedaction) -> String {
    match redaction {
        HeaderRedaction::Credential => match value.trim().split_once(' ') {
            Some((scheme, credentials)) => format!("{scheme} {}", length_of(credentials.trim())),
            None => length_of(value.trim()),
        },
        HeaderRedaction::Cookie if name.eq_ignore_ascii_case("set-cookie") => {
            let mut parts = value.split(';');
            let cookie = parts.next().map(redact_cookie).unwrap_or_default();
            // What follows the cookie itself are its attributes (Path,
            // Expires, SameSite, ...): no secrets, and often the bug.
            std::iter::once(cookie)
                .chain(parts.map(|attribute| attribute.trim().to_string()))
                .collect::<Vec<_>>()
                .join("; ")
        }
        HeaderRedaction::Cookie => value
            .split(';')
            .map(redact_cookie)
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn redact_cookie(pair: &str) -> String {
    match pair.trim().split_once('=') {
        Some((name, value)) => format!("{}={}", name.trim(), length_of(value)),
        None => length_of(pair.trim()),
    }
}

fn length_of(value: &str) -> String {
    format!("[{} bytes]", value.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, value: &str) -> RawHeader {
        RawHeader {
            name: name.into(),
            value: value.into(),
        }
    }

    fn stored(name: &str, value: &str) -> String {
        stored_headers(&[raw(name, value)], false)
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn cookie_values_become_lengths_and_names_stay() {
        assert_eq!(
            stored("Cookie", "_ga=GA1.2.111.222; khaos=ABCDEF; flag"),
            "_ga=[13 bytes]; khaos=[6 bytes]; [4 bytes]"
        );
    }

    #[test]
    fn a_cookie_value_that_contains_equals_signs_is_one_value() {
        assert_eq!(stored("cookie", "token=a=b=c"), "token=[5 bytes]");
    }

    #[test]
    fn set_cookie_keeps_its_attributes_and_each_cookie_is_its_own_header() {
        let headers = stored_headers(
            &[raw(
                "Set-Cookie",
                "sid=s3cr3t; Path=/; Secure; SameSite=None\nuid=42; Max-Age=3600",
            )],
            false,
        );
        assert_eq!(
            headers,
            vec![
                (
                    "Set-Cookie".to_string(),
                    "sid=[6 bytes]; Path=/; Secure; SameSite=None".to_string()
                ),
                (
                    "Set-Cookie".to_string(),
                    "uid=[2 bytes]; Max-Age=3600".to_string()
                ),
            ]
        );
    }

    #[test]
    fn authorization_keeps_its_scheme_only() {
        assert_eq!(
            stored("Authorization", "Bearer abc.def.ghi"),
            "Bearer [11 bytes]"
        );
        assert_eq!(
            stored("proxy-authorization", "Basic dXNlcjpwYXNz"),
            "Basic [12 bytes]"
        );
        assert_eq!(stored("AUTHORIZATION", "opaque-token"), "[12 bytes]");
    }

    #[test]
    fn header_names_are_compared_without_case() {
        for name in [
            "cookie",
            "COOKIE",
            "Set-Cookie",
            "set-cookie",
            "Authorization",
            "Proxy-Authorization",
        ] {
            assert!(redaction_of(name).is_some(), "{name}");
            assert!(
                !stored(name, "name=secretvalue").contains("secretvalue"),
                "{name}"
            );
        }
    }

    #[test]
    fn other_headers_are_stored_as_sent() {
        for (name, value) in [
            ("User-Agent", "Mozilla/5.0"),
            ("Referer", "https://pub.example/a?b=c"),
            ("x-api-key", "k-123"),
            ("Content-Type", "application/json"),
        ] {
            assert_eq!(stored(name, value), value);
        }
    }

    #[test]
    fn full_mode_stores_every_value_as_sent() {
        let headers = [
            raw("Cookie", "_ga=GA1.2.111.222"),
            raw("Authorization", "Bearer abc"),
        ];
        assert_eq!(
            stored_headers(&headers, true),
            vec![
                ("Cookie".to_string(), "_ga=GA1.2.111.222".to_string()),
                ("Authorization".to_string(), "Bearer abc".to_string()),
            ]
        );
    }

    #[test]
    fn displayed_headers_say_which_values_were_replaced() {
        let stored = stored_headers(&[raw("Cookie", "a=1"), raw("Accept", "*/*")], false);
        let shown = displayed_headers(&stored, true);
        assert_eq!(shown[0].redaction, Some(HeaderRedaction::Cookie));
        assert_eq!(shown[0].value, "a=[1 bytes]");
        assert_eq!(shown[1].redaction, None);
        // An archive written with full headers shows them as they are.
        assert!(
            displayed_headers(&stored, false)
                .iter()
                .all(|header| header.redaction.is_none())
        );
    }
}
