//! The environments of the open windows: what is remembered across a restart, and the rules
//! that keep it up to date.
//!
//! [`WindowDocument`] is the saved form (versioned JSON). [`OpenWindows`] is the live list the
//! application edits while windows open, switch and close; it is generic over the key the host
//! identifies a window by, so this module needs no windowing system.

use serde::{Deserialize, Serialize};

use crate::{
    address::{HubAddress, HubId},
    environment::Environment,
    error::WindowDocumentError,
    records::{HubSessionId, OrgId},
};

/// The version this application writes and reads.
const VERSION: u32 = 1;

/// What a window of a hub keeps beyond its server: the organization it shows and the sessions
/// its tabs are attached to, in the order the tabs opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HubWindowMemory {
    organization: Option<OrgId>,
    sessions: Vec<HubSessionId>,
}

impl HubWindowMemory {
    pub const fn new(organization: Option<OrgId>, sessions: Vec<HubSessionId>) -> Self {
        Self {
            organization,
            sessions,
        }
    }

    /// The organization the window showed, if it had settled on one.
    pub const fn organization(&self) -> Option<&OrgId> {
        self.organization.as_ref()
    }

    /// The sessions the window's tabs were attached to.
    pub fn sessions(&self) -> &[HubSessionId] {
        &self.sessions
    }

    pub fn is_empty(&self) -> bool {
        self.organization.is_none() && self.sessions.is_empty()
    }
}

/// One window as remembered: its environment and, for a hub, what the window kept of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowRecord {
    environment: Environment,
    hub: HubWindowMemory,
}

impl WindowRecord {
    pub const fn new(environment: Environment) -> Self {
        Self {
            environment,
            hub: HubWindowMemory {
                organization: None,
                sessions: Vec::new(),
            },
        }
    }

    /// The same window with what it kept of its hub; a window of this computer keeps nothing.
    #[must_use]
    pub fn with_hub(mut self, hub: HubWindowMemory) -> Self {
        if matches!(self.environment, Environment::Hub(_)) {
            self.hub = hub;
        }
        self
    }

    pub const fn environment(&self) -> &Environment {
        &self.environment
    }

    /// What the window kept of its hub; empty for a window of this computer.
    pub const fn hub(&self) -> &HubWindowMemory {
        &self.hub
    }
}

impl From<Environment> for WindowRecord {
    fn from(environment: Environment) -> Self {
        Self::new(environment)
    }
}

/// The saved list of open windows, in the order they were opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowDocument {
    windows: Vec<WindowRecord>,
    /// How many records of the text this version could not read and left out.
    skipped: usize,
}

impl WindowDocument {
    /// The remembered windows, in order.
    pub fn records(&self) -> &[WindowRecord] {
        &self.windows
    }

    /// The remembered environments, in order.
    pub fn environments(&self) -> impl Iterator<Item = &Environment> {
        self.windows.iter().map(WindowRecord::environment)
    }

    /// How many records of the text were left out because this version cannot read them, such
    /// as an environment of a kind a later build added. The others are kept.
    pub const fn skipped(&self) -> usize {
        self.skipped
    }

    /// Reads a document; text that is not one, or a newer version, is an error. A record that
    /// is not readable is left out and counted, so one record never costs the others.
    pub fn decode(text: &str) -> Result<Self, WindowDocumentError> {
        let saved: Saved = serde_json::from_str(text).map_err(WindowDocumentError::Unreadable)?;
        if saved.version > VERSION {
            return Err(WindowDocumentError::UnsupportedVersion {
                found: saved.version,
                supported: VERSION,
            });
        }
        let total = saved.windows.len();
        let windows: Vec<_> = saved
            .windows
            .into_iter()
            .filter_map(|record| serde_json::from_value::<ReadWindow>(record).ok())
            .map(ReadWindow::into_record)
            .collect();
        Ok(Self {
            skipped: total - windows.len(),
            windows,
        })
    }

    pub fn encode(&self) -> Result<String, WindowDocumentError> {
        let written = Written {
            version: VERSION,
            windows: self
                .windows
                .iter()
                .map(|record| WrittenWindow {
                    environment: SavedEnvironment::from(record.environment()),
                    hub: SavedHub::of(record.hub()),
                })
                .collect(),
        };
        serde_json::to_string_pretty(&written).map_err(WindowDocumentError::Unwritable)
    }
}

/// The windows that are open now, each under the key its host knows it by.
///
/// The list never becomes empty by closing: the last window stays, because closing it quits
/// the application and it is what the next start restores.
///
/// It can also hold records that are not windows of this run, see [`OpenWindows::kept`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWindows<K> {
    /// A record with no key is kept, not open.
    windows: Vec<(Option<K>, WindowRecord)>,
}

impl<K> Default for OpenWindows<K> {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
        }
    }
}

impl<K: Copy + PartialEq> OpenWindows<K> {
    /// Adds a window after the others; a key that is open already takes the new environment.
    pub fn opened(&mut self, key: K, environment: Environment) {
        self.opened_record(key, WindowRecord::new(environment));
    }

