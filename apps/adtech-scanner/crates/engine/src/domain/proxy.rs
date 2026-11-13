//! Proxy profiles: what a saved one may look like, and which proxy a scan
//! ends up using.

use std::net::Ipv6Addr;

use adscan_model::{
    BrowserbaseLocation, CaptureProxy, ProxyChoice, ProxyKind, ProxyProfile, ProxyProfileDraft,
    ProxyScheme, ProxyUse,
};
use url::Host;

use super::DomainError;
use super::reach::is_local_only_name;

const MAX_PROXY_NAME_CHARS: usize = 60;
const MAX_LOCATION_CITY_CHARS: usize = 60;

/// Checks a draft and returns the profile to store under `id`. Names are
/// unique without regard to case, so `others` are the profiles it must not
/// collide with.
pub fn validated_profile(
    draft: ProxyProfileDraft,
    id: String,
    others: &[ProxyProfile],
) -> Result<ProxyProfile, DomainError> {
    let name = draft.name.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_PROXY_NAME_CHARS {
        return Err(DomainError::ProxyNameInvalid {
            max: MAX_PROXY_NAME_CHARS,
        });
    }
    if others
        .iter()
        .any(|other| other.id != id && same_name(&other.name, &name))
    {
        return Err(DomainError::ProxyNameTaken(name));
    }
    if draft.port == 0 {
        return Err(DomainError::ProxyPortInvalid);
    }
    let username = draft
        .username
        .map(|username| username.trim().to_string())
        .filter(|username| !username.is_empty());
    if draft.scheme == ProxyScheme::Socks5 && username.is_some() {
        return Err(DomainError::ProxySocksCredentials);
    }
    Ok(ProxyProfile {
        id,
        name,
        scheme: draft.scheme,
        host: proxy_host(&draft.host)?,
        port: draft.port,
        username,
        has_password: false,
    })
}

pub fn same_name(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

/// A bare host name or IP address. Anything that carries more (a scheme, a
/// port, a path, credentials) is refused instead of silently trimmed, so no
/// password pasted as part of a proxy URL is ever stored.
fn proxy_host(input: &str) -> Result<String, DomainError> {
    let host = input.trim();
    if let Ok(address) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<Ipv6Addr>()
    {
        return Ok(address.to_string());
    }
    let is_plain = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    match Host::parse(host) {
        Ok(Host::Domain(domain)) if is_plain => Ok(domain),
        Ok(Host::Ipv4(address)) if is_plain => Ok(address.to_string()),
        _ => Err(DomainError::ProxyHostInvalid),
    }
}

/// The proxy a scan runs through, in the forms its users need.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProxy {
    /// The choice as stored with the scan: a profile carries a snapshot of
    /// its name and host.
    pub choice: ProxyChoice,
    /// What the browser is started with. `None` with a Browserbase
    /// location: that is `location`.
    pub capture: CaptureProxy,
    /// The residential exit Browserbase is asked for, if any.
    pub location: Option<BrowserbaseLocation>,
    /// What the app's own request for the site's ads.txt leaves through.
    pub fetch: CaptureProxy,
    /// The profile whose password applies, if any.
    pub profile_id: Option<String>,
    /// The system's proxy was asked for a browser that cannot use it; the
    /// scan runs without a proxy.
    pub system_proxy_dropped: bool,
}

impl ResolvedProxy {
    /// For a browser on this machine.
    pub fn resolve(choice: &ProxyChoice, profiles: &[ProxyProfile]) -> Result<Self, DomainError> {
        match choice {
            ProxyChoice::None => Ok(Self::plain(ProxyChoice::None, CaptureProxy::None)),
            ProxyChoice::System => Ok(Self::plain(ProxyChoice::System, CaptureProxy::System)),
            ProxyChoice::Profile { id, .. } => Ok(Self::through(find_profile(id, profiles)?)),
            ProxyChoice::BrowserbaseLocation { .. } => Err(DomainError::LocationNeedsBrowserbase),
        }
    }

