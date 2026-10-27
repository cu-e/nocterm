//! The environments of the open windows: what is remembered across a restart, and the rules
//! that keep it up to date.
//!
//! [`WindowDocument`] is the saved form (versioned JSON). [`OpenWindows`] is the live list the
//! application edits while windows open, switch and close; it is generic over the key the host
//! identifies a window by, so this module needs no windowing system.

use serde::{Deserialize, Serialize};

use crate::{address::HubAddress, environment::Environment, error::WindowDocumentError};

/// The version this application writes and reads.
const VERSION: u32 = 1;

/// One window as remembered.
///
/// Later versions add what a window of a hub needs to come back (its sessions, its
/// organization) as new defaulted fields of this record.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowRecord {
    environment: Environment,
}

impl WindowRecord {
    const fn new(environment: Environment) -> Self {
        Self { environment }
    }

    const fn environment(&self) -> &Environment {
        &self.environment
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
            .filter_map(|record| serde_json::from_value::<SavedWindow>(record).ok())
            .map(|saved| WindowRecord::new(saved.environment.into()))
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
                .map(|record| SavedWindow {
                    environment: SavedEnvironment::from(record.environment()),
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
        let record = WindowRecord::new(environment);
        match self.windows.iter_mut().find(|(open, _)| *open == Some(key)) {
            Some((_, current)) => *current = record,
            None => self.windows.push((Some(key), record)),
        }
    }

    /// A remembered environment that could not open in this run, such as a server whose list
    /// could not be read. It stays in the saved document, at its place, so a later run can
    /// open it; it is not a window and closing windows never touches it.
    pub fn kept(&mut self, environment: Environment) {
        self.windows.push((None, WindowRecord::new(environment)));
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

#[derive(Serialize)]
struct Written {
    version: u32,
    windows: Vec<SavedWindow>,
}

#[derive(Deserialize)]
struct Saved {
    #[serde(default)]
    version: u32,
    /// Decoded one by one, see [`WindowDocument::decode`].
    #[serde(default)]
    windows: Vec<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct SavedWindow {
    environment: SavedEnvironment,
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