    /// As [`OpenWindows::opened`], for a window that comes back with what it kept of its hub.
    pub fn restored(&mut self, key: K, record: WindowRecord) {
        self.opened_record(key, record);
    }

    fn opened_record(&mut self, key: K, record: WindowRecord) {
        match self.windows.iter_mut().find(|(open, _)| *open == Some(key)) {
            Some((_, current)) => *current = record,
            None => self.windows.push((Some(key), record)),
        }
    }

    /// A remembered window that could not open in this run, such as a server whose list
    /// could not be read. It stays in the saved document, at its place, so a later run can
    /// open it; it is not a window and closing windows never touches it.
    pub fn kept(&mut self, record: impl Into<WindowRecord>) {
        self.windows.push((None, record.into()));
    }

    /// What the open window `key` of `hub` keeps of that hub now. Ignored when the window is
    /// not open or no longer shows `hub`, so a window switched to another server is not given
    /// what the first one reported late.
    pub fn remembered(&mut self, key: K, hub: &HubId, memory: HubWindowMemory) {
        let found = self
            .windows
            .iter_mut()
            .find(|(open, record)| *open == Some(key) && shows(record.environment(), hub));
        if let Some((_, record)) = found {
            record.hub = memory;
        }
    }

    /// An open window now shows `environment`; a window not in the list is left out.
    pub fn switched(&mut self, key: K, environment: Environment) {
        if let Some((_, record)) = self.windows.iter_mut().find(|(open, _)| *open == Some(key)) {
            *record = WindowRecord::new(environment);
        }
    }

    /// The user closed a window. It is forgotten unless it is the last one.
    pub fn closed(&mut self, key: K) {
        if self.len() > 1 {
            self.windows.retain(|(open, _)| *open != Some(key));
        }
    }

    /// How many windows are open; kept records are not windows.
    pub fn len(&self) -> usize {
        self.windows
            .iter()
            .filter(|(open, _)| open.is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// What is saved: the environments in the order the windows were opened, with the kept
    /// ones where they were.
    pub fn document(&self) -> WindowDocument {
        WindowDocument {
            windows: self
                .windows
                .iter()
                .map(|(_, record)| record.clone())
                .collect(),
            skipped: 0,
        }
    }
}

/// Whether `environment` is the server `hub`.
fn shows(environment: &Environment, hub: &HubId) -> bool {
    matches!(environment, Environment::Hub(shown) if shown == hub)
}

#[derive(Serialize)]
struct Written {
    version: u32,
    windows: Vec<WrittenWindow>,
}

#[derive(Deserialize)]
struct Saved {
    #[serde(default)]
    version: u32,
    /// Decoded one by one, see [`WindowDocument::decode`].
    #[serde(default)]
    windows: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct WrittenWindow {
    environment: SavedEnvironment,
    #[serde(skip_serializing_if = "Option::is_none")]
    hub: Option<SavedHub>,
}

#[derive(Deserialize)]
struct ReadWindow {
    environment: SavedEnvironment,
    /// Read apart from the record: what a window kept never costs the window itself.
    #[serde(default)]
    hub: serde_json::Value,
}

impl ReadWindow {
    fn into_record(self) -> WindowRecord {
        let hub = serde_json::from_value::<SavedHub>(self.hub).unwrap_or_default();
        WindowRecord::new(self.environment.into()).with_hub(hub.into())
    }
}

/// What a window of a hub kept, as saved. Every field may be missing.
#[derive(Default, Serialize, Deserialize)]
struct SavedHub {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    organization: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sessions: Vec<String>,
}

impl SavedHub {
    /// Nothing is written for a window that kept nothing.
    fn of(memory: &HubWindowMemory) -> Option<Self> {
        (!memory.is_empty()).then(|| Self {
            organization: memory.organization().map(|org| org.as_str().to_owned()),
            sessions: memory
                .sessions()
                .iter()
                .map(|session| session.as_str().to_owned())
                .collect(),
        })
    }
}

impl From<SavedHub> for HubWindowMemory {
    fn from(saved: SavedHub) -> Self {
        Self::new(
            saved.organization.map(OrgId::new),
            saved.sessions.into_iter().map(HubSessionId::new).collect(),
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SavedEnvironment {
    Local,
    Hub { hub: HubAddress },
}

impl From<&Environment> for SavedEnvironment {
    fn from(environment: &Environment) -> Self {
        match environment {
            Environment::Local => Self::Local,
            Environment::Hub(hub) => Self::Hub {
                hub: hub.address().clone(),
            },
        }
    }
}

impl From<SavedEnvironment> for Environment {
    fn from(saved: SavedEnvironment) -> Self {
        match saved {
            SavedEnvironment::Local => Self::Local,
            SavedEnvironment::Hub { hub } => Self::Hub(hub.into()),
        }
    }
}

#[cfg(test)]
mod tests;