    /// For a browser on Browserbase. It cannot use this machine's system
    /// proxy, and a proxy of the user must be reachable from the internet.
    /// The site's ads.txt cannot be fetched through Browserbase's network:
    /// it goes through the user's own proxy when the scan uses one, and
    /// otherwise the way the app's other requests do.
    pub fn resolve_for_browserbase(
        choice: &ProxyChoice,
        profiles: &[ProxyProfile],
    ) -> Result<Self, DomainError> {
        let from_this_machine = |choice, location, system_proxy_dropped| Self {
            choice,
            capture: CaptureProxy::None,
            location,
            fetch: CaptureProxy::System,
            profile_id: None,
            system_proxy_dropped,
        };
        match choice {
            ProxyChoice::None => Ok(from_this_machine(ProxyChoice::None, None, false)),
            ProxyChoice::System => Ok(from_this_machine(ProxyChoice::None, None, true)),
            ProxyChoice::Profile { id, .. } => {
                let profile = find_profile(id, profiles)?;
                if is_local_only_name(&profile.host) {
                    return Err(DomainError::ProxyNotReachableFromCloud(
                        profile.name.clone(),
                    ));
                }
                Ok(Self::through(profile))
            }
            ProxyChoice::BrowserbaseLocation {
                country,
                state,
                city,
            } => {
                let location = validated_location(country, state.as_deref(), city.as_deref())?;
                let choice = ProxyChoice::BrowserbaseLocation {
                    country: location.country.clone(),
                    state: location.state.clone(),
                    city: location.city.clone(),
                };
                Ok(from_this_machine(choice, Some(location), false))
            }
        }
    }

    fn plain(choice: ProxyChoice, route: CaptureProxy) -> Self {
        Self {
            choice,
            capture: route.clone(),
            location: None,
            fetch: route,
            profile_id: None,
            system_proxy_dropped: false,
        }
    }

    fn through(profile: &ProxyProfile) -> Self {
        let route = CaptureProxy::Custom {
            scheme: profile.scheme,
            host: profile.host.clone(),
            port: profile.port,
            username: profile.username.clone(),
        };
        Self {
            choice: ProxyChoice::Profile {
                id: profile.id.clone(),
                name: Some(profile.name.clone()),
                host: Some(profile.host.clone()),
            },
            capture: route.clone(),
            location: None,
            fetch: route,
            profile_id: Some(profile.id.clone()),
            system_proxy_dropped: false,
        }
    }

    /// What is recorded about the proxy with the scan's environment.
    pub fn usage(&self) -> ProxyUse {
        match &self.choice {
            ProxyChoice::None => ProxyUse {
                kind: ProxyKind::None,
                name: None,
                host: None,
            },
            ProxyChoice::System => ProxyUse {
                kind: ProxyKind::System,
                name: None,
                host: None,
            },
            ProxyChoice::Profile { name, host, .. } => ProxyUse {
                kind: ProxyKind::Profile,
                name: name.clone(),
                host: host.clone(),
            },
            ProxyChoice::BrowserbaseLocation {
                country,
                state,
                city,
            } => ProxyUse {
                kind: ProxyKind::BrowserbaseLocation,
                name: Some(
                    [Some(country), state.as_ref(), city.as_ref()]
                        .into_iter()
                        .flatten()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                host: None,
            },
        }
    }
}

fn find_profile<'a>(
    id: &str,
    profiles: &'a [ProxyProfile],
) -> Result<&'a ProxyProfile, DomainError> {
    profiles
        .iter()
        .find(|profile| profile.id == id)
        .ok_or(DomainError::UnknownProxy)
}

/// A place as Browserbase's API takes it: codes in upper case, a city in
/// upper case with underscores for spaces.
fn validated_location(
    country: &str,
    state: Option<&str>,
    city: Option<&str>,
) -> Result<BrowserbaseLocation, DomainError> {
    let code = |input: &str| {
        let code = input.trim().to_ascii_uppercase();
        (code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())).then_some(code)
    };
    fn given(input: Option<&str>) -> Option<&str> {
        input.map(str::trim).filter(|value| !value.is_empty())
    }
    let country = code(country).ok_or(DomainError::LocationCountryInvalid)?;
    let state = given(state)
        .map(|state| code(state).ok_or(DomainError::LocationStateInvalid))
        .transpose()?;
    let city = given(city)
        .map(|city| {
            let city = city
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("_")
                .to_uppercase();
            let is_name = city.chars().count() <= MAX_LOCATION_CITY_CHARS
                && city
                    .chars()
                    .all(|c| c.is_alphabetic() || matches!(c, '_' | '-' | '\'' | '.'));
            is_name
                .then_some(city)
                .ok_or(DomainError::LocationCityInvalid {
                    max: MAX_LOCATION_CITY_CHARS,
                })
        })
        .transpose()?;
    Ok(BrowserbaseLocation {
        country,
        state,
        city,
    })
}

