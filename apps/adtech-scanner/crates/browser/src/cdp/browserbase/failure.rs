//! What went wrong at Browserbase, in sentences the user is shown as they
//! are. The service answers failures with JSON `{statusCode, error,
//! message}`; nothing but its `message` is ever passed on, cut short and
//! without anything that looks like an address.

use serde_json::Value;

/// A failed call, sorted by what the user can do about it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub(crate) enum ApiFailure {
    /// The key or the project is not accepted.
    #[error("{0}")]
    Auth(String),
    /// HTTP 429: the project is at a limit. Worth waiting for.
    #[error("{0}")]
    Limit(String),
    #[error("{0}")]
    Failed(String),
}

pub(crate) const KEY_NOT_ACCEPTED: &str =
    "Browserbase did not accept the API key. Check it in Settings.";
pub(crate) const PROJECT_NOT_FOUND: &str = "This Browserbase project was not found for this key.";
pub(crate) const CONTEXT_GONE: &str = "The browser profile kept at Browserbase no longer exists. \
                                       Clear it in Settings and scan again.";
pub(crate) const BROWSER_ENDED: &str = "The Browserbase browser has ended.";
pub(crate) const UNEXPECTED_ANSWER: &str =
    "Browserbase answered in a way the scanner does not understand.";

const MAX_SERVER_MESSAGE_CHARS: usize = 200;

/// The server's `message` of a failure body, fit to be shown: at most 200
/// characters, addresses cut down to what comes before their query.
fn server_message(body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|body| body["message"].as_str().map(str::to_string))
        .unwrap_or_default();
    let without_queries: Vec<&str> = message
        .split_whitespace()
        .map(|word| match word.split_once("://") {
            Some(_) => word.split(['?', '#']).next().unwrap_or(word),
            None => word,
        })
        .collect();
    let message = without_queries.join(" ");
    match message.char_indices().nth(MAX_SERVER_MESSAGE_CHARS) {
        Some((cut, _)) => format!("{}…", &message[..cut]),
        None => message,
    }
}

