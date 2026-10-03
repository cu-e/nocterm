//! Explicit access and zeroization for authentication secrets.
use secrecy::{ExposeSecret as _, SecretString};
use std::fmt;

/// A password or passphrase, redacted in Debug and zeroized on drop.
#[derive(Clone)]
pub struct Secret(SecretString);
impl Secret {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(SecretString::from(secret.into()))
    }
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(…)")
    }
}
impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}
impl Eq for Secret {}
