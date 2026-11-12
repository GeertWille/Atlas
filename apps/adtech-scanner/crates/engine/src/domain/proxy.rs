//! Proxy profiles: what a saved one may look like, and which proxy a scan
//! ends up using.

use std::net::Ipv6Addr;

use adscan_model::{
    CaptureProxy, ProxyChoice, ProxyKind, ProxyProfile, ProxyProfileDraft, ProxyScheme, ProxyUse,
};
use url::Host;

use super::DomainError;

const MAX_PROXY_NAME_CHARS: usize = 60;

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

/// The proxy a scan runs through, in the three forms its users need.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProxy {
    /// The choice as stored with the scan: a profile carries a snapshot of
    /// its name and host.
    pub choice: ProxyChoice,
    /// What the browser is started with.
    pub capture: CaptureProxy,
    /// The profile whose password applies, if any.
    pub profile_id: Option<String>,
}

impl ResolvedProxy {
    pub fn resolve(choice: &ProxyChoice, profiles: &[ProxyProfile]) -> Result<Self, DomainError> {
        match choice {
            ProxyChoice::None => Ok(Self {
                choice: ProxyChoice::None,
                capture: CaptureProxy::None,
                profile_id: None,
            }),
            ProxyChoice::System => Ok(Self {
                choice: ProxyChoice::System,
                capture: CaptureProxy::System,
                profile_id: None,
            }),
            ProxyChoice::Profile { id, .. } => {
                let profile = profiles
                    .iter()
                    .find(|profile| &profile.id == id)
                    .ok_or(DomainError::UnknownProxy)?;
                Ok(Self {
                    choice: ProxyChoice::Profile {
                        id: profile.id.clone(),
                        name: Some(profile.name.clone()),
                        host: Some(profile.host.clone()),
                    },
                    capture: CaptureProxy::Custom {
                        scheme: profile.scheme,
                        host: profile.host.clone(),
                        port: profile.port,
                        username: profile.username.clone(),
                    },
                    profile_id: Some(profile.id.clone()),
                })
            }
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
        }
    }
}

/// The id of the profile a choice names.
pub fn profile_id(choice: &ProxyChoice) -> Option<&str> {
    match choice {
        ProxyChoice::Profile { id, .. } => Some(id),
        ProxyChoice::None | ProxyChoice::System => None,
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
}