/// The id of the profile a choice names.
pub fn profile_id(choice: &ProxyChoice) -> Option<&str> {
    match choice {
        ProxyChoice::Profile { id, .. } => Some(id),
        ProxyChoice::None | ProxyChoice::System | ProxyChoice::BrowserbaseLocation { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(host: &str) -> ProxyProfileDraft {
        ProxyProfileDraft {
            id: None,
            name: " Belgium ".into(),
            scheme: ProxyScheme::Http,
            host: host.into(),
            port: 8080,
            username: Some(" scanner ".into()),
        }
    }

    fn stored(id: &str, name: &str) -> ProxyProfile {
        ProxyProfile {
            id: id.into(),
            name: name.into(),
            scheme: ProxyScheme::Https,
            host: "be.proxy.example".into(),
            port: 443,
            username: None,
            has_password: false,
        }
    }

    #[test]
    fn a_draft_is_trimmed_into_a_profile() {
        let profile = validated_profile(draft(" be.proxy.example "), "p1".into(), &[]).unwrap();
        assert_eq!(profile.name, "Belgium");
        assert_eq!(profile.host, "be.proxy.example");
        assert_eq!(profile.username.as_deref(), Some("scanner"));
        assert!(!profile.has_password);
    }

    #[test]
    fn hosts_are_bare_names_or_addresses() {
        for (input, expected) in [
            ("10.0.0.7", "10.0.0.7"),
            ("[2001:db8::1]", "2001:db8::1"),
            ("::1", "::1"),
            ("Proxy.Example", "proxy.example"),
        ] {
            let profile = validated_profile(draft(input), "p1".into(), &[]).unwrap();
            assert_eq!(profile.host, expected, "{input}");
        }
        for bad in [
            "",
            "http://proxy.example",
            "proxy.example:8080",
            "user:secret@proxy.example",
            "proxy.example/path",
            "proxy example",
        ] {
            assert_eq!(
                validated_profile(draft(bad), "p1".into(), &[]),
                Err(DomainError::ProxyHostInvalid),
                "{bad}"
            );
        }
    }

    #[test]
    fn names_are_required_bounded_and_unique_without_case() {
        let mut nameless = draft("proxy.example");
        nameless.name = "  ".into();
        assert!(matches!(
            validated_profile(nameless, "p1".into(), &[]),
            Err(DomainError::ProxyNameInvalid { .. })
        ));
        let mut long = draft("proxy.example");
        long.name = "x".repeat(MAX_PROXY_NAME_CHARS + 1);
        assert!(matches!(
            validated_profile(long, "p1".into(), &[]),
            Err(DomainError::ProxyNameInvalid { .. })
        ));

        let others = [stored("p2", "BELGIUM")];
        assert_eq!(
            validated_profile(draft("proxy.example"), "p1".into(), &others),
            Err(DomainError::ProxyNameTaken("Belgium".into()))
        );
        // Saving a profile again under its own name is not a collision.
        assert!(validated_profile(draft("proxy.example"), "p2".into(), &others).is_ok());
    }

    #[test]
    fn port_zero_and_socks_credentials_are_refused() {
        let mut no_port = draft("proxy.example");
        no_port.port = 0;
        assert_eq!(
            validated_profile(no_port, "p1".into(), &[]),
            Err(DomainError::ProxyPortInvalid)
        );
        let mut socks = draft("proxy.example");
        socks.scheme = ProxyScheme::Socks5;
        assert_eq!(
            validated_profile(socks.clone(), "p1".into(), &[]),
            Err(DomainError::ProxySocksCredentials)
        );
        socks.username = Some("  ".into());
        assert!(validated_profile(socks, "p1".into(), &[]).is_ok());
    }

    #[test]
    fn a_profile_choice_resolves_to_its_current_values() {
        let profiles = [stored("p2", "Belgium")];
        let stale = ProxyChoice::Profile {
            id: "p2".into(),
            name: Some("Old name".into()),
            host: None,
        };
        let resolved = ResolvedProxy::resolve(&stale, &profiles).unwrap();
        assert_eq!(
            resolved.choice,
            ProxyChoice::Profile {
                id: "p2".into(),
                name: Some("Belgium".into()),
                host: Some("be.proxy.example".into()),
            }
        );
        assert_eq!(
            resolved.capture,
            CaptureProxy::Custom {
                scheme: ProxyScheme::Https,
                host: "be.proxy.example".into(),
                port: 443,
                username: None,
            }
        );
        assert_eq!(resolved.profile_id.as_deref(), Some("p2"));
        assert_eq!(
            resolved.usage(),
            ProxyUse {
                kind: ProxyKind::Profile,
                name: Some("Belgium".into()),
                host: Some("be.proxy.example".into()),
            }
        );
    }

    #[test]
    fn none_and_system_need_no_profile_and_a_missing_one_is_an_error() {
        assert_eq!(
            ResolvedProxy::resolve(&ProxyChoice::None, &[])
                .unwrap()
                .capture,
            CaptureProxy::None
        );
        assert_eq!(
            ResolvedProxy::resolve(&ProxyChoice::System, &[])
                .unwrap()
                .usage()
                .kind,
            ProxyKind::System
        );
        let gone = ProxyChoice::Profile {
            id: "gone".into(),
            name: None,
            host: None,
        };
        assert_eq!(
            ResolvedProxy::resolve(&gone, &[]),
            Err(DomainError::UnknownProxy)
        );
    }

    #[test]
    fn a_browserbase_location_is_only_for_a_browserbase_browser() {
        let belgium = ProxyChoice::BrowserbaseLocation {
            country: "BE".into(),
            state: None,
            city: None,
        };
        assert_eq!(
            ResolvedProxy::resolve(&belgium, &[]),
            Err(DomainError::LocationNeedsBrowserbase)
        );
        let resolved = ResolvedProxy::resolve_for_browserbase(&belgium, &[]).unwrap();
        assert_eq!(resolved.capture, CaptureProxy::None);
        assert_eq!(resolved.profile_id, None);
        assert_eq!(
            resolved.usage(),
            ProxyUse {
                kind: ProxyKind::BrowserbaseLocation,
                name: Some("BE".into()),
                host: None,
            }
        );
    }

    #[test]
    fn a_location_is_normalised_to_what_browserbase_takes() {
        let place = |country: &str, state: Option<&str>, city: Option<&str>| {
            ResolvedProxy::resolve_for_browserbase(
                &ProxyChoice::BrowserbaseLocation {
                    country: country.into(),
                    state: state.map(str::to_string),
                    city: city.map(str::to_string),
                },
                &[],
            )
        };
        let new_york = place(" us ", Some("ny"), Some("  New   York ")).unwrap();
        assert_eq!(
            new_york.choice,
            ProxyChoice::BrowserbaseLocation {
                country: "US".into(),
                state: Some("NY".into()),
                city: Some("NEW_YORK".into()),
            }
        );
        assert_eq!(new_york.usage().name.as_deref(), Some("US, NY, NEW_YORK"));
        // Blank optional parts are no parts.
        assert_eq!(
            place("de", Some("  "), Some("")).unwrap().location,
            Some(BrowserbaseLocation {
                country: "DE".into(),
                state: None,
                city: None,
            })
        );
        assert_eq!(
            place("de", None, Some("München")).unwrap().usage().name,
            Some("DE, MÜNCHEN".into())
        );

        for country in ["", "B", "BEL", "B1", "België"] {
            assert_eq!(
                place(country, None, None),
                Err(DomainError::LocationCountryInvalid),
                "{country}"
            );
        }
        assert_eq!(
            place("US", Some("New York"), None),
            Err(DomainError::LocationStateInvalid)
        );
        for city in [
            "x".repeat(61),
            "New York; DROP".to_string(),
            "a/b".to_string(),
        ] {
            assert_eq!(
                place("US", None, Some(&city)),
                Err(DomainError::LocationCityInvalid { max: 60 }),
                "{city}"
            );
        }
    }

    #[test]
    fn browserbase_drops_the_system_proxy_and_refuses_a_proxy_it_cannot_reach() {
        let system = ResolvedProxy::resolve_for_browserbase(&ProxyChoice::System, &[]).unwrap();
        assert_eq!(system.choice, ProxyChoice::None);
        assert!(system.system_proxy_dropped);
        assert_eq!(system.usage().kind, ProxyKind::None);
        assert!(
            !ResolvedProxy::resolve_for_browserbase(&ProxyChoice::None, &[])
                .unwrap()
                .system_proxy_dropped
        );

        let choice = ProxyChoice::Profile {
            id: "p2".into(),
            name: None,
            host: None,
        };
        let mut local = stored("p2", "Office");
        for host in [
            "localhost",
            "127.0.0.1",
            "192.168.0.4",
            "proxy.local",
            "::1",
        ] {
            local.host = host.into();
            assert_eq!(
                ResolvedProxy::resolve_for_browserbase(&choice, std::slice::from_ref(&local)),
                Err(DomainError::ProxyNotReachableFromCloud("Office".into())),
                "{host}"
            );
            // The same profile is fine for a browser on this machine.
            assert!(ResolvedProxy::resolve(&choice, std::slice::from_ref(&local)).is_ok());
        }
        let public =
            ResolvedProxy::resolve_for_browserbase(&choice, &[stored("p2", "Belgium")]).unwrap();
        assert_eq!(public.profile_id.as_deref(), Some("p2"));
        assert_eq!(public.fetch, public.capture);
    }
}
