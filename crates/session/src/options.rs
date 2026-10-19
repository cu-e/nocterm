//! Serializable session overrides. None inherits the corresponding user default.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Why an explicit terminal type cannot be announced to a session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TermError {
    #[error("TERM must contain 1–128 ASCII letters, digits or -_+.")]
    Empty,
    #[error("TERM must contain 1–128 ASCII letters, digits or -_+.")]
    TooLong { length: usize },
    #[error("TERM must contain 1–128 ASCII letters, digits or -_+.")]
    InvalidCharacter,
}

/// Why a proxy cannot be used. Rejected host text is never copied into the error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProxyError {
    #[error("Proxy needs a host without whitespace and a port from 1 to 65535.")]
    EmptyHost,
    #[error("Proxy needs a host without whitespace and a port from 1 to 65535.")]
    HostTooLong { length: usize },
    #[error("Proxy needs a host without whitespace and a port from 1 to 65535.")]
    InvalidHost,
    #[error("Proxy needs a host without whitespace and a port from 1 to 65535.")]
    ZeroPort,
    #[error("Proxy IPv6 address is invalid.")]
    InvalidIpv6(#[source] std::net::AddrParseError),
}

/// Why explicit output-recording options cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoggingError {
    #[error("Log size must be from 1 to 1024 MiB.")]
    InvalidSize { value: u32 },
    #[error("Log directory cannot be empty.")]
    EmptyDirectory,
}

