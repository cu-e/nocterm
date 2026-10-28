//! The `[hubs]` section: the servers this device knows. It holds no secrets.
//!
//! ```toml
//! [[hubs.servers]]
//! address = "hub.home:7443"
//! fingerprint = "ab:cd:…"   # pins a self-signed certificate
//! label = "Home"
//! color = "blue"            # optional: red, yellow, green, cyan, blue or magenta
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
use crate::color::HubColor;
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
    /// The colour that marks the server in menus; none leaves it unmarked.
    pub color: Option<HubColor>,
}

impl HubRecord {
    pub fn new(address: HubAddress, trust: TrustPolicy, label: impl Into<String>) -> Self {
        Self {
            address,
            trust,
            label: clean_label(&label.into()),
            color: None,
        }
    }

    /// The record marked with `color`.
    pub fn with_color(mut self, color: Option<HubColor>) -> Self {
        self.color = color;
        self
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
    /// `red`, `yellow`, `green`, `cyan`, `blue` or `magenta`. A name this version does not know
    /// leaves the server unmarked.
    #[serde(
        default,
        deserialize_with = "lenient_color",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<HubColor>")]
    color: Option<HubColor>,
}

/// Reads a colour this version knows; anything else is no colour, so a newer file does not
/// make the server unusable.
fn lenient_color<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<HubColor>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Known(HubColor),
        Unusable(serde::de::IgnoredAny),
    }
    Ok(match Entry::deserialize(deserializer)? {
        Entry::Known(color) => Some(color),
        Entry::Unusable(_) => None,
    })
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
        Ok(Self::new(HubAddress::parse(&raw.address)?, trust, raw.label).with_color(raw.color))
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
            color: record.color,
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
    /// The server whose account keeps this device's profiles and vault in step with its other
    /// devices, as `host:port`. Empty keeps both on this device only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
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

    /// Renames a server; empty shows its address. `true` when it was known.
    pub fn set_label(&mut self, hub: &HubId, label: &str) -> bool {
        self.edit(hub, |record| record.label = clean_label(label))
    }

    /// Marks a server with a colour, or clears the mark. `true` when it was known.
    pub fn set_color(&mut self, hub: &HubId, color: Option<HubColor>) -> bool {
        self.edit(hub, |record| record.color = color)
    }

    fn edit(&mut self, hub: &HubId, change: impl FnOnce(&mut HubRecord)) -> bool {
        match self.servers.iter_mut().find(|record| record.id() == *hub) {
            Some(record) => {
                change(record);
                true
            }
            None => false,
        }
    }

    /// Forgets a server; `true` when it was known. A forgotten account server is no longer
    /// the account server.
    pub fn remove(&mut self, hub: &HubId) -> bool {
        if self.account_hub().as_ref() == Some(hub) {
            self.account = None;
        }
        let before = self.servers.len();
        self.servers.retain(|record| record.id() != *hub);
        self.servers.len() != before
    }

    /// The server that keeps the account's profiles and vault, when one is chosen and known.
    pub fn account_hub(&self) -> Option<HubId> {
        let address = HubAddress::parse(self.account.as_deref()?).ok()?;
        let hub = HubId::from(address);
        self.find(&hub).map(HubRecord::id)
    }

    /// Chooses the account server, or none. `true` when it was known (or none was asked).
    pub fn set_account_hub(&mut self, hub: Option<&HubId>) -> bool {
        match hub {
            None => {
                self.account = None;
                true
            }
            Some(hub) if self.find(hub).is_some() => {
                self.account = Some(hub.address().to_string());
                true
            }
            Some(_) => false,
        }
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
        if self.account.is_some() && self.account_hub().is_none() {
            self.account = None;
        }
    }
}

#[cfg(test)]
mod tests;
