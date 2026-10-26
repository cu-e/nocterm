//! The version of the plugin API.

use alloc::string::{String, ToString};
use core::{fmt, str::FromStr};

use thiserror::Error;

/// The API version `major.minor` this build of Nocterm offers.
///
/// It holds the next unreleased minor: the first surface-changing change after a release bumps it.
pub const API_VERSION: ApiVersion = ApiVersion::new(0, 1);

/// An API version. Additive changes bump the minor; a major bump is an owner decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApiVersion {
    pub major: u16,
    pub minor: u16,
}

impl ApiVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Whether a host offering `host` runs a package written for `self`.
    pub fn runs_on(self, host: Self) -> bool {
        self <= host
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// A version that is not `major.minor`.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("`{text}` is not an API version like 0.1")]
pub struct VersionError {
    text: String,
}

impl FromStr for ApiVersion {
    type Err = VersionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || VersionError {
            text: text.to_string(),
        };
        let (major, minor) = text.split_once('.').ok_or_else(error)?;
        let number = |part: &str| {
            let digits = !part.is_empty() && part.len() <= 4;
            (digits && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse::<u16>().ok())
                .flatten()
        };
        Ok(Self::new(
            number(major).ok_or_else(error)?,
            number(minor).ok_or_else(error)?,
        ))
    }
}
