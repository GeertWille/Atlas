//! Requests the browser pauses for the scanner (the `Fetch` domain), and what
//! is answered. Two things need it, and only document requests are paused
//! for them, so a page's scripts, images and bid requests never wait:
//!
//! - a visit that follows links must stay on the scanned site and must not
//!   load an address the link policy would never have chosen, whatever the
//!   page does with a click: rewrite the link as the pointer goes down,
//!   redirect, submit a form, or navigate by itself later;
//! - a proxy that asks for a login gets it here, over the DevTools
//!   connection, because a command line is readable by other processes.
//!
//! Every other paused request is continued untouched.

use std::collections::HashSet;

use serde_json::{Value, json};
use url::Url;

use super::visitor::links::{Skip, classify_destination};
use super::visitor::site::Site;

/// The custom proxy of the scan and what the scan may tell it.
pub(crate) struct ProxyLogin {
    pub host: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Default)]
pub(crate) struct Interception {
    proxy: Option<ProxyLogin>,
    /// The visitor may follow links: navigations are guarded once the visit
    /// is on its first page.
    follows_links: bool,
    /// Requests whose proxy challenge was answered once already.
    answered: HashSet<String>,
    /// The proxy asked for a login the scan does not have, or refused it.
    proxy_login_failed: bool,
    /// The site itself asked for a login (HTTP authentication).
    site_login_wanted: bool,
    guarding: bool,
    /// How many navigations were refused so far.
    refusals: u32,
}

