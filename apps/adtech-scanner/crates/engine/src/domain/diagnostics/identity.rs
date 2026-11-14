//! How the app's own requests for a site's public files (ads.txt,
//! sellers.json) present themselves: as the browser of the scan did, so a
//! site treats them like the visit. Sites and their firewalls refuse clients
//! they do not know, and an app's own name is one of those (found on
//! `www.mediaimpact.de/sellers.json`, which answers it with a block page).
//! One identity per scan; nothing rotates.

/// A desktop Chrome on a Mac, for requests with no scan whose browser could
/// be copied (a scan whose capture reported no user agent). Chrome's reduced
/// user agent string: only the major version is real, and it is not checked
/// against the release calendar here.
pub const FALLBACK_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
/// What Chrome asks for when a person opens an address in a tab.
pub const BROWSER_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,\
     image/avif,image/webp,image/apng,*/*;q=0.8";
const FALLBACK_LANGUAGE: &str = "en-US";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserIdentity {
    pub user_agent: String,
    pub accept_language: String,
}

impl BrowserIdentity {
    /// The identity of a scan whose browser reported `user_agent` and,
    /// where known, `navigator.language`. A headless Chrome names itself
    /// `HeadlessChrome`, which no person's browser does; that word is the
    /// one thing changed.
    pub fn of_scan(user_agent: &str, language: Option<&str>) -> Self {
        let user_agent = user_agent.trim();
        Self {
            user_agent: if user_agent.is_empty() {
                FALLBACK_USER_AGENT.to_string()
            } else {
                user_agent.replace("HeadlessChrome/", "Chrome/")
            },
            accept_language: accept_language(language.unwrap_or(FALLBACK_LANGUAGE)),
        }
    }
}

impl Default for BrowserIdentity {
    fn default() -> Self {
        Self::of_scan("", None)
    }
}

/// `Accept-Language` as Chrome sends it for one preferred language:
/// `nl-BE` becomes `nl-BE,nl;q=0.9`.
fn accept_language(language: &str) -> String {
    let language = language.trim();
    let language = if language.is_empty()
        || !language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        FALLBACK_LANGUAGE
    } else {
        language
    };
    match language.split_once('-') {
        Some((primary, _)) => format!("{language},{primary};q=0.9"),
        None => language.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scan_lends_its_browser_and_the_rest_is_a_desktop_chrome() {
        let scan = BrowserIdentity::of_scan(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/153.0.0.0 Safari/537.36",
            Some("nl-BE"),
        );
        assert_eq!(
            scan.user_agent,
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36"
        );
        assert_eq!(scan.accept_language, "nl-BE,nl;q=0.9");

        let unknown = BrowserIdentity::default();
        assert_eq!(unknown.user_agent, FALLBACK_USER_AGENT);
        assert!(unknown.user_agent.contains("Chrome/") && !unknown.user_agent.contains("adtech"));
        assert_eq!(unknown.accept_language, "en-US,en;q=0.9");
        assert_eq!(
            BrowserIdentity::of_scan("  ", Some("fr")).accept_language,
            "fr"
        );
        assert_eq!(
            BrowserIdentity::of_scan("x", Some("en\r\nX-Evil: 1")).accept_language,
            "en-US,en;q=0.9"
        );
    }
}
