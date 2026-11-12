//! Which addresses belong to the site being scanned. A visit never follows a
//! link away from it, and nothing is recorded on a page outside it.

use url::{Host, Url};

/// The scanned site: a registrable domain ("example.co.uk"), or one exact
/// host and port where there is no such domain (IP addresses, `localhost`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Site {
    identity: Identity,
    secure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Identity {
    Domain(String),
    Origin { host: String, port: u16 },
}

impl Site {
    /// `None` for an address that is not a web page.
    pub(crate) fn of(url: &Url) -> Option<Self> {
        Some(Self {
            identity: identity_of(url)?,
            secure: url.scheme() == "https",
        })
    }

    /// Whether `url` is a page of this site. A plain-http page of a site
    /// visited over https is not: following it would be a downgrade.
    pub(crate) fn contains(&self, url: &Url) -> bool {
        let downgrade = self.secure && url.scheme() != "https";
        !downgrade && identity_of(url).as_ref() == Some(&self.identity)
    }

    /// What the injected tracker compares `location` with: the domain, and
    /// the port when the site is one exact origin.
    pub(crate) fn domain_and_port(&self) -> (&str, Option<u16>) {
        match &self.identity {
            Identity::Domain(domain) => (domain, None),
            Identity::Origin { host, port } => (host, Some(*port)),
        }
    }
}

fn identity_of(url: &Url) -> Option<Identity> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let origin = |host: String| {
        Some(Identity::Origin {
            host,
            port: url.port_or_known_default()?,
        })
    };
    match url.host()? {
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            match psl::domain_str(&name) {
                Some(domain) => Some(Identity::Domain(domain.to_string())),
                None => origin(name),
            }
        }
        Host::Ipv4(address) => origin(address.to_string()),
        Host::Ipv6(address) => origin(format!("[{address}]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn same_site(site: &str, other: &str) -> bool {
        Site::of(&url(site)).unwrap().contains(&url(other))
    }

    #[test]
    fn subdomains_of_one_registrable_domain_are_one_site() {
        assert!(same_site(
            "https://www.news.example/a",
            "https://news.example/b"
        ));
        assert!(same_site(
            "https://news.example/",
            "https://sport.news.example:8443/x"
        ));
        assert!(!same_site("https://news.example/", "https://example/"));
        assert!(!same_site(
            "https://news.example/",
            "https://news.example.evil.test/"
        ));
        assert!(!same_site(
            "https://news.example/",
            "https://notnews.example/"
        ));
    }

    #[test]
    fn the_public_suffix_decides_where_a_site_ends() {
        assert!(same_site(
            "https://www.paper.co.uk/",
            "https://shop.paper.co.uk/"
        ));
        assert!(!same_site(
            "https://www.paper.co.uk/",
            "https://other.co.uk/"
        ));
        assert!(!same_site(
            "https://alice.github.io/",
            "https://bob.github.io/"
        ));
    }

    #[test]
    fn hosts_without_a_registrable_domain_must_match_exactly() {
        assert!(same_site(
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8080/article"
        ));
        assert!(!same_site(
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8081/"
        ));
        assert!(!same_site(
            "http://127.0.0.1:8080/",
            "http://127.0.0.2:8080/"
        ));
        assert!(same_site(
            "http://localhost:3000/",
            "http://localhost:3000/x"
        ));
        assert!(!same_site("http://localhost:3000/", "http://localhost/x"));
        assert!(!same_site(
            "http://localhost:3000/",
            "http://127.0.0.1:3000/"
        ));
        assert!(same_site("http://[::1]:9000/", "http://[::1]:9000/a"));
        assert!(!same_site("http://[::1]:9000/", "http://[::1]:9001/a"));
    }

    #[test]
    fn default_ports_and_letter_case_do_not_matter() {
        assert!(same_site("http://intranet/", "http://INTRANET:80/x"));
        assert!(same_site("https://News.Example./", "https://news.example/"));
    }

    #[test]
    fn leaving_https_is_leaving_the_site() {
        assert!(!same_site("https://news.example/", "http://news.example/a"));
        assert!(same_site("http://news.example/", "https://news.example/a"));
        assert!(same_site("http://news.example/", "http://news.example/a"));
    }

    #[test]
    fn only_web_addresses_are_part_of_a_site() {
        let site = Site::of(&url("https://news.example/")).unwrap();
        for other in [
            "mailto:desk@news.example",
            "javascript:void(0)",
            "data:text/html,hello",
            "blob:https://news.example/1b2c",
            "ftp://news.example/file",
        ] {
            assert!(!site.contains(&url(other)), "{other}");
        }
        assert_eq!(Site::of(&url("about:blank")), None);
    }

    #[test]
    fn the_tracker_is_told_a_domain_or_an_exact_origin() {
        let domain = Site::of(&url("https://www.paper.co.uk/a")).unwrap();
        assert_eq!(domain.domain_and_port(), ("paper.co.uk", None));
        let loopback = Site::of(&url("http://127.0.0.1:4100/")).unwrap();
        assert_eq!(loopback.domain_and_port(), ("127.0.0.1", Some(4100)));
    }
}
