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

/// One open window as remembered.
///
/// Later versions add what a window of a hub needs to come back (its sessions, its
/// organization) as new defaulted fields of this record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowRecord {
    environment: Environment,
}

impl WindowRecord {
    pub const fn new(environment: Environment) -> Self {
        Self { environment }
    }

    pub const fn environment(&self) -> &Environment {
        &self.environment
    }
}

/// The saved list of open windows, in the order they were opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowDocument {
    windows: Vec<WindowRecord>,
}

impl WindowDocument {
    /// The remembered environments, in order.
    pub fn environments(&self) -> impl Iterator<Item = &Environment> {
        self.windows.iter().map(WindowRecord::environment)
    }

    pub fn records(&self) -> &[WindowRecord] {
        &self.windows
    }

    /// Reads a document; text that is not one, or a newer version, is an error.
    pub fn decode(text: &str) -> Result<Self, WindowDocumentError> {
        let saved: Saved = serde_json::from_str(text).map_err(WindowDocumentError::Unreadable)?;
        if saved.version > VERSION {
            return Err(WindowDocumentError::UnsupportedVersion {
                found: saved.version,
                supported: VERSION,
            });
        }
        Ok(Self {
            windows: saved
                .windows
                .into_iter()
                .map(|saved| WindowRecord::new(saved.environment.into()))
                .collect(),
        })
    }

    pub fn encode(&self) -> Result<String, WindowDocumentError> {
        let saved = Saved {
            version: VERSION,
            windows: self
                .windows
                .iter()
                .map(|record| SavedWindow {
                    environment: SavedEnvironment::from(record.environment()),
                })
                .collect(),
        };
        serde_json::to_string_pretty(&saved).map_err(WindowDocumentError::Unwritable)
    }
}

/// The windows that are open now, each under the key its host knows it by.
///
/// The list never becomes empty by closing: the last window stays, because closing it quits
/// the application and it is what the next start restores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWindows<K> {
    windows: Vec<(K, WindowRecord)>,
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
        match self.windows.iter_mut().find(|(open, _)| *open == key) {
            Some((_, current)) => *current = record,
            None => self.windows.push((key, record)),
        }
    }

    /// An open window now shows `environment`; a window not in the list is left out.
    pub fn switched(&mut self, key: K, environment: Environment) {
        if let Some((_, record)) = self.windows.iter_mut().find(|(open, _)| *open == key) {
            *record = WindowRecord::new(environment);
        }
    }

    /// The user closed a window. It is forgotten unless it is the last one.
    pub fn closed(&mut self, key: K) {
        if self.windows.len() > 1 {
            self.windows.retain(|(open, _)| *open != key);
        }
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// What is saved: the environments in the order the windows were opened.
    pub fn document(&self) -> WindowDocument {
        WindowDocument {
            windows: self
                .windows
                .iter()
                .map(|(_, record)| record.clone())
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    windows: Vec<SavedWindow>,
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
