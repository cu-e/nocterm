//! Checked observation values and blocking capabilities; callers own scheduling.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

/// A safe lower-case ISO 3166-1 alpha-2 shaped country key.
/// Shape validation does not assert membership in a current country catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CountryCode([u8; 2]);

impl CountryCode {
    pub fn new(value: &str) -> Result<Self, CountryCodeError> {
        let bytes = value.as_bytes();
        if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_lowercase) {
            return Err(CountryCodeError);
        }
        Ok(Self([bytes[0], bytes[1]]))
    }

    pub fn as_str(&self) -> &str {
        // Construction admits only ASCII, including after deserialization.
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a country code must be two lower-case ASCII letters")]
pub struct CountryCodeError;

impl Serialize for CountryCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CountryCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).map_err(serde::de::Error::custom)
    }
}

/// An address admitted by the existing country-observation privacy policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PublicAddress(IpAddr);

impl PublicAddress {
    pub fn new(ip: IpAddr) -> Option<Self> {
        is_public(ip).then_some(Self(ip))
    }

    pub fn as_ip(self) -> IpAddr {
        self.0
    }
}

pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or_else(|| is_public_v6(ip), is_public_v4),
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
        || (a == 100 && (64..128).contains(&b))
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
}

/// Maximum supplied/cache/network SVG flag body, shared by every producer.
pub const MAX_FLAG_BYTES: u64 = 256 * 1024;

/// Immutable bounded raw flag bytes; decoding and fallback remain presentation-owned.
#[derive(Clone, Debug)]
pub struct FlagBytes(std::sync::Arc<[u8]>);
impl FlagBytes {
    pub fn new(bytes: Vec<u8>) -> Result<Self, ObservationError> {
        if bytes.len() as u64 > MAX_FLAG_BYTES {
            return Err(ObservationError::BodyLimit {
                limit: MAX_FLAG_BYTES,
            });
        }
        Ok(Self(bytes.into()))
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Adapter errors retain native causes without coupling this domain to them.
#[derive(Debug, thiserror::Error)]
pub enum ObservationError {
    #[error("could not resolve public address")]
    Resolution(#[source] Box<dyn Error + Send + Sync>),
    #[error("country or flag network request failed")]
    Network(#[source] Box<dyn Error + Send + Sync>),
    #[error("country or flag request returned HTTP {status}")]
    Status {
        status: u16,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("observation response exceeds {limit} bytes")]
    BodyLimit { limit: u64 },
    #[error("could not read observation response")]
    Body(#[source] Box<dyn Error + Send + Sync>),
    #[error("observation response is invalid")]
    Decode(#[source] Box<dyn Error + Send + Sync>),
    #[error("could not publish flag cache")]
    Cache(#[source] Box<dyn Error + Send + Sync>),
}

pub trait PublicAddressResolver: Send + Sync {
    fn resolve(&self, host: &str) -> Result<Option<PublicAddress>, ObservationError>;
}
pub trait CountryLocator: Send + Sync {
    fn country(&self, ip: PublicAddress) -> Result<Option<CountryCode>, ObservationError>;
}
pub trait FlagRepository: Send + Sync {
    fn load(&self, code: CountryCode) -> Result<Option<Vec<u8>>, ObservationError>;
}

/// Inert dependencies for memory-only installations, with no real I/O.
pub struct OfflineObservations;
impl PublicAddressResolver for OfflineObservations {
    fn resolve(&self, _: &str) -> Result<Option<PublicAddress>, ObservationError> {
        Ok(None)
    }
}
impl CountryLocator for OfflineObservations {
    fn country(&self, _: PublicAddress) -> Result<Option<CountryCode>, ObservationError> {
        Ok(None)
    }
}
impl FlagRepository for OfflineObservations {
    fn load(&self, _: CountryCode) -> Result<Option<Vec<u8>>, ObservationError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
