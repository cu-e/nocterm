//! The installed plugins: what each one is, what the user allowed and whether it runs.

use std::collections::BTreeMap;

use nocterm_plugin_api::{
    ContentHash, Grant, Package, PluginInstance, PluginRuntime, RuntimeError,
};
use nocterm_plugin_model::{PluginId, SettingValues, resolve};

use crate::{
    BrokenPackage, PluginDataDoc, PluginDataError, PluginRecord, RecordsDoc,
    records::{AuditAction, AuditEntry, push_audit},
};

/// What a plugin's data file turned out to be when it was read.
pub(crate) enum DataState {
    Loaded(PluginDataDoc),
    /// The file could not be read. Defaults are in use and the file stays as it is until the
    /// user changes something.
    Broken {
        error: PluginDataError,
        defaults: PluginDataDoc,
    },
}

/// A plugin whose package matches what the user accepted.
pub(crate) struct Ready {
    package: Package,
    grant: Grant,
    instance: Option<Box<dyn PluginInstance>>,
    data: DataState,
    faulted: bool,
}

impl Ready {
    pub(crate) fn new(package: Package, grant: Grant, data: DataState) -> Self {
        Self {
            package,
            grant,
            instance: None,
            data,
            faulted: false,
        }
    }

    /// The running instance, created on first use.
    pub(crate) fn instance(
        &mut self,
        runtime: &dyn PluginRuntime,
    ) -> Result<&dyn PluginInstance, RuntimeError> {
        if self.instance.is_none() {
            self.instance = Some(runtime.instantiate(&self.package)?);
        }
        match &self.instance {
            Some(instance) => Ok(instance.as_ref()),
            None => Err(RuntimeError::Trap),
        }
    }

    pub(crate) fn package(&self) -> &Package {
        &self.package
    }

    pub(crate) fn grant(&self) -> &Grant {
        &self.grant
    }

    pub(crate) fn stop(&mut self) {
        self.instance = None;
    }

    pub(crate) fn faulted(&self) -> bool {
        self.faulted
    }

    pub(crate) fn set_faulted(&mut self, faulted: bool) {
        self.faulted = faulted;
        if faulted {
            self.stop();
        }
    }

    pub(crate) fn data(&self) -> &PluginDataDoc {
        match &self.data {
            DataState::Loaded(doc) => doc,
            DataState::Broken { defaults, .. } => defaults,
        }
    }

    /// The document a storage effect builds on, or why the plugin's file may not be written:
    /// a file that could not be read stays as it is until the user changes something.
    pub(crate) fn writable_data(&self) -> Result<&PluginDataDoc, PluginDataError> {
        match &self.data {
            DataState::Loaded(doc) => Ok(doc),
            DataState::Broken { error, .. } => Err(error.clone()),
        }
    }

    pub(crate) fn data_issue(&self) -> Option<&PluginDataError> {
        match &self.data {
            DataState::Loaded(_) => None,
            DataState::Broken { error, .. } => Some(error),
        }
    }

    /// Moves the data out, for a replacement that keeps it.
    pub(crate) fn take_data(&mut self) -> DataState {
        std::mem::replace(&mut self.data, DataState::Loaded(PluginDataDoc::default()))
    }

    pub(crate) fn set_data(&mut self, doc: PluginDataDoc) {
        self.data = DataState::Loaded(doc);
    }

    /// The setting values resolved against the plugin's schema.
    pub(crate) fn settings(&self) -> SettingValues {
        let schema = self.package.manifest().plugin().settings();
        resolve(schema, &self.data().setting_values()).0
    }
}

pub(crate) enum Status {
    Ready(Box<Ready>),
    /// The package is gone, changed or refused. The record stays so the user can see why.
    Broken(BrokenPackage),
}

pub(crate) struct Installed {
    record: PluginRecord,
    status: Status,
}

impl Installed {
    pub(crate) fn new(record: PluginRecord, status: Status) -> Self {
        Self { record, status }
    }

