//! Where a site's ads.txt and an exchange's sellers.json live, and how far a
//! redirect may lead away from there.

use url::{Host, Url};

/// More redirects than any real ads.txt or sellers.json needs.
const MAX_REDIRECTS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdsTxtLocation {
    pub url: String,
    /// The site's registrable domain, or its host when it has none (an IP
    /// address, `localhost`).
    pub site: String,
    /// `scheme://host:port` of the scanned page when it is served from this
    /// machine: the only non-public origin a fetch may contact.
    pub loopback_origin: Option<String>,
}

/// ads.txt is published at the root domain of the site, over https. A page on
/// this machine (the demo site, a local build) is asked at its own origin.
pub fn ads_txt_location(page_url: &str) -> Option<AdsTxtLocation> {
    let page = Url::parse(page_url).ok()?;
    let host = page.host()?;
    if is_loopback(&host) {
        let origin = page.origin().ascii_serialization();
        return Some(AdsTxtLocation {
            url: format!("{origin}/ads.txt"),
            site: host.to_string(),
            loopback_origin: Some(origin),
        });
    }
    let site = root_of(&host.to_string());
    Some(AdsTxtLocation {
        url: format!("https://{site}/ads.txt"),
        site,
        loopback_origin: None,
    })
}

/// The ads.txt of the page's own host, when the root domain's file hands that
/// subdomain over with a `SUBDOMAIN=` declaration.
pub fn subdomain_ads_txt_url(page_url: &str, declared_subdomains: &[String]) -> Option<String> {
    let page = Url::parse(page_url).ok()?;
    let host = page.host_str()?.to_ascii_lowercase();
    declared_subdomains
        .contains(&host)
        .then(|| format!("https://{host}/ads.txt"))
}

pub fn sellers_json_url(ad_system_domain: &str) -> String {
    format!("https://{ad_system_domain}/sellers.json")
}

pub fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            domain == "localhost" || domain.ends_with(".localhost")
        }
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// The redirect rule both specifications share: any number of redirects
/// within the root domain the request started at, and a single one to a
/// destination outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectScope {
    root: String,
    redirects: u32,
    left_root: bool,
}

impl RedirectScope {
    pub fn starting_at(host: &str) -> Self {
        Self {
            root: root_of(host),
            redirects: 0,
            left_root: false,
        }
    }

    /// Records a redirect to `host`, or says why it must not be followed.
    pub fn follow(&mut self, host: &str) -> Result<(), &'static str> {
        self.redirects += 1;
        if self.redirects > MAX_REDIRECTS {
            return Err("too many redirects");
        }
        if root_of(host) != self.root {
            if self.left_root {
                return Err("redirected outside the original domain more than once");
            }
            self.left_root = true;
        }
        Ok(())
    }
}

/// The registrable domain of a host name; an IP address is its own root.
fn root_of(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let is_ip = host
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .is_ok();
    if is_ip {
        return host;
    }
    psl::domain_str(&host).unwrap_or(&host).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ads_txt_lives_at_the_root_domain_over_https() {
        let cases = [
            (
                "https://www.example.com/news/article?id=1",
                "https://example.com/ads.txt",
                "example.com",
            ),
            (
                "http://blog.example.co.uk/",
                "https://example.co.uk/ads.txt",
                "example.co.uk",
            ),
            (
                "https://example.com:8443/x",
                "https://example.com/ads.txt",
                "example.com",
            ),
            (
                "https://93.184.216.34/page",
                "https://93.184.216.34/ads.txt",
                "93.184.216.34",
            ),
        ];
        for (page, url, site) in cases {
            assert_eq!(
                ads_txt_location(page),
                Some(AdsTxtLocation {
                    url: url.into(),
                    site: site.into(),
                    loopback_origin: None,
                }),
                "{page}"
            );
        }
    }

    #[test]
    fn a_page_on_this_machine_is_asked_at_its_own_origin() {
        let cases = [
            ("http://127.0.0.1:49152/", "http://127.0.0.1:49152"),
            ("http://localhost:3000/demo/page", "http://localhost:3000"),
            ("http://[::1]:8080/", "http://[::1]:8080"),
            ("https://app.localhost/", "https://app.localhost"),
        ];
        for (page, origin) in cases {
            let location = ads_txt_location(page).unwrap();
            assert_eq!(location.url, format!("{origin}/ads.txt"), "{page}");
            assert_eq!(location.loopback_origin.as_deref(), Some(origin), "{page}");
        }
        assert_eq!(ads_txt_location("not a url"), None);
        assert_eq!(ads_txt_location("data:text/html,hi"), None);
    }

    #[test]
    fn a_declared_subdomain_has_its_own_file() {
        let declared = vec!["games.example.com".to_string()];
        assert_eq!(
            subdomain_ads_txt_url("https://Games.example.com/play", &declared).as_deref(),
            Some("https://games.example.com/ads.txt")
        );
        assert_eq!(
            subdomain_ads_txt_url("https://www.example.com/", &declared),
            None
        );
    }

    #[test]
    fn redirects_may_leave_the_root_domain_once() {
        let mut scope = RedirectScope::starting_at("appnexus.com");
        assert_eq!(scope.follow("www.appnexus.com"), Ok(()));
        assert_eq!(scope.follow("acdn.adnxs.com"), Ok(()));
        assert_eq!(scope.follow("APPNEXUS.com."), Ok(()));
        assert_eq!(
            scope.follow("cdn.adnxs.com"),
            Err("redirected outside the original domain more than once")
        );

        let mut within = RedirectScope::starting_at("example.co.uk");
        for _ in 0..5 {
            assert_eq!(within.follow("www.example.co.uk"), Ok(()));
        }
        assert_eq!(
            within.follow("www.example.co.uk"),
            Err("too many redirects")
        );

        let mut local = RedirectScope::starting_at("127.0.0.1");
        assert_eq!(local.follow("127.0.0.1"), Ok(()));
        assert!(!local.left_root);
        assert_eq!(local.follow("127.0.0.2"), Ok(()));
        assert_eq!(
            local.follow("other.example"),
            Err("redirected outside the original domain more than once")
        );
    }
}
