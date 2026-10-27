//! The `[hubs]` section: the servers this device knows. It holds no secrets.
//!
//! ```toml
//! [[hubs.servers]]
//! address = "hub.home:7443"
//! fingerprint = "ab:cd:…"   # pins a self-signed certificate
//! label = "Home"
//!
//! [[hubs.servers]]
//! address = "hub.example.com"
//! trust = "public-ca"       # any certificate a public authority vouches for
//! ```

use std::collections::HashSet;

use nocterm_settings::SettingsSection;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::address::{HubAddress, HubId};
use crate::trust::{Fingerprint, TrustPolicy};

/// Longest label kept, in characters.
const MAX_LABEL_CHARS: usize = 64;

/// One known server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "RawRecord", into = "RawRecord")]
#[schemars(with = "RawRecord")]
pub struct HubRecord {
    pub address: HubAddress,
    pub trust: TrustPolicy,
    /// How the server is named in menus; empty shows the address.
    pub label: String,
}

impl HubRecord {
    pub fn new(address: HubAddress, trust: TrustPolicy, label: impl Into<String>) -> Self {
        Self {
            address,
            trust,
            label: clean_label(&label.into()),
        }
    }

    pub fn id(&self) -> HubId {
        HubId::from(self.address.clone())
    }

    /// The label, or the address when there is none.
    pub fn display_label(&self) -> String {
        if self.label.is_empty() {
            self.address.to_string()
        } else {
            self.label.clone()
        }
    }
}

fn clean_label(label: &str) -> String {
    let printable: String = label.chars().filter(|c| !c.is_control()).collect();
    printable.trim().chars().take(MAX_LABEL_CHARS).collect()
}

/// The shape of one `[[hubs.servers]]` table.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct RawRecord {
    /// The server as `host` or `host:port`; the port defaults to 7443.
    address: String,
    /// SHA-256 fingerprint of the server's certificate, in hex with or without colons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
    /// `public-ca` trusts certificates that public authorities vouch for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trust: Option<RawTrust>,
    /// How the server is named in menus.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    label: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum RawTrust {
    PublicCa,
}

/// Why a table is not a usable server.
#[derive(Debug, thiserror::Error)]
enum RecordError {
    #[error("invalid address")]
    Address(#[from] crate::error::HubAddressError),
    #[error("invalid fingerprint")]
    Fingerprint(#[from] crate::error::FingerprintError),
    #[error("neither a fingerprint nor `trust = \"public-ca\"`")]
    NoTrust,
}

impl TryFrom<RawRecord> for HubRecord {
    type Error = RecordError;

    fn try_from(raw: RawRecord) -> Result<Self, Self::Error> {
        // A fingerprint is the stricter statement, so it wins over `trust`.
        let trust = match (raw.fingerprint, raw.trust) {
            (Some(fingerprint), _) => TrustPolicy::Pinned(Fingerprint::parse(&fingerprint)?),
            (None, Some(RawTrust::PublicCa)) => TrustPolicy::PublicCa,
            (None, None) => return Err(RecordError::NoTrust),
        };
        Ok(Self::new(
            HubAddress::parse(&raw.address)?,
            trust,
            raw.label,
        ))
    }
}

impl From<HubRecord> for RawRecord {
    fn from(record: HubRecord) -> Self {
        let (fingerprint, trust) = match record.trust {
            TrustPolicy::Pinned(fingerprint) => (Some(fingerprint.to_string()), None),
            TrustPolicy::PublicCa => (None, Some(RawTrust::PublicCa)),
        };
        Self {
            address: record.address.to_string(),
            fingerprint,
            trust,
            label: record.label,
        }
    }
}

/// The servers this device has signed in to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HubsSettings {
    /// Known servers in the order they were added. A table that is not a usable server is
    /// ignored without affecting the others.
    #[serde(deserialize_with = "lenient_servers")]
    pub servers: Vec<HubRecord>,
}

/// Reads every table that is a usable server and skips the rest.
fn lenient_servers<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<HubRecord>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Server(HubRecord),
        Unusable(serde::de::IgnoredAny),
    }
    let entries = Vec::<Entry>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Server(record) => Some(record),
            Entry::Unusable(_) => None,
        })
        .collect())
}

impl HubsSettings {
    pub fn find(&self, hub: &HubId) -> Option<&HubRecord> {
        self.servers.iter().find(|record| record.id() == *hub)
    }

    /// Adds a server, or replaces the one with the same address in place.
    pub fn upsert(&mut self, record: HubRecord) {
        match self
            .servers
            .iter_mut()
            .find(|existing| existing.address == record.address)
        {
            Some(existing) => *existing = record,
            None => self.servers.push(record),
        }
    }

    /// Forgets a server; `true` when it was known.
    pub fn remove(&mut self, hub: &HubId) -> bool {
        let before = self.servers.len();
        self.servers.retain(|record| record.id() != *hub);
        self.servers.len() != before
    }
}

impl SettingsSection for HubsSettings {
    const KEY: &'static str = "hubs";

    /// Keeps the first server of each address and shortens labels.
    fn sanitize(&mut self) {
        let mut seen = HashSet::new();
        self.servers.retain(|record| seen.insert(record.id()));
        for record in &mut self.servers {
            record.label = clean_label(&record.label);
        }
    }
}

#[cfg(test)]
mod tests;
