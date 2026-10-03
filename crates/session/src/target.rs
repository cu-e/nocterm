//! Where a session connects and how the user proves who they are.

use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

/// The port SSH listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 22;

/// A host and the account to log in to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Target {
    pub host: String,
    #[serde(default = "default_port", skip_serializing_if = "is_default_port")]
    pub port: u16,
    pub user: String,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn is_default_port(port: &u16) -> bool {
    *port == DEFAULT_PORT
}

/// Text is not a `[user@]host[:port]` destination.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseTargetError {
    #[error("enter a host to connect to")]
    MissingHost,
    #[error("enter a user name, as in user@host")]
    MissingUser,
    #[error("`{0}` is not a port number")]
    InvalidPort(String),
    #[error("a host name cannot contain spaces")]
    InvalidHost,
}

impl Target {
    pub fn new(user: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
        }
    }

    /// Parses a destination the way it is typed: `[user@]host[:port]`, with
    /// IPv6 addresses in brackets when a port follows, optionally prefixed
    /// with `ssh://`.
    ///
    /// `default_user` fills in for a missing `user@`.
    pub fn parse(text: &str, default_user: Option<&str>) -> Result<Self, ParseTargetError> {
        let text = text.trim();
        let text = text.strip_prefix("ssh://").unwrap_or(text);
        let text = text.strip_suffix('/').unwrap_or(text);

        // The user is everything before the last `@`: user names may hold one.
        let (user, rest) = match text.rsplit_once('@') {
            Some((user, rest)) => (Some(user), rest),
            None => (None, text),
        };

        let (host, port) = if let Some(bracketed) = rest.strip_prefix('[') {
            let (host, after) = bracketed
                .split_once(']')
                .ok_or(ParseTargetError::InvalidHost)?;
            match after.strip_prefix(':') {
                Some(port) => (host, Some(port)),
                None if after.is_empty() => (host, None),
                None => return Err(ParseTargetError::InvalidHost),
            }
        } else if rest.matches(':').count() > 1 {
            // A bare IPv6 address; its colons are not a port separator.
            (rest, None)
        } else {
            match rest.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (rest, None),
            }
        };

        if host.is_empty() {
            return Err(ParseTargetError::MissingHost);
        }
        if host.contains(char::is_whitespace) {
            return Err(ParseTargetError::InvalidHost);
        }
        let port = match port {
            None => DEFAULT_PORT,
            Some(port) => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| ParseTargetError::InvalidPort(port.to_owned()))?,
        };
        let user = user
            .or(default_user)
            .map(str::trim)
            .filter(|user| !user.is_empty())
            .ok_or(ParseTargetError::MissingUser)?;

        Ok(Self::new(user, host, port))
    }
}

/// `user@host`, with `:port` when it is not the default. IPv6 hosts are
/// bracketed whenever a port follows, so the output parses back.
impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@", self.user)?;
        match (self.port == DEFAULT_PORT, self.host.contains(':')) {
            (true, _) => write!(formatter, "{}", self.host),
            (false, true) => write!(formatter, "[{}]:{}", self.host, self.port),
            (false, false) => write!(formatter, "{}:{}", self.host, self.port),
        }
    }
}

/// How the user proves who they are.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Auth {
    /// What `ssh` does: the agent, then the default keys, then a password.
    #[default]
    Auto,
    /// Always ask for the account's password.
    Password,
    /// One private key file.
    Key { path: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Target, ParseTargetError> {
        Target::parse(text, Some("me"))
    }

    #[test]
    fn parses_the_forms_people_type() {
        assert_eq!(
            parse("example.com"),
            Ok(Target::new("me", "example.com", 22))
        );
        assert_eq!(
            parse("root@example.com"),
            Ok(Target::new("root", "example.com", 22))
        );
        assert_eq!(
            parse(" root@10.0.0.1:2222 "),
            Ok(Target::new("root", "10.0.0.1", 2222))
        );
        assert_eq!(
            parse("ssh://deploy@example.com:2200/"),
            Ok(Target::new("deploy", "example.com", 2200))
        );
        assert_eq!(
            parse("first.last@corp@host"),
            Ok(Target::new("first.last@corp", "host", 22))
        );
    }

    #[test]
    fn parses_ipv6_addresses() {
        assert_eq!(parse("::1"), Ok(Target::new("me", "::1", 22)));
        assert_eq!(parse("root@[::1]"), Ok(Target::new("root", "::1", 22)));
        assert_eq!(
            parse("root@[fe80::1]:2222"),
            Ok(Target::new("root", "fe80::1", 2222))
        );
    }

    #[test]
    fn rejects_what_cannot_be_a_destination() {
        assert_eq!(parse(""), Err(ParseTargetError::MissingHost));
        assert_eq!(parse("root@"), Err(ParseTargetError::MissingHost));
        assert_eq!(
            parse("host:ssh"),
            Err(ParseTargetError::InvalidPort("ssh".into()))
        );
        assert_eq!(
            parse("host:0"),
            Err(ParseTargetError::InvalidPort("0".into()))
        );
        assert_eq!(
            parse("host:70000"),
            Err(ParseTargetError::InvalidPort("70000".into()))
        );
        assert_eq!(parse("my host"), Err(ParseTargetError::InvalidHost));
        assert_eq!(parse("[::1"), Err(ParseTargetError::InvalidHost));
        assert_eq!(
            Target::parse("host", None),
            Err(ParseTargetError::MissingUser)
        );
        assert_eq!(
            Target::parse("@host", None),
            Err(ParseTargetError::MissingUser)
        );
    }

    #[test]
    fn display_round_trips_through_parse() {
        for target in [
            Target::new("root", "example.com", 22),
            Target::new("root", "example.com", 2222),
            Target::new("root", "::1", 22),
            Target::new("root", "fe80::1", 2222),
        ] {
            assert_eq!(Target::parse(&target.to_string(), None), Ok(target));
        }
        assert_eq!(Target::new("a", "h", 22).to_string(), "a@h");
        assert_eq!(Target::new("a", "::1", 2222).to_string(), "a@[::1]:2222");
    }
}

/// An opaque reference to encrypted credentials. It contains no secret metadata.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialId([u8; 16]);
impl CredentialId {
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes))
    }
}
impl fmt::Display for CredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl fmt::Debug for CredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CredentialId({self})")
    }
}
impl std::str::FromStr for CredentialId {
    type Err = &'static str;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("credential ID must be 32 hexadecimal characters");
        }
        let mut bytes = [0; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
                .map_err(|_| "invalid credential ID")?;
        }
        Ok(Self(bytes))
    }
}
impl Serialize for CredentialId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for CredentialId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
