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
        Host::Ipv6(ip) => match embedded_v4(*ip) {
            Some(embedded) => is_local_v4(embedded),
            None => is_local_v6(*ip),
        },
    }
}

/// The IPv4 address an IPv6 address stands for, when it is one of the
/// forms that carry one: IPv4-mapped (`::ffff:a.b.c.d`), the old
/// IPv4-compatible form (`::a.b.c.d`, which covers `::` and `::1`), NAT64
/// (`64:ff9b::a.b.c.d`) and 6to4 (`2002:aabb:ccdd::/48`). Whoever routes
/// such an address ends up at the IPv4 address inside it.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let v4 = |high: u16, low: u16| Ipv4Addr::from((u32::from(high) << 16) | u32::from(low));
    match segments {
        [0, 0, 0, 0, 0, 0xffff, high, low] => Some(v4(high, low)),
        [0, 0, 0, 0, 0, 0, high, low] => Some(v4(high, low)),
        [0x64, 0xff9b, 0, 0, 0, 0, high, low] => Some(v4(high, low)),
        [0x2002, high, low, ..] => Some(v4(high, low)),
        _ => None,
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
    // 0.0.0.0/8: "this network". Linux takes 0.0.0.0 for this machine.
    let this_network = a == 0;
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || this_network
        || carrier_grade_nat
}

fn is_local_v6(ip: Ipv6Addr) -> bool {
    let [first, second, ..] = ip.segments();
    let unique_local = first & 0xfe00 == 0xfc00;
    let link_local = first & 0xffc0 == 0xfe80;
    // 64:ff9b:1::/48 is NAT64 for a network's own use.
    let local_nat64 = first == 0x64 && second == 0xff9b && ip.segments()[2] == 1;
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || unique_local
        || link_local
        || local_nat64
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

    /// The same local addresses in the spellings that are easy to miss:
    /// the rest of "this network", multicast, and the IPv6 forms that only
    /// wrap an IPv4 address.
    #[test]
    fn a_local_address_in_another_spelling_is_still_local_only() {
        for host in [
            // 0.0.0.0/8, not only 0.0.0.0.
            "0.1.2.3",
            "0.255.255.255",
            // Multicast.
            "224.0.0.251",
            "239.255.255.250",
            "ff02::1",
            "ff05::1:3",
            // IPv4-compatible.
            "::127.0.0.1",
            "::10.0.0.5",
            "::192.168.1.20",
            "::",
            // IPv4-mapped.
            "::ffff:127.0.0.1",
            "::ffff:0.0.0.0",
            "::ffff:224.0.0.1",
            // NAT64.
            "64:ff9b::127.0.0.1",
            "64:ff9b::10.0.0.5",
            "64:ff9b::c0a8:114",
            "64:ff9b:1::1",
            // 6to4.
            "2002:7f00:1::1",
            "2002:a00:5::",
            "2002:c0a8:114:1::1",
            "2002:a9fe:a01::1",
        ] {
            assert!(is_local_only_name(host), "{host}");
        }
        for url in [
            "http://[::127.0.0.1]:8080/admin?token=1",
            "http://[64:ff9b::7f00:1]/",
            "http://[2002:c0a8:0114::1]/",
            "http://0.0.0.1/",
            "http://224.0.0.251/",
            // Numbers a browser reads as an IPv4 address.
            "http://0x7f.1/",
            "http://2130706433/",
        ] {
            let url = url::Url::parse(url).unwrap();
            assert!(is_local_only(&url.host().unwrap()), "{url}");
        }
    }

    #[test]
    fn a_public_address_wrapped_in_ipv6_is_as_public_as_it_was() {
        for host in [
            "::8.8.8.8",
            "64:ff9b::8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
            "2001:db8::1",
            "2606:4700:4700::1111",
            "1.0.0.1",
            "223.255.255.1",
        ] {
            assert!(!is_local_only_name(host), "{host}");
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
