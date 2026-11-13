//! What is asked of Browserbase when a session is created. Unknown fields
//! are refused by the service, and a default of the service must not be
//! able to change a scan, so the body is spelled out in full here and
//! nowhere else.

use adscan_model::{BrowserbaseLocation, BrowserbaseRegion, BrowserbaseSessionSettings};
use serde_json::{Map, Value, json};

/// The shortest and longest time limit the service takes for a session.
pub(crate) const MIN_TIMEOUT_SECONDS: u64 = 60;
pub(crate) const MAX_TIMEOUT_SECONDS: u64 = 21_600;

/// The `userMetadata.app` of every session this app starts.
pub(crate) const APP_TAG: &str = "adscan";

/// Where the session's traffic leaves from.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Exit {
    /// The cloud region itself.
    Region,
    /// The user's own proxy, which Browserbase signs in to.
    External {
        /// `scheme://host:port`
        server: String,
        username: Option<String>,
        password: Option<String>,
    },
    /// A residential exit of Browserbase.
    Location(BrowserbaseLocation),
}

impl Exit {
    /// The name of the kind, as the audit trail has it.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Region => "none",
            Self::External { .. } => "external",
            Self::Location(_) => "browserbase",
        }
    }
}

/// One session to ask for. Holds a proxy password, so it has no `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionOrder {
    pub region: BrowserbaseRegion,
    /// Clamped to what the service takes when the body is made.
    pub timeout_seconds: u64,
    pub settings: BrowserbaseSessionSettings,
    pub exit: Exit,
    /// The context that holds the kept browser profile.
    pub context_id: Option<String>,
    pub install_id: String,
    /// The scan's id, or `probe`.
    pub scan_id: String,
}

impl SessionOrder {
    /// The time limit the service is given.
    pub(crate) fn timeout(&self) -> u64 {
        self.timeout_seconds
            .clamp(MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS)
    }