/// The override that failed validation; the original cause remains available.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionOptionsError {
    #[error(transparent)]
    Term(#[from] TermError),
    #[error(transparent)]
    Proxy(#[from] ProxyError),
    #[error(transparent)]
    Logging(#[from] LoggingError),
}

pub const TERM_PRESETS: &[&str] = &[
    "xterm-256color",
    "xterm",
    "screen-256color",
    "screen",
    "vt100",
    "linux",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Charset {
    #[default]
    Utf8,
    Windows1251,
    Koi8R,
    Windows1252,
    Gbk,
}
impl Charset {
    pub const ALL: [Self; 5] = [
        Self::Utf8,
        Self::Windows1251,
        Self::Koi8R,
        Self::Windows1252,
        Self::Gbk,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Windows1251 => "Windows-1251",
            Self::Koi8R => "KOI8-R",
            Self::Windows1252 => "Windows-1252",
            Self::Gbk => "GBK",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxyConfig {
    #[default]
    Direct,
    HttpConnect {
        host: String,
        port: u16,
    },
    Socks5 {
        host: String,
        port: u16,
        remote_dns: bool,
    },
}
impl ProxyConfig {
    pub fn validate(&self) -> Result<(), ProxyError> {
        let (host, port) = match self {
            Self::Direct => return Ok(()),
            Self::HttpConnect { host, port } | Self::Socks5 { host, port, .. } => (host, port),
        };
        if host.is_empty() {
            return Err(ProxyError::EmptyHost);
        }
        if host.len() > 255 {
            return Err(ProxyError::HostTooLong { length: host.len() });
        }
        if !host.is_ascii()
            || host
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b"/\\@?#".contains(&b))
        {
            return Err(ProxyError::InvalidHost);
        }
        if *port == 0 {
            return Err(ProxyError::ZeroPort);
        }
        if host.contains(':') {
            host.parse::<std::net::Ipv6Addr>()
                .map_err(ProxyError::InvalidIpv6)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingOptions {
    /// Automatically record remote output after a successful connection. Never records authentication or input directly.
    pub auto_start: bool,
    /// Output directory. Unset uses the application's state/logs directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
    /// Stop recording at this size instead of filling the disk.
    #[schemars(extend("minimum"=1,"maximum"=1024))]
    pub max_file_mib: u32,
}
impl Default for LoggingOptions {
    fn default() -> Self {
        Self {
            auto_start: false,
            directory: None,
            max_file_mib: 10,
        }
    }
}
impl nocterm_settings::SettingsSection for LoggingOptions {
    const KEY: &'static str = "logging";

    fn sanitize(&mut self) {
        self.max_file_mib = self.max_file_mib.clamp(1, 1024);
    }
}

impl LoggingOptions {
    pub fn validate(&self) -> Result<(), LoggingError> {
        if !(1..=1024).contains(&self.max_file_mib) {
            return Err(LoggingError::InvalidSize {
                value: self.max_file_mib,
            });
        }
        if self
            .directory
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(LoggingError::EmptyDirectory);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SessionOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charset: Option<Charset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingOptions>,
}
impl SessionOptions {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), SessionOptionsError> {
        if let Some(term) = &self.term {
            validate_term(term)?;
        }
        if let Some(proxy) = &self.proxy {
            proxy.validate()?;
        }
        if let Some(logging) = &self.logging {
            logging.validate()?;
        }
        Ok(())
    }
}
pub fn validate_term(term: &str) -> Result<(), TermError> {
    if term.is_empty() {
        return Err(TermError::Empty);
    }
    if term.len() > 128 {
        return Err(TermError::TooLong { length: term.len() });
    }
    if !term
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_+.".contains(&b))
    {
        return Err(TermError::InvalidCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_type_limits_preserve_literal_values() {
        assert_eq!(validate_term(""), Err(TermError::Empty));
        assert!(validate_term(&"a".repeat(128)).is_ok());
        assert_eq!(
            validate_term(&"a".repeat(129)),
            Err(TermError::TooLong { length: 129 })
        );
        for invalid in ["té", "a b", "a\nb", "a/b", "a\0b"] {
            assert_eq!(validate_term(invalid), Err(TermError::InvalidCharacter));
        }
        assert!(validate_term("xterm-256color_+.").is_ok());
    }

    #[test]
    fn proxy_failures_distinguish_host_port_and_ipv6_causes() {
        let proxy = |host: &str, port| ProxyConfig::HttpConnect {
            host: host.into(),
            port,
        };
        assert_eq!(proxy("", 8080).validate(), Err(ProxyError::EmptyHost));
        assert!(proxy(&"a".repeat(255), 65535).validate().is_ok());
        assert_eq!(
            proxy(&"a".repeat(256), 1).validate(),
            Err(ProxyError::HostTooLong { length: 256 })
        );
        for host in [
            "bad host", "tést", "a@b", "a/b", "a\\b", "a?b", "a#b", "a\0b",
        ] {
            assert_eq!(proxy(host, 1).validate(), Err(ProxyError::InvalidHost));
        }
        assert_eq!(proxy("::invalid", 0).validate(), Err(ProxyError::ZeroPort));
        assert!(proxy("::1", 1).validate().is_ok());
        let error = proxy("127.0.0.1:1080", 8080).validate().unwrap_err();
        assert!(matches!(error, ProxyError::InvalidIpv6(_)));
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .is::<std::net::AddrParseError>()
        );
        assert_eq!(error.to_string(), "Proxy IPv6 address is invalid.");
        assert!(ProxyConfig::Direct.validate().is_ok());
    }

    #[test]
    fn logging_validation_accepts_defaults_and_reports_exact_limits() {
        let options = |value| LoggingOptions {
            max_file_mib: value,
            ..Default::default()
        };
        for value in [1, 1024] {
            assert!(options(value).validate().is_ok());
        }
        for value in [0, 1025] {
            assert_eq!(
                options(value).validate(),
                Err(LoggingError::InvalidSize { value })
            );
        }
        let options = LoggingOptions {
            directory: Some(PathBuf::new()),
            ..Default::default()
        };
        assert_eq!(options.validate(), Err(LoggingError::EmptyDirectory));
    }

    #[test]
    fn session_overrides_keep_validation_order_and_typed_causes() {
        let mut options = SessionOptions {
            term: Some(String::new()),
            proxy: Some(ProxyConfig::HttpConnect {
                host: String::new(),
                port: 0,
            }),
            logging: Some(LoggingOptions {
                max_file_mib: 0,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            options.validate(),
            Err(SessionOptionsError::Term(TermError::Empty))
        );
        options.term = None;
        assert_eq!(
            options.validate(),
            Err(SessionOptionsError::Proxy(ProxyError::EmptyHost))
        );
        options.proxy = None;
        assert_eq!(
            options.validate(),
            Err(SessionOptionsError::Logging(LoggingError::InvalidSize {
                value: 0
            }))
        );
        assert!(SessionOptions::default().validate().is_ok());
    }

    #[test]
    fn optional_overrides_round_trip_and_reject_bad_values() {
        let options = SessionOptions {
            term: Some("screen-256color".into()),
            charset: Some(Charset::Koi8R),
            proxy: Some(ProxyConfig::Socks5 {
                host: "127.0.0.1".into(),
                port: 1080,
                remote_dns: true,
            }),
            logging: Some(LoggingOptions::default()),
        };
        options.validate().unwrap();
        let text = toml::to_string(&options).unwrap();
        assert_eq!(toml::from_str::<SessionOptions>(&text).unwrap(), options);
        assert!(
            toml::to_string(&SessionOptions::default())
                .unwrap()
                .is_empty()
        );
        for value in ["", "a\nb", "path/name", "$(sh)"] {
            assert!(validate_term(value).is_err());
        }
        for host in [
            "bad name",
            "username@host",
            "tést",
            "127.0.0.1:1080",
            "host/path",
        ] {
            assert!(
                ProxyConfig::HttpConnect {
                    host: host.into(),
                    port: 8080
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ProxyConfig::Socks5 {
                host: "::1".into(),
                port: 1080,
                remote_dns: true
            }
            .validate()
            .is_ok()
        );
        assert!(
            LoggingOptions {
                max_file_mib: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