/// A document request as the browser paused it.
pub(crate) struct DocumentRequest<'a> {
    pub url: &'a str,
    pub method: &'a str,
    /// It would replace the page of its tab, not fill an iframe.
    pub replaces_the_page: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Paused {
    Continue,
    /// Fail the request: the visit stays on the page it is on.
    Refuse(Refusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    OffSite,
    /// Anything but a plain page load: a form being submitted.
    Submission,
    /// An address that names an action on an account or a basket.
    Sensitive,
    File,
}

impl Refusal {
    /// The reason as the audit trail names it, and the sentence that goes
    /// with it. `host` is where the navigation was headed.
    pub(crate) fn report(self, host: &str) -> (&'static str, String) {
        match self {
            Refusal::OffSite => (
                "off-site",
                format!(
                    "A navigation off the scanned site ({host}) was refused; the visit stays on its page"
                ),
            ),
            Refusal::Submission => (
                "form",
                "The page tried to submit a form; that was refused".to_string(),
            ),
            Refusal::Sensitive => (
                "sensitive",
                "A navigation to an address that names an account or basket action was refused"
                    .to_string(),
            ),
            Refusal::File => ("download", "A navigation to a file was refused".to_string()),
        }
    }
}

impl Interception {
    pub(crate) fn new(proxy: Option<ProxyLogin>, follows_links: bool) -> Self {
        Self {
            proxy,
            follows_links,
            ..Self::default()
        }
    }

    /// Parameters of `Fetch.enable`, or `None` when nothing needs pausing.
    pub(crate) fn fetch_enable(&self) -> Option<Value> {
        (self.proxy.is_some() || self.follows_links).then(|| {
            json!({
                "patterns": [{ "resourceType": "Document" }],
                "handleAuthRequests": self.proxy.is_some(),
            })
        })
    }

    /// The visit is on its first page. From now on, for a visitor that
    /// follows links, nothing replaces a page of the visit except a plain
    /// load of an address of the scanned site that the link policy allows.
    pub(crate) fn confine_to_site(&mut self) {
        self.guarding = self.follows_links;
    }

    /// How many navigations were refused so far.
    pub(crate) fn refusals(&self) -> u32 {
        self.refusals
    }

    /// The name of the proxy that would not let the scan in, if one did.
    pub(crate) fn failed_proxy_login(&self) -> Option<&str> {
        let proxy = self.proxy.as_ref()?;
        self.proxy_login_failed.then_some(proxy.host.as_str())
    }

    /// Whether the site, not the proxy, asked for a login.
    pub(crate) fn site_wanted_a_login(&self) -> bool {
        self.site_login_wanted
    }

    /// A document request of the recorded tab is paused; redirects are
    /// paused again, hop by hop. `site` is the scanned site, once the visit
    /// has reached it. A refusal is counted.
    pub(crate) fn on_request_paused(
        &mut self,
        request: &DocumentRequest<'_>,
        site: Option<&Site>,
    ) -> Paused {
        let verdict = self.verdict(request, site);
        if let Paused::Refuse(_) = verdict {
            self.refusals += 1;
        }
        verdict
    }

    /// Whether the visit may load the document.
    pub(crate) fn verdict(&self, request: &DocumentRequest<'_>, site: Option<&Site>) -> Paused {
        let (true, true, Some(site)) = (self.guarding, request.replaces_the_page, site) else {
            return Paused::Continue;
        };
        if !request.method.eq_ignore_ascii_case("GET") {
            return Paused::Refuse(Refusal::Submission);
        }
        match Url::parse(request.url).map(|url| classify_destination(&url, site)) {
            Ok(Ok(())) => Paused::Continue,
            Ok(Err(Skip::Sensitive)) => Paused::Refuse(Refusal::Sensitive),
            Ok(Err(Skip::File)) => Paused::Refuse(Refusal::File),
            Ok(Err(_)) | Err(_) => Paused::Refuse(Refusal::OffSite),
        }
    }

    /// The `authChallengeResponse` to a `Fetch.authRequired` event.
    pub(crate) fn on_auth_required(&mut self, params: &Value) -> Value {
        if params["authChallenge"]["source"] != "Proxy" {
            // A page behind HTTP authentication: as without interception.
            // The proxy's login is never an answer to a site.
            self.site_login_wanted = true;
            return json!({ "response": "Default" });
        }
        let request_id = params["requestId"].as_str().unwrap_or_default();
        let credentials = self
            .proxy
            .as_ref()
            .and_then(|proxy| Some((proxy.username.as_ref()?, proxy.password.as_ref()?)));
        match credentials {
            // Asked again for the same request: the answer was not accepted.
            Some((username, password)) if self.answered.insert(request_id.to_string()) => json!({
                "response": "ProvideCredentials",
                "username": username,
                "password": password,
            }),
            _ => {
                self.proxy_login_failed = true;
                json!({ "response": "CancelAuth" })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(username: Option<&str>, password: Option<&str>) -> Option<ProxyLogin> {
        Some(ProxyLogin {
            host: "proxy.corp.example".into(),
            username: username.map(str::to_string),
            password: password.map(str::to_string),
        })
    }

    fn challenge(request_id: &str, source: &str) -> Value {
        json!({
            "requestId": request_id,
            "authChallenge": { "source": source, "origin": "http://proxy.corp.example:3128", "scheme": "basic", "realm": "corp" },
        })
    }

    #[test]
    fn only_documents_are_paused_and_only_when_something_needs_it() {
        assert_eq!(Interception::new(None, false).fetch_enable(), None);
        assert_eq!(
            Interception::new(None, true).fetch_enable(),
            Some(
                json!({ "patterns": [{ "resourceType": "Document" }], "handleAuthRequests": false })
            )
        );
        assert_eq!(
            Interception::new(proxy(None, None), false).fetch_enable(),
            Some(
                json!({ "patterns": [{ "resourceType": "Document" }], "handleAuthRequests": true })
            )
        );
    }

    fn site() -> Site {
        Site::of(&Url::parse("https://news.example/").unwrap()).unwrap()
    }

    fn load(url: &str) -> DocumentRequest<'_> {
        DocumentRequest {
            url,
            method: "GET",
            replaces_the_page: true,
        }
    }

    #[test]
    fn a_visit_that_follows_links_is_confined_to_the_site_once_it_is_on_its_first_page() {
        let site = site();
        let mut interception = Interception::new(None, true);
        let off_site = "https://partner.example/offer?id=1";
        assert_eq!(
            interception.on_request_paused(&load(off_site), None),
            Paused::Continue,
            "the entry page may redirect where it likes"
        );

        interception.confine_to_site();
        assert_eq!(
            interception.on_request_paused(&load("https://news.example/b"), Some(&site)),
            Paused::Continue
        );
        let in_an_iframe = DocumentRequest {
            replaces_the_page: false,
            ..load("https://ads.example/frame")
        };
        assert_eq!(
            interception.on_request_paused(&in_an_iframe, Some(&site)),
            Paused::Continue,
            "an iframe may load what it likes"
        );
        assert_eq!(interception.refusals(), 0);
        assert_eq!(
            interception.on_request_paused(&load(off_site), Some(&site)),
            Paused::Refuse(Refusal::OffSite)
        );
        assert_eq!(
            interception.on_request_paused(&load("http://news.example/b"), Some(&site)),
            Paused::Refuse(Refusal::OffSite),
            "leaving https is leaving the site"
        );
        assert_eq!(
            interception.on_request_paused(&load("not an address"), Some(&site)),
            Paused::Refuse(Refusal::OffSite)
        );
        assert_eq!(interception.refusals(), 3);
    }

    #[test]
    fn a_confined_visit_loads_no_form_submission_no_action_address_and_no_file() {
        let site = site();
        let mut interception = Interception::new(None, true);
        interception.confine_to_site();
        let submission = DocumentRequest {
            method: "POST",
            ..load("https://news.example/comments")
        };
        assert_eq!(
            interception.on_request_paused(&submission, Some(&site)),
            Paused::Refuse(Refusal::Submission)
        );
        // What a link may have been rewritten to as the pointer went down,
        // or what a followed address may redirect to.
        assert_eq!(
            interception.on_request_paused(&load("https://news.example/logout"), Some(&site)),
            Paused::Refuse(Refusal::Sensitive)
        );
        assert_eq!(
            interception
                .on_request_paused(&load("https://news.example/basket/add?id=3"), Some(&site)),
            Paused::Refuse(Refusal::Sensitive)
        );
        assert_eq!(
            interception.on_request_paused(&load("https://news.example/kit.pdf"), Some(&site)),
            Paused::Refuse(Refusal::File)
        );
        assert_eq!(interception.refusals(), 4);
    }

    #[test]
    fn a_visit_that_follows_no_links_is_never_confined() {
        let site = site();
        let mut interception = Interception::new(proxy(None, None), false);
        interception.confine_to_site();
        assert_eq!(
            interception.on_request_paused(&load("https://partner.example/"), Some(&site)),
            Paused::Continue
        );
        let submission = DocumentRequest {
            method: "POST",
            ..load("https://news.example/comments")
        };
        assert_eq!(
            interception.on_request_paused(&submission, Some(&site)),
            Paused::Continue
        );
    }

    #[test]
    fn every_refusal_has_a_reason_and_a_sentence_for_the_audit_trail() {
        let (reason, message) = Refusal::OffSite.report("partner.example");
        assert_eq!(reason, "off-site");
        assert!(message.contains("partner.example"), "{message}");
        assert_eq!(Refusal::Submission.report("news.example").0, "form");
        assert_eq!(Refusal::Sensitive.report("news.example").0, "sensitive");
        assert_eq!(Refusal::File.report("news.example").0, "download");
    }

    #[test]
    fn a_proxy_challenge_is_answered_once_per_request() {
        let mut interception = Interception::new(proxy(Some("scanner"), Some("s3cret")), false);
        assert_eq!(
            interception.on_auth_required(&challenge("r1", "Proxy")),
            json!({ "response": "ProvideCredentials", "username": "scanner", "password": "s3cret" })
        );
        assert_eq!(interception.failed_proxy_login(), None);
        assert_eq!(
            interception.on_auth_required(&challenge("r2", "Proxy"))["response"],
            "ProvideCredentials",
            "another request may be challenged too"
        );

        assert_eq!(
            interception.on_auth_required(&challenge("r1", "Proxy")),
            json!({ "response": "CancelAuth" }),
            "the proxy did not accept the answer"
        );
        assert_eq!(
            interception.failed_proxy_login(),
            Some("proxy.corp.example")
        );
    }

    #[test]
    fn without_a_user_name_or_a_password_the_challenge_is_cancelled() {
        for login in [
            proxy(None, None),
            proxy(Some("scanner"), None),
            proxy(None, Some("s3cret")),
        ] {
            let mut interception = Interception::new(login, false);
            assert_eq!(
                interception.on_auth_required(&challenge("r1", "Proxy")),
                json!({ "response": "CancelAuth" })
            );
            assert_eq!(
                interception.failed_proxy_login(),
                Some("proxy.corp.example")
            );
        }
    }

    #[test]
    fn a_login_the_site_itself_asks_for_is_left_to_the_browser() {
        let mut interception = Interception::new(proxy(Some("scanner"), Some("s3cret")), false);
        assert_eq!(
            interception.on_auth_required(&challenge("r1", "Server")),
            json!({ "response": "Default" })
        );
        assert_eq!(interception.failed_proxy_login(), None);
        assert!(interception.site_wanted_a_login());
    }
}