    /// The body of `POST /v1/sessions`.
    pub(crate) fn body(&self, project_id: &str) -> Value {
        let settings = &self.settings;
        let mut browser_settings = json!({
            "solveCaptchas": settings.solve_captchas,
            "recordSession": settings.record_session,
            "logSession": settings.log_session,
            "blockAds": settings.block_ads,
        });
        if let Some(context) = &self.context_id {
            browser_settings["context"] = json!({ "id": context, "persist": true });
        }
        let mut body = json!({
            "projectId": project_id,
            "region": self.region.as_str(),
            "timeout": self.timeout(),
            "keepAlive": settings.keep_alive,
            "browserSettings": browser_settings,
            "userMetadata": {
                "app": APP_TAG,
                "install": self.install_id,
                "scan": self.scan_id,
            },
        });
        match &self.exit {
            Exit::Region => {}
            Exit::External {
                server,
                username,
                password,
            } => {
                let mut proxy = Map::new();
                proxy.insert("type".into(), json!("external"));
                proxy.insert("server".into(), json!(server));
                if let Some(username) = username {
                    proxy.insert("username".into(), json!(username));
                }
                if let Some(password) = password {
                    proxy.insert("password".into(), json!(password));
                }
                body["proxies"] = json!([proxy]);
            }
            Exit::Location(location) => {
                let mut geolocation = Map::new();
                geolocation.insert("country".into(), json!(location.country));
                if let Some(state) = &location.state {
                    geolocation.insert("state".into(), json!(state));
                }
                if let Some(city) = &location.city {
                    geolocation.insert("city".into(), json!(city));
                }
                body["proxies"] = json!([{ "type": "browserbase", "geolocation": geolocation }]);
            }
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = "3f8a1c52-6e0b-4b7d-9a43-5d2c8e9f1a07";

    fn order() -> SessionOrder {
        SessionOrder {
            region: BrowserbaseRegion::EuCentral1,
            timeout_seconds: 240,
            settings: BrowserbaseSessionSettings::STANDARD,
            exit: Exit::Region,
            context_id: None,
            install_id: "0123456789abcdef0123456789abcdef".into(),
            scan_id: "7c9e6679-7425-40de-944b-e07fc1f90ae7".into(),
        }
    }

    fn location(country: &str, state: Option<&str>, city: Option<&str>) -> Exit {
        Exit::Location(BrowserbaseLocation {
            country: country.into(),
            state: state.map(str::to_string),
            city: city.map(str::to_string),
        })
    }

    #[test]
    fn a_plain_session_is_asked_for_with_every_setting_stated_and_nothing_else() {
        assert_eq!(
            order().body(PROJECT),
            json!({
                "projectId": PROJECT,
                "region": "eu-central-1",
                "timeout": 240,
                "keepAlive": false,
                "browserSettings": {
                    "solveCaptchas": true,
                    "recordSession": true,
                    "logSession": true,
                    "blockAds": false,
                },
                "userMetadata": {
                    "app": "adscan",
                    "install": "0123456789abcdef0123456789abcdef",
                    "scan": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
                },
            })
        );
    }

    #[test]
    fn the_settings_are_sent_as_given_field_by_field() {
        let reversed = BrowserbaseSessionSettings {
            solve_captchas: false,
            record_session: false,
            log_session: false,
            block_ads: true,
            keep_alive: true,
        };
        let body = SessionOrder {
            settings: reversed,
            ..order()
        }
        .body(PROJECT);

        assert_eq!(body["keepAlive"], true);
        assert_eq!(
            body["browserSettings"],
            json!({
                "solveCaptchas": false,
                "recordSession": false,
                "logSession": false,
                "blockAds": true,
            })
        );
    }

    #[test]
    fn every_region_is_named_as_the_service_names_it() {
        for (region, name) in [
            (BrowserbaseRegion::UsWest2, "us-west-2"),
            (BrowserbaseRegion::UsEast1, "us-east-1"),
            (BrowserbaseRegion::EuCentral1, "eu-central-1"),
            (BrowserbaseRegion::ApSoutheast1, "ap-southeast-1"),
        ] {
            let body = SessionOrder { region, ..order() }.body(PROJECT);
            assert_eq!(body["region"], name);
        }
    }

    #[test]
    fn a_kept_profile_is_a_context_that_persists() {
        let body = SessionOrder {
            context_id: Some("c0ffee00-0000-4000-8000-000000000001".into()),
            ..order()
        }
        .body(PROJECT);

        assert_eq!(
            body["browserSettings"]["context"],
            json!({ "id": "c0ffee00-0000-4000-8000-000000000001", "persist": true })
        );
        assert_eq!(body["browserSettings"].as_object().unwrap().len(), 5);
        assert!(
            order().body(PROJECT)["browserSettings"]
                .get("context")
                .is_none()
        );
    }

    #[test]
    fn the_users_own_proxy_is_an_external_proxy_with_the_login_it_has() {
        let with = |username: Option<&str>, password: Option<&str>| {
            SessionOrder {
                exit: Exit::External {
                    server: "http://proxy.example.net:3128".into(),
                    username: username.map(str::to_string),
                    password: password.map(str::to_string),
                },
                ..order()
            }
            .body(PROJECT)
        };

        assert_eq!(with(Some("scanner"), Some("pa55")), {
            let mut expected = order().body(PROJECT);
            expected["proxies"] = json!([{
                "type": "external",
                "server": "http://proxy.example.net:3128",
                "username": "scanner",
                "password": "pa55",
            }]);
            expected
        });
        assert_eq!(
            with(None, None)["proxies"],
            json!([{ "type": "external", "server": "http://proxy.example.net:3128" }])
        );
        assert_eq!(
            with(Some("scanner"), None)["proxies"],
            json!([{
                "type": "external",
                "server": "http://proxy.example.net:3128",
                "username": "scanner",
            }])
        );
    }

    #[test]
    fn a_browserbase_location_is_a_browserbase_proxy_with_only_what_was_chosen() {
        let proxies = |exit: Exit| SessionOrder { exit, ..order() }.body(PROJECT)["proxies"].take();

        assert_eq!(
            proxies(location("BE", None, None)),
            json!([{ "type": "browserbase", "geolocation": { "country": "BE" } }])
        );
        assert_eq!(
            proxies(location("US", Some("NY"), None)),
            json!([{ "type": "browserbase", "geolocation": { "country": "US", "state": "NY" } }])
        );
        assert_eq!(
            proxies(location("US", Some("NY"), Some("NEW_YORK"))),
            json!([{
                "type": "browserbase",
                "geolocation": { "country": "US", "state": "NY", "city": "NEW_YORK" },
            }])
        );
        assert!(order().body(PROJECT).get("proxies").is_none());
    }

    /// Every combination: the top level never has a key the service does
    /// not know, and never one of the settings this account is refused.
    #[test]
    fn no_combination_sends_a_field_the_service_refuses() {
        const TOP_LEVEL: [&str; 7] = [
            "projectId",
            "region",
            "timeout",
            "keepAlive",
            "browserSettings",
            "proxies",
            "userMetadata",
        ];
        const SETTINGS: [&str; 5] = [
            "solveCaptchas",
            "recordSession",
            "logSession",
            "blockAds",
            "context",
        ];
        let exits = [
            Exit::Region,
            Exit::External {
                server: "socks5://proxy.example.net:1080".into(),
                username: None,
                password: None,
            },
            location("DE", None, Some("BERLIN")),
        ];
        for exit in exits {
            for context_id in [None, Some("context".to_string())] {
                for region in BrowserbaseRegion::ALL {
                    let body = SessionOrder {
                        region,
                        exit: exit.clone(),
                        context_id: context_id.clone(),
                        ..order()
                    }
                    .body(PROJECT);
                    for key in body.as_object().unwrap().keys() {
                        assert!(TOP_LEVEL.contains(&key.as_str()), "{key}");
                    }
                    for key in body["browserSettings"].as_object().unwrap().keys() {
                        assert!(SETTINGS.contains(&key.as_str()), "{key}");
                    }
                    let metadata = body["userMetadata"].as_object().unwrap();
                    assert_eq!(metadata.len(), 3);
                    assert!(metadata.values().all(Value::is_string));
                    assert!(body["userMetadata"].to_string().len() <= 512);
                }
            }
        }
    }

    #[test]
    fn the_time_limit_is_kept_within_what_the_service_takes() {
        let timeout = |seconds| {
            SessionOrder {
                timeout_seconds: seconds,
                ..order()
            }
            .body(PROJECT)["timeout"]
                .take()
        };
        assert_eq!(timeout(0), 60);
        assert_eq!(timeout(59), 60);
        assert_eq!(timeout(60), 60);
        assert_eq!(timeout(1_080), 1_080);
        assert_eq!(timeout(21_600), 21_600);
        assert_eq!(timeout(999_999), 21_600);
    }

    #[test]
    fn the_kind_of_exit_is_named_without_its_details() {
        assert_eq!(Exit::Region.kind(), "none");
        assert_eq!(location("BE", None, None).kind(), "browserbase");
        let external = Exit::External {
            server: "http://proxy.example.net:3128".into(),
            username: None,
            password: Some("pa55".into()),
        };
        assert_eq!(external.kind(), "external");
    }
}
