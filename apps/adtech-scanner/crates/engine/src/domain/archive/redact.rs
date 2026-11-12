//! What never reaches the disk as it was captured.
//!
//! A scan is a visit to somebody's site, sometimes by a signed-in person,
//! and the archive keeps what the browser sent and received. Three things
//! in it are credentials rather than ad-tech facts, and each is replaced by
//! its length before anything is written:
//!
//! - **Header values** of `Cookie`, `Set-Cookie`, `Authorization` and of any
//!   header whose name says it carries a key, token, secret or signature
//!   (`X-Api-Key`, `X-Auth-Token`, `X-CSRF-Token`, ...). Cookie names and
//!   attributes and the authorization scheme stay, because "was the id
//!   cookie sent, and how large was it" is what ad debugging asks. The
//!   "Store full headers" setting turns this off, except for
//!   `Proxy-Authorization`: that is the app's own proxy password, not the
//!   site's, and is never stored.
//! - **URL parameters** with such a name (`access_token=`, `api_key=`,
//!   `password=`), in every URL the archive holds: the request's, `Referer`,
//!   `Location`, initiators. Always.
//! - **Fields** with such a name in request bodies (forms, JSON, multipart)
//!   and in JSON answers: a login form keeps its user name and loses its
//!   password. Always.
//!
//! Everything else is stored as captured: consent strings, ad-tech ids, the
//! pages themselves.

use std::borrow::Cow;
use std::sync::LazyLock;

use adscan_model::{Header, HeaderRedaction, RawHeader};
use regex::{Captures, Regex};

/// Why a header's value is not stored as sent, when redaction is on.
pub fn redaction_of(name: &str) -> Option<HeaderRedaction> {
    if name.eq_ignore_ascii_case("cookie") || name.eq_ignore_ascii_case("set-cookie") {
        Some(HeaderRedaction::Cookie)
    } else if names_a_credential(name) {
        Some(HeaderRedaction::Credential)
    } else {
        None
    }
}

/// The proxy password the app itself sends: redacted in every mode.
fn is_never_stored(name: &str) -> bool {
    name.eq_ignore_ascii_case("proxy-authorization")
}

/// Headers whose value is an address.
fn holds_a_url(name: &str) -> bool {
    ["referer", "location", "content-location"]
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known))
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
                Some(redaction) if !full || is_never_stored(name) => redact(name, value, redaction),
                _ if holds_a_url(name) => stored_url(value).into_owned(),
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
            redaction: redaction_of(name).filter(|_| redacted || is_never_stored(name)),
        })
        .collect()
}

/// `url` as it is stored: the values of parameters that carry a credential,
/// in the query and in a fragment that is written like one, and a password
/// before the host are replaced by their length.
pub fn stored_url(url: &str) -> Cow<'_, str> {
    let (address, fragment) = match url.split_once('#') {
        Some((address, fragment)) => (address, Some(fragment)),
        None => (url, None),
    };
    let (location, query) = match address.split_once('?') {
        Some((location, query)) => (location, Some(query)),
        None => (address, None),
    };
    let location = without_password(location);
    let query = query.map(stored_parameters);
    let fragment = fragment.map(stored_parameters);
    let unchanged = matches!(location, Cow::Borrowed(_))
        && !matches!(query, Some(Cow::Owned(_)))
        && !matches!(fragment, Some(Cow::Owned(_)));
    if unchanged {
        return Cow::Borrowed(url);
    }
    let mut stored = location.into_owned();
    if let Some(query) = query {
        stored.push('?');
        stored.push_str(&query);
    }
    if let Some(fragment) = fragment {
        stored.push('#');
        stored.push_str(&fragment);
    }
    Cow::Owned(stored)
}

/// `scheme://user:password@host/...` without the password.
fn without_password(location: &str) -> Cow<'_, str> {
    let Some((scheme, rest)) = location.split_once("://") else {
        return Cow::Borrowed(location);
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return Cow::Borrowed(location);
    };
    match rest[..at].split_once(':') {
        Some((user, password)) if !password.is_empty() => Cow::Owned(format!(
            "{scheme}://{user}:{}{}",
            url_placeholder(password),
            &rest[at..]
        )),
        _ => Cow::Borrowed(location),
    }
}

/// `a=1&token=abc` with the credentials among its values replaced.
fn stored_parameters(parameters: &str) -> Cow<'_, str> {
    let carries_one = |pair: &str| {
        pair.split_once('=')
            .is_some_and(|(name, value)| !value.is_empty() && names_a_credential(name))
    };
    if !parameters.split('&').any(carries_one) {
        return Cow::Borrowed(parameters);
    }
    let stored: Vec<Cow<'_, str>> = parameters
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) if carries_one(pair) => {
                Cow::Owned(format!("{name}={}", url_placeholder(value)))
            }
            _ => Cow::Borrowed(pair),
        })
        .collect();
    Cow::Owned(stored.join("&"))
}

/// A request body as it is stored: the values of fields that carry a
/// credential replaced, whether the body is a form, JSON or multipart.
/// Works on what is there, so a body that was cut off is treated the same.
pub fn stored_request_body(text: &str) -> Cow<'_, str> {
    if !MENTIONS_A_CREDENTIAL.is_match(text) {
        return Cow::Borrowed(text);
    }
    let stored = stored_json(text);
    let stored = replace_all(stored, &FORM_PAIR, |found| {
        credential_field(&found[2], &found[3]).then(|| {
            format!("{}{}={}", &found[1], &found[2], length_of(&found[3]))
        })
    });
    replace_all(stored, &MULTIPART_FIELD, |found| {
        credential_field(&found[2], &found[3])
            .then(|| format!("{}{}", &found[1], length_of(&found[3])))
    })
}

