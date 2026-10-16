//! IP address and CIDR helpers (IPv4 and IPv6), `WebPulse\Util\Ip`.

use std::net::IpAddr;

use crate::util::strs::php_trim_str;

/// A compiled network: packed address (4 or 16 bytes, masked) and prefix length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cidr {
    net: Vec<u8>,
    bits: u32,
}

fn packed(ip: &IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(a) => a.octets().to_vec(),
        IpAddr::V6(a) => a.octets().to_vec(),
    }
}

/// Parses an address as `inet_pton()` does (no zone ids, no brackets).
pub fn parse_ip(s: &str) -> Option<IpAddr> {
    if s.contains('%') {
        return None;
    }
    s.parse::<IpAddr>().ok()
}

/// True for a syntactically valid IPv4 or IPv6 address.
pub fn is_valid(ip: &str) -> bool {
    parse_ip(ip).is_some()
}

/// True for `addr` or `addr/prefix` with a prefix valid for the family.
pub fn is_valid_cidr(cidr: &str) -> bool {
    parse_cidr(cidr).is_some()
}

/// Zeroes every bit after the first `bits` of a packed address.
fn mask(p: &[u8], bits: u32) -> Vec<u8> {
    p.iter()
        .enumerate()
        .map(|(i, &b)| {
            let take = (bits as i64 - (i as i64) * 8).clamp(0, 8) as u32;
            let m: u8 = if take == 0 {
                0
            } else {
                (0xFFu16 << (8 - take)) as u8
            };
            b & m
        })
        .collect()
}

/// Parses `addr[/bits]` (trimmed); `None` when invalid.
pub fn parse_cidr(cidr: &str) -> Option<Cidr> {
    let cidr = php_trim_str(cidr);
    let (addr, bits) = match cidr.split_once('/') {
        Some((a, b)) => {
            if b.is_empty() || !b.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            (a, Some(b.parse::<u64>().unwrap_or(u64::MAX)))
        }
        None => (cidr, None),
    };
    let ip = parse_ip(addr)?;
    let p = packed(&ip);
    let max = (p.len() * 8) as u64;
    let bits = bits.unwrap_or(max);
    if bits > max {
        return None;
    }
    let bits = bits as u32;
    Some(Cidr {
        net: mask(&p, bits),
        bits,
    })
}

/// Compiles IPs/CIDRs into matchers; invalid entries are skipped.
pub fn compile<S: AsRef<str>>(list: &[S]) -> Vec<Cidr> {
    list.iter().filter_map(|c| parse_cidr(c.as_ref())).collect()
}

/// Packed form of an address with IPv4-mapped IPv6 (`::ffff:a.b.c.d`) reduced to IPv4.
pub fn packed_unmapped(ip: &str) -> Option<Vec<u8>> {
    let a = parse_ip(ip)?;
    let mut p = packed(&a);
    if p.len() == 16 && p[..12] == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff] {
        p = p[12..].to_vec();
    }
    Some(p)
}

/// True when `ip` falls in one of the compiled networks (IPv4-mapped IPv6 matched as IPv4).
pub fn matches(ip: &str, compiled: &[Cidr]) -> bool {
    if compiled.is_empty() {
        return false;
    }
    let Some(p) = packed_unmapped(ip) else {
        return false;
    };
    compiled
        .iter()
        .any(|c| c.net.len() == p.len() && mask(&p, c.bits) == c.net)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_matching() {
        let c = compile(&[
            "203.0.113.0/24",
            "2001:db8::/32",
            "198.51.100.7",
            "bogus",
            "10.0.0.0/33",
        ]);
        assert_eq!(c.len(), 3);
        assert!(matches("203.0.113.99", &c));
        assert!(!matches("203.0.114.1", &c));
        assert!(matches("2001:db8:1::5", &c));
        assert!(!matches("2001:db9::1", &c));
        assert!(matches("::ffff:198.51.100.7", &c));
        assert!(!matches("not-an-ip", &c));
        assert!(is_valid_cidr("::/0"));
        assert!(!is_valid_cidr("1.2.3.4/"));
        assert!(is_valid_cidr(" 1.2.3.0/24 "));
    }
}
