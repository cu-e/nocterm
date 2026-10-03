//! Serializable session overrides. None inherits the corresponding user default.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub fn validate(&self) -> Result<(), String> {
        let (host, port) = match self {
            Self::Direct => return Ok(()),
            Self::HttpConnect { host, port } | Self::Socks5 { host, port, .. } => (host, port),
        };
        if host.is_empty()
            || host.len() > 255
            || !host.is_ascii()
            || host
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b"/\\@?#".contains(&b))
            || *port == 0
        {
            return Err("Proxy needs a host without whitespace and a port from 1 to 65535.".into());
        }
        if host.contains(':') && host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err("Proxy IPv6 address is invalid.".into());
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
impl LoggingOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=1024).contains(&self.max_file_mib) {
            return Err("Log size must be from 1 to 1024 MiB.".into());
        }
        if self
            .directory
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err("Log directory cannot be empty.".into());
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
    pub fn validate(&self) -> Result<(), String> {
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
pub fn validate_term(term: &str) -> Result<(), String> {
    if term.is_empty()
        || term.len() > 128
        || !term
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_+.".contains(&b))
    {
        return Err("TERM must contain 1–128 ASCII letters, digits or -_+.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
