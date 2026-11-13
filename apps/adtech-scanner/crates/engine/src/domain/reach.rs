//! Which addresses a browser in someone else's cloud cannot reach.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::Host;

/// Whether `host` only means something on this machine or its own network:
/// loopback, a private or link-local address, or a name that is never
/// resolved on the public internet.
pub fn is_local_only(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            !domain.contains('.')
                || [".localhost", ".local", ".internal", ".home.arpa"]
                    .iter()
                    .any(|suffix| domain.ends_with(suffix))
        }
        Host::Ipv4(ip) => is_local_v4(*ip),
        Host::Ipv6(ip) => match ip.to_ipv4_mapped() {
            Some(mapped) => is_local_v4(mapped),
            None => is_local_v6(*ip),
        },
    }
}

/// The same question for a host as the user typed it into a proxy profile.
pub fn is_local_only_name(host: &str) -> bool {
    if let Ok(ip) = host.parse::<Ipv6Addr>() {
        return is_local_only(&Host::Ipv6(ip));
    }
    match Host::parse(host) {
        Ok(Host::Domain(domain)) => is_local_only(&Host::Domain(&domain)),
        Ok(Host::Ipv4(ip)) => is_local_only(&Host::Ipv4(ip)),
        Ok(Host::Ipv6(ip)) => is_local_only(&Host::Ipv6(ip)),
        // Not an address anyone can reach.
        Err(_) => true,
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    let carrier_grade_nat = a == 100 && (64..128).contains(&b);
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || carrier_grade_nat
}

fn is_local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    let unique_local = first & 0xfe00 == 0xfc00;
    let link_local = first & 0xffc0 == 0xfe80;
    ip.is_loopback() || ip.is_unspecified() || unique_local || link_local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_of_this_machine_and_its_network_are_local_only() {
        for host in [
            "localhost",
            "app.localhost",
            "printer.local",
            "Build.Internal",
            "nas.home.arpa",
            "intranet",
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.20",
            "169.254.10.1",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.20",
            "not a host",
        ] {
            assert!(is_local_only_name(host), "{host}");
        }
    }

    #[test]
    fn public_names_and_addresses_are_not() {
        for host in [
            "example.com",
            "localhost.example.com",
            "local.example",
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
        ] {
            assert!(!is_local_only_name(host), "{host}");
        }
    }
}
