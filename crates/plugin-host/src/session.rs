//! Records and data kept in memory for the life of one session and never written anywhere.
//!
//! A developer session (`nocterm plugin dev`) composes these instead of the file-backed ports, so
//! the records, data and packages of a session are unsaved by construction: the host has no path
//! to a file. What else a session keeps in memory is the application's composition.

use std::{cell::RefCell, collections::BTreeMap};

use gpui::{App, Task};
use nocterm_plugin_model::PluginId;

use crate::{
    PluginData, PluginDataChange, PluginDataDoc, PluginDataError, PluginRecords, RecordsDoc,
    RecordsError,
};

/// A records port that keeps the document in memory.
#[derive(Default)]
pub struct SessionRecords {
    doc: RefCell<RecordsDoc>,
}

impl SessionRecords {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts from a document, as if an earlier session had saved it.
    pub fn seeded(doc: RecordsDoc) -> Self {
        Self {
            doc: RefCell::new(doc),
        }
    }

    /// The document last replaced.
    pub fn doc(&self) -> RecordsDoc {
        self.doc.borrow().clone()
    }
}

impl PluginRecords for SessionRecords {
    fn load(&self, _: &App) -> RecordsDoc {
        self.doc()
    }

    fn replace(&self, doc: RecordsDoc, _: &mut App) -> Task<Result<(), RecordsError>> {
        *self.doc.borrow_mut() = doc;
        Task::ready(Ok(()))
    }
}

/// A data port that keeps every plugin's document in memory.
#[derive(Default)]
pub struct SessionData {
    docs: RefCell<BTreeMap<PluginId, PluginDataDoc>>,
}

impl SessionData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts a document in the session, as if it had been written earlier.
    pub fn put(&self, plugin: &PluginId, doc: PluginDataDoc) {
        self.docs.borrow_mut().insert(plugin.clone(), doc);
    }

    /// The document of a plugin, if the session holds one.
    pub fn stored(&self, plugin: &PluginId) -> Option<PluginDataDoc> {
        self.docs.borrow().get(plugin).cloned()
    }
}

impl PluginData for SessionData {
    fn load(&self, plugin: &PluginId, _: &App) -> Result<PluginDataDoc, PluginDataError> {
        Ok(self.stored(plugin).unwrap_or_default())
    }

    fn change(
        &self,
        plugin: PluginId,
        change: PluginDataChange,
        _: &mut App,
    ) -> Result<(), PluginDataError> {
        match change {
            PluginDataChange::Replace(doc) => {
                self.docs.borrow_mut().insert(plugin, doc);
            }
            PluginDataChange::Remove => {
                self.docs.borrow_mut().remove(&plugin);
            }
        }
        Ok(())
    }
}
