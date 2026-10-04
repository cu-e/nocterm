//! Where a server is: its country, from its public address, and that
//! country's flag.
//!
//! Lookups block and go over the network; callers run them on the background
//! executor. Only the resolved address is sent to the GeoIP service, never the
//! host name, and private, local and reserved addresses are never sent.

use std::{
    io::Read as _,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs as _},
    time::Duration,
};

/// Answers `{"ip": "…", "country": "DE"}` for an address.
const GEOIP_URL: &str = "https://api.country.is/";
/// Serves `<code>.svg` for lower-case ISO 3166-1 alpha-2 codes.
const FLAG_URL: &str = "https://flagcdn.com/";
const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_GEOIP_BYTES: u64 = 4 * 1024;
const MAX_FLAG_BYTES: u64 = 256 * 1024;

/// Whether `code` is shaped like an ISO 3166-1 alpha-2 code in lower case.
/// Codes end up in URLs and file names, so nothing else is accepted.
pub(crate) fn is_country_code(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_lowercase())
}

/// Whether `ip` is routed on the public internet, so a GeoIP service can
/// place it.
pub(crate) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(ip) => is_public_v4(ip),
            None => is_public_v6(ip),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        // Shared address space (carrier-grade NAT).
        || (a == 100 && (64..128).contains(&b))
        // Benchmarking.
        || (a == 198 && (18..20).contains(&b))
        // Reserved for future use.
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local.
        || (first & 0xfe00) == 0xfc00
        // Link local.
        || (first & 0xffc0) == 0xfe80
        // Documentation.
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
}

/// The public address `host` stands for, if it has one. Names are resolved
/// locally; a name that resolves only to private addresses has none.
pub(crate) fn public_address(host: &str) -> Option<IpAddr> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public(ip).then_some(ip);
    }
    if host.is_empty() || host.ends_with(".local") || !host.contains('.') {
        return None;
    }
    (host, 0)
        .to_socket_addrs()
        .ok()?
        .map(|address| address.ip())
        .find(|ip| is_public(*ip))
}

pub(crate) fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("nocterm/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())
}

/// The country `ip` is in, as a lower-case code; `None` when the service
/// does not know.
pub(crate) fn country_of(
    client: &reqwest::blocking::Client,
    ip: IpAddr,
) -> Result<Option<String>, String> {
    #[derive(serde::Deserialize)]
    struct Answer {
        country: Option<String>,
    }
    let body = get(client, &format!("{GEOIP_URL}{ip}"), MAX_GEOIP_BYTES)?;
    let answer: Answer = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    Ok(answer
        .country
        .map(|code| code.to_ascii_lowercase())
        .filter(|code| is_country_code(code)))
}

/// Downloads the flag of `code` as SVG.
pub(crate) fn download_flag(
    client: &reqwest::blocking::Client,
    code: &str,
) -> Result<Vec<u8>, String> {
    if !is_country_code(code) {
        return Err(format!("`{code}` is not a country code"));
    }
    let body = get(client, &format!("{FLAG_URL}{code}.svg"), MAX_FLAG_BYTES)?;
    if !is_svg(&body) {
        return Err(format!("the flag of `{code}` is not an SVG image"));
    }
    Ok(body)
}

/// Whether `bytes` look like an SVG document, not an error page.
pub(crate) fn is_svg(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).contains("<svg")
}

fn get(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| error.without_url().to_string())?;
    let mut body = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() as u64 > limit {
        return Err(format!("response exceeds {limit} bytes"));
    }
    Ok(body)
}

/// Flags drawn from stripes alone, for when the network is unavailable.
pub(crate) fn builtin_flag(code: &str) -> Option<Vec<u8>> {
    // Whether the stripes are vertical, and their colours from the top or left.
    let (vertical, colors): (bool, &[&str]) = match code {
        "at" => (false, &["#C8102E", "#FFFFFF", "#C8102E"]),
        "be" => (true, &["#000000", "#FDDA24", "#EF3340"]),
        "bg" => (false, &["#FFFFFF", "#00966E", "#D62612"]),
        "de" => (false, &["#000000", "#DD0000", "#FFCE00"]),
        "ee" => (false, &["#0072CE", "#000000", "#FFFFFF"]),
        "fr" => (true, &["#002654", "#FFFFFF", "#CE1126"]),
        "hu" => (false, &["#CE2939", "#FFFFFF", "#477050"]),
        "id" => (false, &["#CE1126", "#FFFFFF"]),
        "ie" => (true, &["#169B62", "#FFFFFF", "#FF883E"]),
        "it" => (true, &["#009246", "#FFFFFF", "#CE2B37"]),
        "lt" => (false, &["#FDB913", "#006A44", "#C1272D"]),
        "lu" => (false, &["#EA141D", "#FFFFFF", "#51ADDA"]),
        "lv" => (
            false,
            &["#9E3039", "#9E3039", "#FFFFFF", "#9E3039", "#9E3039"],
        ),
        "mc" => (false, &["#CE1126", "#FFFFFF"]),
        "ng" => (true, &["#008751", "#FFFFFF", "#008751"]),
        "nl" => (false, &["#AE1C28", "#FFFFFF", "#21468B"]),
        "pl" => (false, &["#FFFFFF", "#DC143C"]),
        "ro" => (true, &["#002B7F", "#FCD116", "#CE1126"]),
        "ru" => (false, &["#FFFFFF", "#0039A6", "#D52B1E"]),
        "ua" => (false, &["#0057B7", "#FFD700"]),
        _ => return None,
    };
    let count = colors.len();
    let stripes: String = colors
        .iter()
        .enumerate()
        .map(|(ix, color)| {
            // Each stripe overlaps the next a little, against seams.
            let (x, y, width, height) = if vertical {
                (ix as f32, 0.0, 1.05, 1.0)
            } else {
                (0.0, ix as f32, 1.0, 1.05)
            };
            format!(r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="{color}"/>"#)
        })
        .collect();
    let view_box = if vertical {
        format!("0 0 {count} 1")
    } else {
        format!("0 0 1 {count}")
    };
    Some(
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16" viewBox="{view_box}" preserveAspectRatio="none">{stripes}</svg>"#
        )
        .into_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_reserved_addresses_are_not_public() {
        for private in [
            "10.0.0.1",
            "172.16.4.2",
            "192.168.1.10",
            "127.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fd12:3456::1",
            "2001:db8::1",
            "::ffff:192.168.0.1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in ["1.1.1.1", "8.8.8.8", "100.128.0.1", "2a00:1450:4001::1"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn local_names_are_not_resolved() {
        assert_eq!(public_address("192.168.1.10"), None);
        assert_eq!(public_address("[::1]"), None);
        assert_eq!(public_address("pi.local"), None);
        assert_eq!(public_address("localhost"), None);
        assert_eq!(public_address("8.8.8.8"), Some("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn country_codes_are_two_lower_case_letters() {
        assert!(is_country_code("de"));
        for invalid in ["", "d", "DE", "deu", "../", "d1", "ü"] {
            assert!(!is_country_code(invalid), "{invalid}");
        }
    }

    #[test]
    fn builtin_flags_are_svg() {
        for code in ["de", "fr", "nl", "ua", "lv"] {
            let flag = builtin_flag(code).unwrap();
            assert!(is_svg(&flag), "{code}");
        }
        assert!(builtin_flag("us").is_none());
        assert!(!is_svg(b"<html>Not found</html>"));
        assert!(is_svg(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"\"></svg>"));
    }
}