/// `(running, limit)` from "(limit 3, currently 3)".
fn concurrency_of(message: &str) -> Option<(u32, u32)> {
    let number_after = |label: &str| -> Option<u32> {
        message.match_indices(label).find_map(|(at, _)| {
            let digits: String = message[at + label.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
    };
    Some((number_after("currently ")?, number_after("limit ")?))
}

/// The failure an HTTP status and its body stand for. `region` names
/// where a browser was asked for, when one was.
pub(crate) fn from_response(status: u16, body: &str, region: Option<&str>) -> ApiFailure {
    let message = server_message(body);
    let about_the_project = [
        "Unauthorized Project ID",
        "Invalid Project ID",
        "Project not found",
    ]
    .iter()
    .any(|known| message.contains(known));
    match status {
        _ if about_the_project => ApiFailure::Auth(PROJECT_NOT_FOUND.into()),
        401 | 403 => ApiFailure::Auth(KEY_NOT_ACCEPTED.into()),
        429 if message.contains("max concurrent sessions") => {
            let numbers = concurrency_of(&message)
                .map(|(running, limit)| {
                    format!(" ({running} of {limit}, shared by everything that uses this project)")
                })
                .unwrap_or_default();
            ApiFailure::Limit(format!(
                "Browserbase is at its limit of browsers running at once{numbers}. Try again \
                 when one has finished."
            ))
        }
        429 => ApiFailure::Limit(
            "Browserbase is at its limit of new browsers per minute. Try again in a minute.".into(),
        ),
        400 if message.contains("Context ID not found") => ApiFailure::Failed(CONTEXT_GONE.into()),
        400 if message.contains("proxies") => {
            ApiFailure::Failed(format!("Browserbase did not accept the proxy: {message}"))
        }
        400..=499 if message.is_empty() => {
            ApiFailure::Failed(format!("Browserbase refused the request (HTTP {status})."))
        }
        400..=499 => ApiFailure::Failed(format!("Browserbase refused the request: {message}")),
        500..=599 => ApiFailure::Failed(match region {
            Some(region) => format!(
                "Browserbase could not start a browser in {region} right now (HTTP {status})."
            ),
            None => format!("Browserbase is not available right now (HTTP {status})."),
        }),
        _ => ApiFailure::Failed(UNEXPECTED_ANSWER.into()),
    }
}

/// A call that got no answer. `reason` must not hold the address.
pub(crate) fn unreachable(reason: &str) -> ApiFailure {
    ApiFailure::Failed(format!("Browserbase could not be reached: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(status: u16, error: &str, message: &str) -> String {
        serde_json::json!({ "statusCode": status, "error": error, "message": message }).to_string()
    }

    fn failed(message: &str) -> ApiFailure {
        ApiFailure::Failed(message.to_string())
    }

    #[test]
    fn a_refused_key_and_an_unknown_project_are_told_apart() {
        let auth = |message: &str| ApiFailure::Auth(message.to_string());
        assert_eq!(
            from_response(401, &body(401, "Unauthorized", "Unauthorized"), None),
            auth(KEY_NOT_ACCEPTED)
        );
        assert_eq!(from_response(403, "", None), auth(KEY_NOT_ACCEPTED));
        assert_eq!(
            from_response(
                401,
                &body(401, "Unauthorized", "Unauthorized Project ID"),
                None
            ),
            auth(PROJECT_NOT_FOUND)
        );
        assert_eq!(
            from_response(400, &body(400, "Bad Request", "Invalid Project ID"), None),
            auth(PROJECT_NOT_FOUND)
        );
        assert_eq!(
            from_response(404, &body(404, "Not Found", "Project not found"), None),
            auth(PROJECT_NOT_FOUND)
        );
    }

    #[test]
    fn the_concurrency_limit_is_stated_with_the_services_numbers() {
        let measured = "You've exceeded your max concurrent sessions limit (limit 3, currently 3). \
                        Please contact support to increase your limit.";
        assert_eq!(
            from_response(
                429,
                &body(429, "Too Many Requests", measured),
                Some("eu-central-1")
            ),
            ApiFailure::Limit(
                "Browserbase is at its limit of browsers running at once (3 of 3, shared by \
                 everything that uses this project). Try again when one has finished."
                    .into()
            )
        );
        assert_eq!(
            from_response(
                429,
                &body(
                    429,
                    "Too Many Requests",
                    "You've exceeded your max concurrent sessions limit."
                ),
                None
            ),
            ApiFailure::Limit(
                "Browserbase is at its limit of browsers running at once. Try again when one \
                 has finished."
                    .into()
            )
        );
        assert_eq!(
            from_response(
                429,
                &body(429, "Too Many Requests", "Rate limit exceeded"),
                None
            ),
            ApiFailure::Limit(
                "Browserbase is at its limit of new browsers per minute. Try again in a minute."
                    .into()
            )
        );
        assert_eq!(
            from_response(429, "not json", None),
            from_response(429, "", None)
        );
    }

    #[test]
    fn refusals_carry_the_services_reason_as_it_was_measured() {
        assert_eq!(
            from_response(400, &body(400, "Bad Request", "Context ID not found"), None),
            failed(CONTEXT_GONE)
        );
        assert_eq!(
            from_response(
                400,
                &body(
                    400,
                    "Bad Request",
                    "Validation error: Invalid country code at \"proxies[0].geolocation.country\""
                ),
                Some("eu-central-1")
            ),
            failed(
                "Browserbase did not accept the proxy: Validation error: Invalid country code \
                 at \"proxies[0].geolocation.country\""
            )
        );
        assert_eq!(
            from_response(
                400,
                &body(
                    400,
                    "Bad Request",
                    "body/proxies/0/type must be equal to constant"
                ),
                None
            ),
            failed(
                "Browserbase did not accept the proxy: body/proxies/0/type must be equal to \
                 constant"
            )
        );
        assert_eq!(
            from_response(
                400,
                &body(400, "Bad Request", "body/timeout must be >= 60"),
                None
            ),
            failed("Browserbase refused the request: body/timeout must be >= 60")
        );
        assert_eq!(
            from_response(
                403,
                &body(
                    403,
                    "Forbidden",
                    "Verified mode is only available on the Enterprise plan"
                ),
                None
            ),
            ApiFailure::Auth(KEY_NOT_ACCEPTED.into())
        );
        assert_eq!(
            from_response(410, &body(410, "Gone", "Session stopped"), None),
            failed("Browserbase refused the request: Session stopped")
        );
        assert_eq!(
            from_response(404, "<html>not found</html>", None),
            failed("Browserbase refused the request (HTTP 404).")
        );
    }

    #[test]
    fn a_failing_service_names_the_region_a_browser_was_asked_for_in() {
        assert_eq!(
            from_response(503, "", Some("ap-southeast-1")),
            failed("Browserbase could not start a browser in ap-southeast-1 right now (HTTP 503).")
        );
        assert_eq!(
            from_response(500, &body(500, "Internal Server Error", "boom"), None),
            failed("Browserbase is not available right now (HTTP 500).")
        );
        assert_eq!(from_response(302, "", None), failed(UNEXPECTED_ANSWER));
        assert_eq!(
            unreachable("the connection was refused"),
            failed("Browserbase could not be reached: the connection was refused")
        );
    }

    #[test]
    fn the_services_words_are_cut_short_and_lose_the_query_of_any_address() {
        let long = "x".repeat(500);
        let ApiFailure::Failed(message) =
            from_response(400, &body(400, "Bad Request", &long), None)
        else {
            panic!("a refusal");
        };
        assert_eq!(
            message.chars().count(),
            "Browserbase refused the request: ".len() + 200 + 1
        );

        let with_address =
            "Could not reach wss://connect.browserbase.com/s/1?signingKey=s3cret now";
        assert_eq!(
            from_response(400, &body(400, "Bad Request", with_address), None),
            failed(
                "Browserbase refused the request: Could not reach \
                 wss://connect.browserbase.com/s/1 now"
            )
        );
    }
}
