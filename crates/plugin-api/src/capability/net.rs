//! `net.request`: a request to the web address the user entered in the plugin's settings.
//!
//! The plugin names a method, a path and a body; the host joins the path to the origin the user
//! typed into the setting the manifest's `[network]` table names, adds the credential the user
//! entered (which the plugin never reads), sends it without cookies and without following
//! redirects, and answers with a `response` event under the request's key.

use alloc::string::String;
use serde::{Deserialize, Serialize};

use super::{Capability, sealed};
use crate::LimitError;

/// Longest request path, query included, in bytes.
pub const MAX_PATH: usize = 2048;
/// Longest request body, in bytes.
pub const MAX_BODY: usize = 224 * 1024;
/// Longest response body a plugin receives, in bytes; a longer one is cut there.
pub const MAX_RESPONSE: usize = 256 * 1024;
/// Most bytes of an answer to a `bytes` request, which comes back as base64 (a third longer)
/// and must still fit in the call that hands it to the plugin; a longer answer is refused, not
/// cut.
pub const MAX_BYTES_RESPONSE: usize = 160 * 1024;
/// Most requests of one plugin in flight at once.
pub const MAX_IN_FLIGHT: usize = 4;
/// How long the host waits for an answer, in milliseconds.
pub const TIMEOUT_MS: u32 = 15_000;

/// The method of a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    /// Every method, in declaration order.
    pub const ALL: [Self; 4] = [Self::Get, Self::Post, Self::Put, Self::Delete];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Delete => "delete",
        }
    }
}

/// A request to the plugin's origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Request {
    pub method: Method,
    /// The path and query below the origin: starts with one `/`, holds visible ASCII only.
    pub path: String,
    /// A JSON body, sent as `application/json`; with `bytes`, base64 of the raw bytes, sent as
    /// `application/octet-stream`.
    #[serde(default)]
    pub body: Option<String>,
    /// The body is base64 of bytes, and the answer's body comes back as base64 of the bytes
    /// the server sent, so a binary file keeps every byte.
    #[serde(default)]
    pub bytes: bool,
}

impl Request {
    pub fn check(&self) -> Result<(), LimitError> {
        check_path(&self.path)?;
        if self.body.as_ref().is_some_and(|body| body.len() > MAX_BODY) {
            return Err(LimitError::TooLong {
                field: "body",
                max: MAX_BODY,
            });
        }
        if self.bytes
            && self
                .body
                .as_deref()
                .is_some_and(|body| crate::base64::decode(body).is_none())
        {
            return Err(LimitError::Syntax { field: "body" });
        }
        Ok(())
    }
}

/// A path the host can join to an origin without leaving it: one leading `/` (never `//`, which
/// would name another host), visible ASCII without `\`, `#` or spaces, and no `..` segment.
pub fn check_path(path: &str) -> Result<(), LimitError> {
    if path.len() > MAX_PATH {
        return Err(LimitError::TooLong {
            field: "path",
            max: MAX_PATH,
        });
    }
    let syntax = LimitError::Syntax { field: "path" };
    let visible = path
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\\' | b'#'));
    if !path.starts_with('/') || path.starts_with("//") || !visible {
        return Err(syntax);
    }
    let route = path.split('?').next().unwrap_or_default();
    if route
        .split('/')
        .any(|segment| segment == ".." || segment == ".")
    {
        return Err(syntax);
    }
    Ok(())
}

/// Marker of `net.request`.
pub struct NetRequest;

impl sealed::Sealed for NetRequest {}

impl Capability for NetRequest {
    const ID: &'static str = "net.request";
    type Value = Request;
}

/// The web address a user typed into the setting that names a plugin's origin: `http` or
/// `https`, a host with an optional port, and an optional base path every request goes below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    secure: bool,
    authority: String,
    base: String,
}

/// A typed address the host does not send requests to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OriginError {
    #[error("no address is set")]
    Empty,
    #[error("the address does not start with http:// or https://")]
    Scheme,
    #[error("the address has no host, or one with a user name, a space or a bad port")]
    Host,
    #[error("the address has a query, a fragment or an unusual path")]
    Path,
}

impl Origin {
    pub fn parse(text: &str) -> Result<Self, OriginError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(OriginError::Empty);
        }
        let lower = text.to_ascii_lowercase();
        let (secure, rest) = if lower.starts_with("https://") {
            (true, &text[8..])
        } else if lower.starts_with("http://") {
            (false, &text[7..])
        } else {
            return Err(OriginError::Scheme);
        };
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(split);
        if !authority_allowed(authority) {
            return Err(OriginError::Host);
        }
        let base = path.trim_end_matches('/');
        if !base.is_empty() && check_path(base).is_err() {
            return Err(OriginError::Path);
        }
        Ok(Self {
            secure,
            authority: authority.to_ascii_lowercase(),
            base: String::from(base),
        })
    }

    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// `host` or `host:port`, lower case.
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// The address of `path` below the origin.
    pub fn url(&self, path: &str) -> Result<String, LimitError> {
        check_path(path)?;
        let scheme = if self.secure { "https" } else { "http" };
        Ok(alloc::format!(
            "{scheme}://{}{}{path}",
            self.authority,
            self.base
        ))
    }
}

/// A host name, an IPv4 address or a bracketed IPv6 address, with an optional port.
fn authority_allowed(authority: &str) -> bool {
    let (host, port) = match authority.rsplit_once(':') {
        // An IPv6 address outside brackets: refused below by the host characters.
        Some((host, _)) if !host.ends_with(']') && host.contains(':') => (authority, None),
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let port_fine = port.is_none_or(|port| {
        !port.is_empty()
            && port.len() <= 5
            && port.bytes().all(|byte| byte.is_ascii_digit())
            && port
                .parse::<u32>()
                .is_ok_and(|port| (1..=65535).contains(&port))
    });
    let bracketed = host.starts_with('[') && host.ends_with(']');
    let host_fine = if bracketed {
        host.len() > 2
            && host[1..host.len() - 1]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
    } else {
        !host.is_empty()
            && host.len() <= 253
            && !host.starts_with(['.', '-'])
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
    };
    port_fine && host_fine
}

/// One request, ready to be sent: the whole address and the credential header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetCall {
    pub method: Method,
    /// The address below `origin`, built by [`Origin::url`].
    pub url: String,
    /// The origin the address must stay at; the adapter checks it again after parsing.
    pub origin: Origin,
    /// The body's bytes: JSON text, or raw bytes when `bytes` is set.
    pub body: Option<alloc::vec::Vec<u8>>,
    /// The body is raw bytes, and the answer comes back as base64.
    pub bytes: bool,
    /// The credential header and its value. Never logged.
    pub credential: Option<(String, String)>,
}

/// What came back: the status and the body (cut to [`MAX_RESPONSE`], lossily decoded), or
/// status 0 and the host's words for why nothing came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetAnswer {
    pub status: u16,
    pub body: String,
}

impl NetAnswer {
    /// No answer came, for `reason`.
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            status: 0,
            body: reason.into(),
        }
    }
}
