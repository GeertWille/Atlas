//! Requests the browser pauses for the scanner (the `Fetch` domain), and what
//! is answered. Two things need it, and only document requests are paused
//! for them, so a page's scripts, images and bid requests never wait:
//!
//! - a scripted visitor's click must not take the visit to another site;
//! - a proxy that asks for a login gets it here, over the DevTools
//!   connection, because a command line is readable by other processes.
//!
//! Every other paused request is continued untouched.

use std::collections::HashSet;

use serde_json::{Value, json};

/// The custom proxy of the scan and what the scan may tell it.
pub(crate) struct ProxyLogin {
    pub host: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Default)]
pub(crate) struct Interception {
    proxy: Option<ProxyLogin>,
    /// The visitor may follow links: navigations it causes are guarded.
    follows_links: bool,
    /// Requests whose proxy challenge was answered once already.
    answered: HashSet<String>,
    /// The proxy asked for a login the scan does not have, or refused it.
    proxy_login_failed: bool,
    /// A click of the visitor is under way.
    guarding: bool,
    /// The off-site address the guarded click was refused.
    refused: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Paused {
    Continue,
    /// Fail the request: the visit stays on the page it is on.
    Refuse,
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

    /// From now until [`Self::stop_guarding`], a main-frame navigation to
    /// another site is refused.
    pub(crate) fn guard_navigation(&mut self) {
        self.guarding = true;
        self.refused = None;
    }

    pub(crate) fn stop_guarding(&mut self) {
        self.guarding = false;
    }

    /// The address a guarded navigation was refused, once.
    pub(crate) fn take_refused(&mut self) -> Option<String> {
        self.refused.take()
    }

    /// The name of the proxy that would not let the scan in, if one did.
    pub(crate) fn failed_proxy_login(&self) -> Option<&str> {
        let proxy = self.proxy.as_ref()?;
        self.proxy_login_failed.then_some(proxy.host.as_str())
    }

    /// A document request is paused. `on_site` is whether it stays on the
    /// scanned site; redirects are paused again, hop by hop.
    pub(crate) fn on_request_paused(&mut self, url: &str, main_frame: bool, on_site: bool) -> Paused {
        if self.guarding && main_frame && !on_site {
            self.refused = Some(url.to_string());
            return Paused::Refuse;
        }
        Paused::Continue
    }

    /// The `authChallengeResponse` to a `Fetch.authRequired` event.
    pub(crate) fn on_auth_required(&mut self, params: &Value) -> Value {
        if params["authChallenge"]["source"] != "Proxy" {
            // A page behind HTTP authentication: as without interception.
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
            Some(json!({ "patterns": [{ "resourceType": "Document" }], "handleAuthRequests": false }))
        );
        assert_eq!(
            Interception::new(proxy(None, None), false).fetch_enable(),
            Some(json!({ "patterns": [{ "resourceType": "Document" }], "handleAuthRequests": true }))
        );
    }

    #[test]
    fn a_navigation_off_the_site_is_refused_only_while_a_click_is_guarded() {
        let mut interception = Interception::new(None, true);
        let off_site = "https://partner.example/offer?id=1";
        assert_eq!(interception.on_request_paused(off_site, true, false), Paused::Continue);

        interception.guard_navigation();
        assert_eq!(
            interception.on_request_paused("https://news.example/b", true, true),
            Paused::Continue
        );
        assert_eq!(
            interception.on_request_paused("https://ads.example/frame", false, false),
            Paused::Continue,
            "an iframe may load what it likes"
        );
        assert_eq!(interception.take_refused(), None);
        assert_eq!(interception.on_request_paused(off_site, true, false), Paused::Refuse);
        assert_eq!(interception.take_refused().as_deref(), Some(off_site));
        assert_eq!(interception.take_refused(), None);

        interception.stop_guarding();
        assert_eq!(interception.on_request_paused(off_site, true, false), Paused::Continue);
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
        assert_eq!(interception.failed_proxy_login(), Some("proxy.corp.example"));
    }

    #[test]
    fn without_a_user_name_or_a_password_the_challenge_is_cancelled() {
        for login in [proxy(None, None), proxy(Some("scanner"), None), proxy(None, Some("s3cret"))] {
            let mut interception = Interception::new(login, false);
            assert_eq!(
                interception.on_auth_required(&challenge("r1", "Proxy")),
                json!({ "response": "CancelAuth" })
            );
            assert_eq!(interception.failed_proxy_login(), Some("proxy.corp.example"));
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
    }
}