    pub(crate) fn record(&self) -> &PluginRecord {
        &self.record
    }

    pub(crate) fn record_mut(&mut self) -> &mut PluginRecord {
        &mut self.record
    }

    pub(crate) fn ready_mut(&mut self) -> Option<&mut Ready> {
        match &mut self.status {
            Status::Ready(ready) => Some(ready),
            Status::Broken(_) => None,
        }
    }
}

/// How a plugin shows in the plugins list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginStatus {
    Running,
    Disabled,
    /// Three traps or missed deadlines this session.
    Faulted,
    /// The stored package does not match what the user accepted, or cannot run.
    Broken(BrokenPackage),
}

/// One row of the plugins list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSummary {
    pub id: PluginId,
    pub name: String,
    pub hash: String,
    pub enabled: bool,
    pub placeable: bool,
    pub status: PluginStatus,
    /// Why the plugin's data file was not read and defaults are in use.
    pub data_issue: Option<PluginDataError>,
}

#[derive(Default)]
pub(crate) struct Registry {
    entries: BTreeMap<PluginId, Installed>,
    audit: Vec<AuditEntry>,
}

impl Registry {
    pub(crate) fn with_audit(audit: Vec<AuditEntry>) -> Self {
        Self {
            entries: BTreeMap::new(),
            audit,
        }
    }

    pub(crate) fn insert(&mut self, installed: Installed) {
        self.entries
            .insert(installed.record().id.clone(), installed);
    }

    pub(crate) fn remove(&mut self, plugin: &PluginId) -> Option<Installed> {
        self.entries.remove(plugin)
    }

    pub(crate) fn get(&self, plugin: &PluginId) -> Option<&Installed> {
        self.entries.get(plugin)
    }

    pub(crate) fn get_mut(&mut self, plugin: &PluginId) -> Option<&mut Installed> {
        self.entries.get_mut(plugin)
    }

    pub(crate) fn enabled(&self) -> usize {
        self.entries
            .values()
            .filter(|installed| installed.record().enabled)
            .count()
    }

    pub(crate) fn audit(&mut self, plugin: &PluginId, action: AuditAction) {
        push_audit(&mut self.audit, plugin, action);
    }

    pub(crate) fn to_doc(&self) -> RecordsDoc {
        RecordsDoc {
            plugins: self
                .entries
                .values()
                .map(|installed| installed.record().clone())
                .collect(),
            audit: self.audit.clone(),
        }
    }

    pub(crate) fn summaries(&self) -> Vec<PluginSummary> {
        self.entries.values().map(summary).collect()
    }

    /// The plugin if it is on and its package is the one in `hash`.
    pub(crate) fn running(
        &mut self,
        plugin: &PluginId,
        hash: Option<&ContentHash>,
    ) -> Option<&mut Ready> {
        let installed = self.entries.get_mut(plugin)?;
        if !installed.record().enabled {
            return None;
        }
        installed.ready_mut().filter(|ready| {
            !ready.faulted() && hash.is_none_or(|hash| ready.package().hash() == hash)
        })
    }
}

fn summary(installed: &Installed) -> PluginSummary {
    let record = installed.record();
    let (name, status, data_issue) = match &installed.status {
        Status::Ready(ready) => {
            let status = if ready.faulted() {
                PluginStatus::Faulted
            } else if record.enabled {
                PluginStatus::Running
            } else {
                PluginStatus::Disabled
            };
            (
                ready.package().manifest().plugin().name().to_owned(),
                status,
                ready.data_issue().cloned(),
            )
        }
        Status::Broken(reason) => (
            record.id.as_str().to_owned(),
            PluginStatus::Broken(reason.clone()),
            None,
        ),
    };
    PluginSummary {
        id: record.id.clone(),
        name,
        hash: record.hash.clone(),
        enabled: record.enabled,
        placeable: record.placeable,
        status,
        data_issue,
    }
}
