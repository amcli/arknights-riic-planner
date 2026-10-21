//! Who a request comes from: the client's IP address, for per-client
//! limits.
//!
//! The client is the connection's peer, unless that peer is one of the
//! trusted proxies the server was told about. Then the client is read from
//! `X-Forwarded-For`, from the right: each proxy appends the address it got
//! the request from, so the rightmost entry that is not itself a trusted
//! proxy is the first address no client could have chosen. Anything to its
//! left came from the client and is ignored. With no trusted proxies the
//! header is never read, so a client cannot pose as someone else by sending
//! it. Each entry is read on its own, so one a client garbled cannot hide
//! the entries the proxies added after it. An IPv4 address written as IPv6
//! (`::ffff:192.0.2.1`, as a dual-stack listener reports IPv4 peers) is
//! treated as the IPv4 address throughout.
//!
//! A request with no peer address did not come through a socket: it was
//! made in-process, as tests do, and has no client to tell apart.

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request};
use ipnet::IpNet;

/// The client a request comes from, given the proxies trusted to name it;
/// `None` when the request has no peer address.
pub fn client_ip<B>(req: &Request<B>, trusted: &[IpNet]) -> Option<IpAddr> {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>()?.0.ip();
    Some(behind(peer.to_canonical(), req.headers(), trusted))
}

/// The client behind `peer`: `peer` itself unless it is a trusted proxy.
/// Walking `X-Forwarded-For` stops at an entry that is not an address, and
/// the last address read stands.
fn behind(peer: IpAddr, headers: &HeaderMap, trusted: &[IpNet]) -> IpAddr {
    let is_trusted = |ip: &IpAddr| trusted.iter().any(|net| net.contains(ip));
    if !is_trusted(&peer) {
        return peer;
    }
    // Entries are split before they are decoded: a byte that is not text
    // spoils only its own entry.
    let hops: Vec<&[u8]> = headers
        .get_all("x-forwarded-for")
        .iter()
        .flat_map(|value| value.as_bytes().split(|&b| b == b','))
        .collect();
    let mut client = peer;
    for hop in hops.iter().rev() {
        let Some(ip) = std::str::from_utf8(hop).ok().and_then(parse_hop) else {
            break;
        };
        client = ip;
        if !is_trusted(&ip) {
            break;
        }
    }
    client
}

/// An `X-Forwarded-For` entry: an address, perhaps with a port (IPv6 in
/// brackets).
fn parse_hop(hop: &str) -> Option<IpAddr> {
    let hop = hop.trim();
    hop.parse::<IpAddr>()
        .ok()
        .or_else(|| hop.parse::<SocketAddr>().ok().map(|s| s.ip()))
        .or_else(|| hop.strip_prefix('[')?.strip_suffix(']')?.parse().ok())
        .map(|ip| ip.to_canonical())
}

