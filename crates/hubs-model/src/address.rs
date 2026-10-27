//! Where a hub listens: the canonical address that also identifies it.

use std::fmt;
use std::net::Ipv6Addr;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::HubAddressError;

/// The port a hub listens on unless the address names another.
pub const DEFAULT_PORT: u16 = 7443;

const MAX_HOST_LEN: usize = 253;
const MAX_LABEL_LEN: usize = 63;

/// A hub's `host:port`, lower-case, with the default port made explicit.
///
/// Only TLS addresses exist: `https://` is accepted and dropped, `http://` is refused.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HubAddress {
    host: String,
    port: u16,
}

impl HubAddress {
    /// Reads what a person types: `hub.example.com`, `hub.example.com:7443`,
    /// `[::1]:7443` or `https://hub.example.com/`.
    pub fn parse(text: &str) -> Result<Self, HubAddressError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(HubAddressError::Empty);
        }
        let rest = strip_scheme(text)?;
        let rest = rest.trim_end_matches('/');
        let (host, port) = split_host_port(rest)?;
        let port = match port {
            Some(port) => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or(HubAddressError::InvalidPort)?,
            None => DEFAULT_PORT,
        };
        let host = host.to_ascii_lowercase();
        if !valid_host(&host) {
            return Err(HubAddressError::InvalidHost);
        }
        Ok(Self { host, port })
    }

    /// The host name or IP literal, lower-case and without brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Drops `https://`; refuses plaintext and every other scheme.
fn strip_scheme(text: &str) -> Result<&str, HubAddressError> {
    let Some((scheme, rest)) = text.split_once("://") else {
        return Ok(text);
    };
    if scheme.eq_ignore_ascii_case("https") {
        Ok(rest)
    } else if scheme.eq_ignore_ascii_case("http") {
        Err(HubAddressError::PlaintextRefused)
    } else {
        Err(HubAddressError::UnsupportedScheme)
    }
}

/// `host` and the port text after it, if any; an IPv6 literal is in brackets.
fn split_host_port(text: &str) -> Result<(&str, Option<&str>), HubAddressError> {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(HubAddressError::InvalidHost)?;
        if host.parse::<Ipv6Addr>().is_err() {
            return Err(HubAddressError::InvalidHost);
        }
        return match after {
            "" => Ok((host, None)),
            after => after
                .strip_prefix(':')
                .map(|port| (host, Some(port)))
                .ok_or(HubAddressError::InvalidHost),
        };
    }
    match text.split_once(':') {
        None => Ok((text, None)),
        Some((host, port)) if !port.contains(':') => Ok((host, Some(port))),
        // A second colon: an IPv6 literal without brackets cannot carry a port.
        Some(_) => Err(HubAddressError::InvalidHost),
    }
}

fn valid_host(host: &str) -> bool {
    if host.contains(':') {
        return host.parse::<Ipv6Addr>().is_ok();
    }
    host.len() <= MAX_HOST_LEN
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= MAX_LABEL_LEN
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

impl fmt::Display for HubAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

impl fmt::Debug for HubAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HubAddress({self})")
    }
}

impl FromStr for HubAddress {
    type Err = HubAddressError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Serialize for HubAddress {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for HubAddress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// The identity of a hub on this device: its canonical address.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HubId(HubAddress);

impl HubId {
    pub fn address(&self) -> &HubAddress {
        &self.0
    }
}

impl From<HubAddress> for HubId {
    fn from(address: HubAddress) -> Self {
        Self(address)
    }
}

impl fmt::Display for HubId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for HubId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HubId({})", self.0)
    }
}

#[cfg(test)]
mod tests;
