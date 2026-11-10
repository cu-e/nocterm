//! Certificate fingerprints and the decision whether to trust a hub's certificate.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::FingerprintError;

/// SHA-256 of a hub's leaf certificate.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Reads the digits a hub prints or a person pastes: hex, any case, with or without
    /// colons and spaces.
    pub fn parse(text: &str) -> Result<Self, FingerprintError> {
        let digits: Vec<char> = text
            .chars()
            .filter(|c| *c != ':' && !c.is_whitespace())
            .collect();
        if digits.iter().any(|c| !c.is_ascii_hexdigit()) {
            return Err(FingerprintError::NotHex);
        }
        if digits.len() != 64 {
            return Err(FingerprintError::Length {
                found: digits.len(),
            });
        }
        let mut bytes = [0_u8; 32];
        for (byte, pair) in bytes.iter_mut().zip(digits.chunks(2)) {
            let high = pair[0].to_digit(16).unwrap_or(0);
            let low = pair[1].to_digit(16).unwrap_or(0);
            *byte = u8::try_from(high * 16 + low).unwrap_or(0);
        }
        Ok(Self(bytes))
    }

    /// Eight groups of four digits for reading aloud and comparing by eye.
    pub fn groups(&self) -> [String; 8] {
        std::array::from_fn(|group| {
            self.0[group * 4..group * 4 + 4]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        })
    }
}

impl fmt::Display for Fingerprint {
    /// Lower-case pairs separated by colons, the form stored in settings.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(":")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

impl FromStr for Fingerprint {
    type Err = FingerprintError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// How a hub's certificate is checked on every connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrustPolicy {
    /// Exactly this certificate; a different one is an error, never a prompt.
    Pinned(Fingerprint),
    /// Any certificate a public authority vouches for under the hub's name.
    PublicCa,
}

/// What a TLS handshake without trust showed of a hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbedHub {
    pub fingerprint: Fingerprint,
    /// A public authority vouches for the certificate under the probed name.
    pub publicly_trusted: bool,
}

/// What to do about the certificate a hub presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustDecision {
    /// The fingerprint the user typed matches: pin it, no question to ask.
    Pinned(Fingerprint),
    /// A public authority vouches for the certificate and nothing was typed.
    PublicCa,
    /// A self-signed certificate and nothing typed: show the fingerprint and let the user
    /// confirm it against the server's output before pinning.
    ConfirmFirstUse(Fingerprint),
    /// The typed fingerprint is not the presented one. There is nothing to accept.
    Mismatch {
        typed: Fingerprint,
        presented: Fingerprint,
    },
}

impl TrustDecision {
    /// The policy that needs no further answer, if there is one.
    pub const fn settled(&self) -> Option<TrustPolicy> {
        match self {
            Self::Pinned(fingerprint) => Some(TrustPolicy::Pinned(*fingerprint)),
            Self::PublicCa => Some(TrustPolicy::PublicCa),
            Self::ConfirmFirstUse(_) | Self::Mismatch { .. } => None,
        }
    }

    /// The policy after the user confirmed the shown fingerprint; only a first-use
    /// decision can be confirmed.
    pub const fn confirmed(&self) -> Option<TrustPolicy> {
        match self {
            Self::ConfirmFirstUse(fingerprint) => Some(TrustPolicy::Pinned(*fingerprint)),
            Self::Pinned(_) | Self::PublicCa | Self::Mismatch { .. } => None,
        }
    }
}

/// Decides how to treat a probed certificate given the fingerprint the user typed, if any.
///
/// A typed fingerprint always wins: it must match, even for a certificate a public authority
/// vouches for. Without one, a publicly trusted chain needs no pinning, so certificate
/// rotation stays silent, and a self-signed certificate must be confirmed once.
pub fn decide(probe: &ProbedHub, typed: Option<&Fingerprint>) -> TrustDecision {
    match typed {
        Some(typed) if *typed == probe.fingerprint => TrustDecision::Pinned(probe.fingerprint),
        Some(typed) => TrustDecision::Mismatch {
            typed: *typed,
            presented: probe.fingerprint,
        },
        None if probe.publicly_trusted => TrustDecision::PublicCa,
        None => TrustDecision::ConfirmFirstUse(probe.fingerprint),
    }
}

#[cfg(test)]
mod tests;