/// A trusted proxy as configured: a block such as `10.0.0.0/8`, or a single
/// address.
pub fn parse_trusted(text: &str) -> Result<IpNet, String> {
    let text = text.trim();
    text.parse::<IpNet>()
        .or_else(|_| {
            text.parse::<IpAddr>()
                .map(|ip| IpNet::from(ip.to_canonical()))
        })
        .map_err(|_| format!("{text:?} is not an IP address or block such as 10.0.0.0/8"))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn forwarded(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn without_trusted_proxies_the_header_is_ignored() {
        let headers = forwarded(&["203.0.113.9"]);
        assert_eq!(
            behind(ip("198.51.100.1"), &headers, &[]),
            ip("198.51.100.1")
        );
        let proxy = [parse_trusted("10.0.0.1").unwrap()];
        assert_eq!(
            behind(ip("198.51.100.1"), &headers, &proxy),
            ip("198.51.100.1")
        );
    }

    #[test]
    fn behind_a_trusted_proxy_the_rightmost_untrusted_hop_is_the_client() {
        let proxies = [
            parse_trusted("10.0.0.0/8").unwrap(),
            parse_trusted("::1").unwrap(),
        ];
        let peer = ip("10.0.0.1");
        // The client claimed to be 1.1.1.1; the proxy saw 203.0.113.9.
        let headers = forwarded(&["1.1.1.1, 203.0.113.9"]);
        assert_eq!(behind(peer, &headers, &proxies), ip("203.0.113.9"));
        // Two proxies in a row, the header split over two lines.
        let headers = forwarded(&["1.1.1.1", "203.0.113.9, 10.0.0.7"]);
        assert_eq!(behind(peer, &headers, &proxies), ip("203.0.113.9"));
        // Ports and brackets.
        let headers = forwarded(&["203.0.113.9:4711"]);
        assert_eq!(behind(peer, &headers, &proxies), ip("203.0.113.9"));
        let headers = forwarded(&["[2001:db8::5]:443"]);
        assert_eq!(behind(ip("::1"), &headers, &proxies), ip("2001:db8::5"));
        let headers = forwarded(&["[2001:db8::5]"]);
        assert_eq!(behind(ip("::1"), &headers, &proxies), ip("2001:db8::5"));
    }

    #[test]
    fn a_bad_or_missing_header_leaves_the_last_address_read() {
        let proxies = [parse_trusted("10.0.0.0/8").unwrap()];
        let peer = ip("10.0.0.1");
        assert_eq!(behind(peer, &HeaderMap::new(), &proxies), peer);
        let headers = forwarded(&["203.0.113.9, unknown"]);
        assert_eq!(behind(peer, &headers, &proxies), peer);
        let headers = forwarded(&["unknown, 10.0.0.7"]);
        assert_eq!(behind(peer, &headers, &proxies), ip("10.0.0.7"));
        // Every hop a proxy: the leftmost stands.
        let headers = forwarded(&["10.0.0.9, 10.0.0.7"]);
        assert_eq!(behind(peer, &headers, &proxies), ip("10.0.0.9"));
    }

    #[test]
    fn a_garbled_entry_spoils_only_itself() {
        let proxies = [parse_trusted("10.0.0.0/8").unwrap()];
        let peer = ip("10.0.0.1");
        // The client wrote a byte that is not text; the proxy appended the
        // address it saw, on the same line.
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_bytes(b"caf\xe9, 203.0.113.9").unwrap(),
        );
        assert_eq!(behind(peer, &headers, &proxies), ip("203.0.113.9"));
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_bytes(b"203.0.113.\xff").unwrap(),
        );
        assert_eq!(behind(peer, &headers, &proxies), peer);
    }

    #[test]
    fn ipv4_written_as_ipv6_is_ipv4() {
        // A dual-stack listener reports an IPv4 proxy as ::ffff:10.0.0.1.
        let proxies = [parse_trusted("10.0.0.1").unwrap()];
        let headers = forwarded(&["203.0.113.9"]);
        assert_eq!(
            behind(ip("::ffff:10.0.0.1").to_canonical(), &headers, &proxies),
            ip("203.0.113.9")
        );
        let proxies = [parse_trusted("::ffff:10.0.0.1").unwrap()];
        assert_eq!(
            behind(ip("10.0.0.1"), &headers, &proxies),
            ip("203.0.113.9")
        );
        let headers = forwarded(&["::ffff:203.0.113.9"]);
        assert_eq!(
            behind(ip("10.0.0.1"), &headers, &proxies),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn trusted_proxies_are_addresses_or_blocks() {
        assert_eq!(
            parse_trusted(" 10.0.0.0/8 ").unwrap().to_string(),
            "10.0.0.0/8"
        );
        assert_eq!(
            parse_trusted("127.0.0.1").unwrap().to_string(),
            "127.0.0.1/32"
        );
        assert_eq!(parse_trusted("::1").unwrap().to_string(), "::1/128");
        assert!(parse_trusted("localhost").is_err());
        assert!(parse_trusted("10.0.0.0/33").is_err());
    }
}
