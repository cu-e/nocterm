//! The `[network]` table: which settings hold the web address `net.request` reaches and the
//! credential the host sends with it.
//!
//! The origin is whatever the user types into the named text setting, so a plugin reaches only
//! the address its user chose. The credential is a secret setting the plugin never reads: the
//! host puts it into one request header, for that origin only.

use alloc::{collections::BTreeMap, string::String};

use nocterm_plugin_model::{FieldKind, SettingsSchema};
use serde::Serialize;
use thiserror::Error;

use crate::capability::{Capability as _, net::NetRequest};

/// Longest credential header name.
const MAX_HEADER: usize = 64;
/// Longest credential prefix, in characters.
const MAX_PREFIX: usize = 32;

/// The table as written. The manifest reads it as a table of text values, which
/// [`NetworkDto::try_from`] turns into this, refusing unknown keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct NetworkDto {
    /// The key of a text setting that holds the origin, such as `https://service.example.com`.
    pub origin_setting: String,
    /// The key of a secret text setting the host sends in `credential_header`.
    #[serde(default)]
    pub credential_setting: Option<String>,
    /// Default `Authorization`. Otherwise a header named `X-…`.
    #[serde(default)]
    pub credential_header: Option<String>,
    /// What comes before the credential in the header. Default `Bearer `.
    #[serde(default)]
    pub credential_prefix: Option<String>,
}

/// A `[network]` table that does not read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum NetworkTableError {
    #[error("missing field `origin_setting`")]
    NoOrigin,
    #[error("unknown field `{0}`")]
    Unknown(String),
}

impl TryFrom<BTreeMap<String, String>> for NetworkDto {
    type Error = NetworkTableError;

    fn try_from(mut table: BTreeMap<String, String>) -> Result<Self, NetworkTableError> {
        let dto = Self {
            origin_setting: table
                .remove("origin_setting")
                .ok_or(NetworkTableError::NoOrigin)?,
            credential_setting: table.remove("credential_setting"),
            credential_header: table.remove("credential_header"),
            credential_prefix: table.remove("credential_prefix"),
        };
        match table.into_keys().next() {
            Some(key) => Err(NetworkTableError::Unknown(key)),
            None => Ok(dto),
        }
    }
}

/// Where a package's requests go and what they carry, checked against its settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    origin_setting: String,
    credential: Option<Credential>,
}

/// The secret setting a request carries, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credential {
    setting: String,
    header: String,
    prefix: String,
}

/// A `[network]` table, or its absence, the host refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum NetworkError {
    #[error("the `[network]` table does not read")]
    Table(#[from] NetworkTableError),
    #[error("a package that declares `net.request` needs a `[network]` table")]
    Missing,
    #[error("a `[network]` table needs the `net.request` capability")]
    Unused,
    #[error("`origin_setting` names `{0}`, which is not a text setting that is not secret")]
    Origin(String),
    #[error("`credential_setting` names `{0}`, which is not a secret text setting")]
    Credential(String),
    #[error("the secret setting `{0}` is not the credential of the `[network]` table")]
    StraySecret(String),
    #[error("`credential_header` must be `Authorization` or a header named `X-…`")]
    Header,
    #[error("`credential_prefix` holds at most {MAX_PREFIX} visible characters and spaces")]
    Prefix,
}

impl Network {
    /// The key of the setting whose value is the origin of every request.
    pub fn origin_setting(&self) -> &str {
        &self.origin_setting
    }

    pub fn credential(&self) -> Option<&Credential> {
        self.credential.as_ref()
    }
}

impl Credential {
    /// The key of the secret setting.
    pub fn setting(&self) -> &str {
        &self.setting
    }

    pub fn header(&self) -> &str {
        &self.header
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// Checks the table against the declared capabilities and the settings schema.
pub(super) fn network(
    table: Option<&BTreeMap<String, String>>,
    requests: bool,
    schema: &SettingsSchema,
) -> Result<Option<Network>, NetworkError> {
    let dto = table.cloned().map(NetworkDto::try_from).transpose()?;
    let Some(dto) = dto.as_ref() else {
        if requests {
            return Err(NetworkError::Missing);
        }
        return match stray_secret(schema, None) {
            Some(key) => Err(NetworkError::StraySecret(key)),
            None => Ok(None),
        };
    };
    if !requests {
        return Err(NetworkError::Unused);
    }
    let text = |key: &str| {
        schema.field(key).and_then(|field| match field.kind {
            FieldKind::Text { secret, .. } => Some(secret),
            _ => None,
        })
    };
    if text(&dto.origin_setting) != Some(false) {
        return Err(NetworkError::Origin(dto.origin_setting.clone()));
    }
    let credential = match &dto.credential_setting {
        None => {
            if dto.credential_header.is_some() || dto.credential_prefix.is_some() {
                return Err(NetworkError::Credential(String::new()));
            }
            None
        }
        Some(key) => {
            if text(key) != Some(true) {
                return Err(NetworkError::Credential(key.clone()));
            }
            let header = dto
                .credential_header
                .clone()
                .unwrap_or_else(|| String::from("Authorization"));
            if !header_allowed(&header) {
                return Err(NetworkError::Header);
            }
            let prefix = dto
                .credential_prefix
                .clone()
                .unwrap_or_else(|| String::from("Bearer "));
            if prefix.chars().count() > MAX_PREFIX
                || !prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            {
                return Err(NetworkError::Prefix);
            }
            Some(Credential {
                setting: key.clone(),
                header,
                prefix,
            })
        }
    };
    if let Some(key) = stray_secret(schema, credential.as_ref()) {
        return Err(NetworkError::StraySecret(key));
    }
    Ok(Some(Network {
        origin_setting: dto.origin_setting.clone(),
        credential,
    }))
}

/// Whether a package declares `net.request`, required or optional.
pub(super) fn declares_requests<'a>(mut ids: impl Iterator<Item = &'a str>) -> bool {
    ids.any(|id| id == NetRequest::ID)
}

/// A secret setting that is not the credential: a package may not collect secrets it never
/// sends.
fn stray_secret(schema: &SettingsSchema, credential: Option<&Credential>) -> Option<String> {
    schema
        .fields()
        .filter(|field| field.kind.is_secret())
        .find(|field| credential.is_none_or(|credential| credential.setting != field.key))
        .map(|field| String::from(&*field.key))
}

/// `Authorization`, or a header of the `X-` family, in token characters.
fn header_allowed(header: &str) -> bool {
    let token = !header.is_empty()
        && header.len() <= MAX_HEADER
        && header
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    let named = header.eq_ignore_ascii_case("authorization")
        || header
            .get(..2)
            .is_some_and(|start| start.eq_ignore_ascii_case("x-"));
    token && named
}
