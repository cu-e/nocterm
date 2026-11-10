//! The installed plugins: what each one is, what the user allowed and whether it runs.

use std::collections::BTreeMap;

use nocterm_plugin_api::{
    ContentHash, Grant, Package, PluginInstance, PluginRuntime, RuntimeError,
};
use nocterm_plugin_model::{PluginId, SettingValues, resolve};

use crate::{
    BrokenPackage, ManageError, PluginDataDoc, PluginDataError, PluginRecord, RecordsDoc,
    records::{AuditAction, AuditEntry, push_audit},
};

/// Original records and background-acquired package/data results, delivered together.
pub struct RestoredPlugin {
    record: PluginRecord,
    package: Result<crate::PreparedPlugin, BrokenPackage>,
    data: Result<PluginDataDoc, PluginDataError>,
}
impl RestoredPlugin {
    pub fn new(
        record: PluginRecord,
        package: Result<crate::PreparedPlugin, BrokenPackage>,
        data: Result<PluginDataDoc, PluginDataError>,
    ) -> Self {
        Self {
            record,
            package,
            data,
        }
    }
    pub(crate) fn restrict_enabled(&mut self) -> Option<PluginId> {
        if !self.record.enabled {
            return None;
        }
        self.record.enabled = false;
        Some(self.record.id.clone())
    }
    pub(crate) fn into_installed(self) -> Installed {
        let status = self.package.and_then(|prepared| {
            let (package, instance) = prepared.into_parts();
            if package.hash().to_string() != self.record.hash
                || package.manifest().plugin().id() != &self.record.id
            {
                return Err(nocterm_plugin_api::PackageError::HashChanged.into());
            }
            let accepted: Vec<_> = self
                .record
                .granted_optional
                .iter()
                .map(String::as_str)
                .collect();
            let grant = Grant::consent(&package, &accepted)?;
            let data = match self
                .data
                .and_then(|doc| doc.check().map(|()| doc).map_err(PluginDataError::from))
            {
                Ok(doc) => DataState::Loaded(doc),
                Err(error) => DataState::Broken {
                    error,
                    defaults: PluginDataDoc::default(),
                },
            };
            Ok(
                Ready::new(package, grant, data).with_instance(if self.record.enabled {
                    instance
                } else {
                    None
                }),
            )
        });
        Installed::new(
            self.record,
            match status {
                Ok(ready) => Status::Ready(Box::new(ready)),
                Err(error) => Status::Broken(error),
            },
        )
    }
}

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

    pub(crate) fn with_instance(mut self, instance: Option<Box<dyn PluginInstance>>) -> Self {
        self.instance = instance;
        self
    }

    pub(crate) fn start_prepared(&mut self, instance: Box<dyn PluginInstance>) {
        self.instance = Some(instance);
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
    /// `values` to store, with the credential forgotten if the address it was for changed.
    pub(crate) fn bound(&self, values: &SettingValues) -> SettingValues {
        crate::net::bind_credential(self.package.manifest(), &self.settings(), values.clone())
    }

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
    data_held: bool,
}

impl Installed {
    pub(crate) fn new(record: PluginRecord, status: Status) -> Self {
        Self {
            record,
            status,
            data_held: false,
        }
    }

    pub(crate) fn record(&self) -> &PluginRecord {
        &self.record
    }

    pub(crate) fn record_mut(&mut self) -> &mut PluginRecord {
        &mut self.record
    }

    pub(crate) fn ready(&self) -> Option<&Ready> {
        match &self.status {
            Status::Ready(ready) => Some(ready),
            Status::Broken(_) => None,
        }
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
    revision: u64,
}

impl Registry {
    pub(crate) fn with_audit(audit: Vec<AuditEntry>) -> Self {
        Self {
            entries: BTreeMap::new(),
            audit,
            revision: 0,
        }
    }

    /// Counts a change the shell must learn of.
    pub(crate) fn touch(&mut self) {
        self.revision += 1;
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn audit_log(&self) -> &[AuditEntry] {
        &self.audit
    }

    pub(crate) fn installed(&self) -> impl Iterator<Item = &Installed> {
        self.entries.values()
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

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
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

    pub(crate) fn hold_data(&mut self, plugin: &PluginId, held: bool) -> Result<(), ManageError> {
        self.entries
            .get_mut(plugin)
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?
            .data_held = held;
        self.touch();
        Ok(())
    }
    pub(crate) fn data_held(&self, plugin: &PluginId) -> bool {
        self.entries
            .get(plugin)
            .is_some_and(|installed| installed.data_held)
    }
    pub(crate) fn clear_data(&mut self, plugin: &PluginId) -> Result<(), ManageError> {
        let installed = self
            .entries
            .get_mut(plugin)
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?;
        if let Some(ready) = installed.ready_mut() {
            ready.set_data(PluginDataDoc::default());
        }
        self.audit(plugin, AuditAction::DataRemoved);
        Ok(())
    }

    /// The plugin if it is on and its package is the one in `hash`.
    pub(crate) fn running(
        &mut self,
        plugin: &PluginId,
        hash: Option<&ContentHash>,
    ) -> Option<&mut Ready> {
        let installed = self.entries.get_mut(plugin)?;
        if !installed.record().enabled || installed.data_held {
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