/// A JSON answer as it is stored: the string and number values of fields
/// that carry a credential replaced (`"access_token":"[212 bytes]"`).
pub fn stored_json(text: &str) -> Cow<'_, str> {
    if !MENTIONS_A_CREDENTIAL.is_match(text) {
        return Cow::Borrowed(text);
    }
    replace_all(Cow::Borrowed(text), &JSON_PAIR, |found| {
        let value = found.get(3).or(found.get(4))?.as_str();
        credential_field(&found[2], value)
            .then(|| format!("{}\"{}\"", &found[1], length_of(value)))
    })
}

fn credential_field(name: &str, value: &str) -> bool {
    !value.is_empty() && names_a_credential(name)
}

/// `text` with every match for which `replacement` has one replaced.
fn replace_all<'a>(
    text: Cow<'a, str>,
    pattern: &Regex,
    replacement: impl Fn(&Captures) -> Option<String>,
) -> Cow<'a, str> {
    let mut changed = false;
    let replaced = pattern.replace_all(&text, |found: &Captures| {
        replacement(found).map_or_else(
            || found[0].to_string(),
            |replaced| {
                changed = true;
                replaced
            },
        )
    });
    if changed {
        Cow::Owned(replaced.into_owned())
    } else {
        text
    }
}

/// A cheap first look: nearly every body mentions none of these and is
/// stored without being searched field by field.
static MENTIONS_A_CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)passw|pwd|passphrase|secret|token|jwt|api.?key|auth|signature|csrf|xsrf|otp|cvv|cvc|cookie|private.?key|access.?key")
        .expect("a valid pattern")
});

/// `"name": "value"` or `"name": 123`, also inside JSON that is itself a
/// string (`\"name\":\"value\"`).
static JSON_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(\\?"([A-Za-z0-9_.\-\[\] ]{1,64})\\?"\s*:\s*)(?:\\?"((?:[^"\\]|\\[^"])*)\\?"|(-?[0-9][0-9.eE+\-]*))"#,
    )
    .expect("a valid pattern")
});

/// `name=value` as forms and query strings write it, wherever it stands.
static FORM_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(^|[?&;\s"'])([A-Za-z0-9_.\-\[\]%]{1,64})=([^&\s"'<>]*)"#)
        .expect("a valid pattern")
});

/// One part of a `multipart/form-data` body: its headers, then its value.
static MULTIPART_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(content-disposition:[^\r\n]*\bname="([^"\r\n]{1,64})"[^\r\n]*\r?\n(?:[^\r\n]+\r?\n)*\r?\n)([^\r\n]*)"#)
        .expect("a valid pattern")
});

/// Whether a header, parameter or field of this name carries a credential.
/// Names are read word by word (`X-Api-Key`, `access_token`, `csrfToken`),
/// so that `:authority` or `Sec-WebSocket-Key` are not taken for one.
pub fn names_a_credential(name: &str) -> bool {
    const WORDS: &[&str] = &[
        "password",
        "passwd",
        "pwd",
        "passphrase",
        "secret",
        "token",
        "jwt",
        "apikey",
        "auth",
        "authorization",
        "signature",
        "csrf",
        "xsrf",
        "otp",
        "cvv",
        "cvc",
    ];
    /// What makes the word "key" a credential when it stands before it.
    const KEY_OF: &[&str] = &["api", "secret", "private", "access", "subscription"];
    let words = words_of(name);
    words.iter().enumerate().any(|(index, word)| {
        WORDS.contains(&word.as_str())
            || (word == "key" && index > 0 && KEY_OF.contains(&words[index - 1].as_str()))
    })
}

/// The words of a name in lower case: split at anything that is not a
/// letter or digit and where a capital follows a small letter.
fn words_of(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut previous_was_lower = false;
    for character in name.chars() {
        let starts_a_word = character.is_ascii_uppercase() && previous_was_lower;
        if (!character.is_ascii_alphanumeric() || starts_a_word) && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if character.is_ascii_alphanumeric() {
            word.push(character.to_ascii_lowercase());
        }
        previous_was_lower = character.is_ascii_lowercase();
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Schemes of `Authorization` that are kept beside the length; anything
/// else before a space may be part of the credential itself.
const AUTHORIZATION_SCHEMES: &[&str] = &[
    "basic",
    "bearer",
    "digest",
    "negotiate",
    "ntlm",
    "hawk",
    "oauth",
    "token",
    "aws4-hmac-sha256",
];

fn redact(name: &str, value: &str, redaction: HeaderRedaction) -> String {
    match redaction {
        HeaderRedaction::Credential => match value.trim().split_once(' ') {
            Some((scheme, credentials))
                if AUTHORIZATION_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) =>
            {
                format!("{scheme} {}", length_of(credentials.trim()))
            }
            _ => length_of(value.trim()),
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

/// The placeholder inside a URL, where a space would not survive.
fn url_placeholder(value: &str) -> String {
    format!("[redacted-{}-bytes]", value.len())
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
