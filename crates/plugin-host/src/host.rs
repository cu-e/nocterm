//! The host: installed plugins, their calls and the effects they ask for.

use std::{rc::Rc, sync::Arc};

use gpui::{Context, WindowId};
use nocterm_plugin_api::{
    ContentHash, ContributionContext, Effects, Event, Grant, LocalId, Manifest, Package,
    PackageError, PluginPackages, PluginRuntime, Runtime, RuntimeError, View,
};
use nocterm_plugin_model::{PluginId, SettingValues, UserEntry};
use nocterm_plugin_ports::PluginProviders;

use crate::{
    AuditEntry, BrokenPackage, CallError, ManageError, PluginData, PluginDataChange, PluginDataDoc,
    PluginDataError, PluginRecord, PluginRecords, PluginSummary,
    broker::{Broker, Queued},
    cadence::Cadence,
    caps::MAX_ENABLED,
    faults::Faults,
    gesture::Input,
    records::AuditAction,
    registry::{DataState, Installed, Ready, Registry, Status},
    snapshot::{Subject, assemble},
};

/// What the host is built from. The application composes these; the host never looks them up.
pub struct HostPorts {
    pub records: Rc<dyn PluginRecords>,
    pub data: Rc<dyn PluginData>,
    pub packages: Arc<dyn PluginPackages>,
    pub runtime: Arc<dyn PluginRuntime>,
}

/// One contribution of one plugin in one window, asking for a view or reporting an event.
#[derive(Clone, Debug)]
pub struct Call {
    pub window: WindowId,
    pub plugin: PluginId,
    pub local: LocalId,
    pub context: ContributionContext,
    /// Whether the contribution's panel has focus, which samples at the faster cadence.
    pub in_focus: bool,
}

/// The user plugin host, one per application.
pub struct PluginHost {
    records: Rc<dyn PluginRecords>,
    data: Rc<dyn PluginData>,
    packages: Arc<dyn PluginPackages>,
    runtime: Arc<dyn PluginRuntime>,
    registry: Registry,
    broker: Broker,
    cadence: Cadence,
    faults: Faults,
}

impl PluginHost {
    pub fn new(ports: HostPorts) -> Self {
        Self {
            records: ports.records,
            data: ports.data,
            packages: ports.packages,
            runtime: ports.runtime,
            registry: Registry::default(),
            broker: Broker::default(),
            cadence: Cadence::default(),
            faults: Faults::default(),
        }
    }

    /// Reads the records and loads each plugin's stored package. A package that no longer has
    /// the hash the user accepted is listed as broken and does not run.
    pub fn restore(&mut self, cx: &mut Context<Self>) {
        let doc = self.records.load(cx).bounded();
        let mut registry = Registry::with_audit(doc.audit);
        let mut trimmed = false;
        for mut record in doc.plugins {
            if record.enabled && registry.enabled() >= MAX_ENABLED {
                record.enabled = false;
                registry.audit(&record.id, AuditAction::Disabled);
                trimmed = true;
            }
            let status = self.load_status(&record, cx);
            registry.insert(Installed::new(record, status));
        }
        registry.touch();
        self.registry = registry;
        if trimmed {
            self.save(cx);
        }
    }

    /// The plugins, for the plugins list.
    pub fn plugins(&self) -> Vec<PluginSummary> {
        self.registry.summaries()
    }

    /// Counts every change of what is installed, enabled, placeable or valued, so a reader can
    /// tell that its copy is stale.
    pub fn revision(&self) -> u64 {
        self.registry.revision()
    }

    /// The plugins the shell can place: every installed package that matches what the user
    /// accepted. A plugin that faulted this session is listed as off.
    pub fn entries(&self) -> Vec<UserEntry> {
        self.registry
            .installed()
            .filter_map(|installed| {
                let ready = installed.ready()?;
                Some(UserEntry {
                    plugin: ready.package().manifest().plugin().clone(),
                    enabled: installed.record().enabled && !ready.faulted(),
                    placeable: installed.record().placeable,
                })
            })
            .collect()
    }

    /// The manifest of an installed plugin whose package matches what the user accepted.
    pub fn manifest(&self, plugin: &PluginId) -> Option<&Manifest> {
        self.registry
            .get(plugin)?
            .ready()
            .map(|ready| ready.package().manifest())
    }

    /// The sha256 the user consented to, in hexadecimal.
    pub fn hash_of(&self, plugin: &PluginId) -> Option<String> {
        self.registry
            .get(plugin)
            .map(|installed| installed.record().hash.clone())
    }

    /// Every capability the user allowed the plugin to use.
    pub fn granted(&self, plugin: &PluginId) -> Vec<String> {
        self.registry
            .get(plugin)
            .and_then(Installed::ready)
            .map(|ready| ready.grant().granted().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// The plugin's setting values, resolved against its schema.
    pub fn values(&self, plugin: &PluginId) -> Option<SettingValues> {
        self.registry.get(plugin)?.ready().map(Ready::settings)
    }

    /// What happened to plugins, oldest first.
    pub fn audit(&self) -> &[AuditEntry] {
        self.registry.audit_log()
    }

    /// Reads and checks the package at `location` without storing anything.
    pub fn read_package(&self, location: &str) -> Result<Package, PackageError> {
        self.packages.read(location)
    }

    /// Installs a package the user consented to, with the optional capabilities they accepted.
    pub fn install(
        &mut self,
        package: Package,
        accepted: &[&str],
        cx: &mut Context<Self>,
    ) -> Result<PluginId, ManageError> {
        let id = package.manifest().plugin().id().clone();
        if self.registry.get(&id).is_some() {
            return Err(ManageError::Installed(id));
        }
        if self.registry.enabled() >= MAX_ENABLED {
            return Err(ManageError::TooManyEnabled { max: MAX_ENABLED });
        }
        let grant = Grant::consent(&package, accepted)?;
        self.packages.publish(&package, false)?;
        let record = record_of(&package, accepted, true);
        let data = self.read_data(&id, cx);
        let status = Status::Ready(Box::new(Ready::new(package, grant, data)));
        self.registry.insert(Installed::new(record, status));
        self.registry.audit(&id, AuditAction::Installed);
        self.save(cx);
        Ok(id)
    }

    /// Replaces an installed plugin by a new package of the same id. The new hash starts not
    /// placeable. The plugin's data is kept only when `keep_data` is set.
    pub fn replace(
        &mut self,
        package: Package,
        accepted: &[&str],
        keep_data: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        let id = package.manifest().plugin().id().clone();
        let Some(installed) = self.registry.get(&id) else {
            return Err(ManageError::Unknown(id));
        };
        let enabled = installed.record().enabled;
        let grant = Grant::consent(&package, accepted)?;
        self.packages.publish(&package, true)?;
        self.halt(&id);
        let data = if keep_data {
            let kept = self
                .registry
                .get_mut(&id)
                .and_then(Installed::ready_mut)
                .map(Ready::take_data);
            kept.unwrap_or_else(|| self.read_data(&id, cx))
        } else {
            self.drop_data(&id, cx);
            DataState::Loaded(PluginDataDoc::default())
        };
        let mut record = record_of(&package, accepted, enabled);
        record.placeable = false;
        let status = Status::Ready(Box::new(Ready::new(package, grant, data)));
        self.registry.insert(Installed::new(record, status));
        self.registry.audit(&id, AuditAction::Replaced);
        self.save(cx);
        Ok(())
    }

    /// Removes a plugin and its package; its data goes too when `remove_data` is set.
    pub fn uninstall(
        &mut self,
        plugin: &PluginId,
        remove_data: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        if self.registry.get(plugin).is_none() {
            return Err(ManageError::Unknown(plugin.clone()));
        }
        self.packages.remove(plugin)?;
        self.halt(plugin);
        self.broker.forget(plugin);
        self.registry.remove(plugin);
        if remove_data {
            self.drop_data(plugin, cx);
        }
        self.registry.audit(plugin, AuditAction::Removed);
        self.save(cx);
        Ok(())
    }

    /// Turns a plugin on or off. Off drops everything queued for it. On clears this session's
    /// faults.
    pub fn set_enabled(
        &mut self,
        plugin: &PluginId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        let was = self
            .registry
            .get(plugin)
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?
            .record()
            .enabled;
        if enabled && !was && self.registry.enabled() >= MAX_ENABLED {
            return Err(ManageError::TooManyEnabled { max: MAX_ENABLED });
        }
        self.halt(plugin);
        if let Some(installed) = self.registry.get_mut(plugin) {
            installed.record_mut().enabled = enabled;
            if enabled && let Some(ready) = installed.ready_mut() {
                ready.set_faulted(false);
            }
        }
        let action = if enabled {
            AuditAction::Enabled
        } else {
            AuditAction::Disabled
        };
        self.registry.audit(plugin, action);
        self.save(cx);
        Ok(())
    }

    /// Removes the values and storage of an installed plugin and leaves the plugin installed.
    pub fn remove_data(
        &mut self,
        plugin: &PluginId,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        let installed = self
            .registry
            .get_mut(plugin)
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?;
        self.data
            .change(plugin.clone(), PluginDataChange::Remove, cx)?;
        if let Some(ready) = installed.ready_mut() {
            ready.set_data(PluginDataDoc::default());
        }
        // An effect still queued must not write a key back after the user removed the data.
        self.broker.drop_plugin(plugin);
        self.registry.audit(plugin, AuditAction::DataRemoved);
        self.save(cx);
        Ok(())
    }

    /// Records whether the user accepted placing the plugin's contributions.
    pub fn set_placeable(
        &mut self,
        plugin: &PluginId,
        placeable: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        let installed = self
            .registry
            .get_mut(plugin)
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?;
        installed.record_mut().placeable = placeable;
        let action = if placeable {
            AuditAction::Placeable
        } else {
            AuditAction::NotPlaceable
        };
        self.registry.audit(plugin, action);
        self.save(cx);
        Ok(())
    }

    /// Stores the setting values the user edited. A file that could not be read is replaced
    /// here for the first time.
    pub fn replace_values(
        &mut self,
        plugin: &PluginId,
        values: &SettingValues,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        let ready = self
            .registry
            .get_mut(plugin)
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?
            .ready_mut()
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?;
        let next = ready.data().with_values(values)?;
        self.data
            .change(plugin.clone(), PluginDataChange::Replace(next.clone()), cx)?;
        ready.set_data(next);
        self.registry.touch();
        cx.notify();
        Ok(())
    }

    /// The view of a contribution.
    ///
    /// Over the render budget the call fails with `Throttled` and the caller keeps its last
    /// view. It may run inside any entity's update, because observation providers read no entity.
    pub fn render(
        &mut self,
        call: &Call,
        providers: &PluginProviders,
        cx: &mut Context<Self>,
    ) -> Result<View, CallError> {
        let now = cx.background_executor().now();
        self.check_running(call)?;
        self.cadence
            .admit_render(now)
            .map_err(|retry_after_ms| CallError::Throttled { retry_after_ms })?;
        let outcomes = self.broker.outcomes(&call.plugin);
        let answer = self.guest(call, providers, outcomes, now, cx, |instance, snapshot| {
            instance.render(&call.local, snapshot)
        });
        match answer {
            Ok(view) => view.into_validated().map_err(|error| {
                self.refused_reply(&call.plugin, cx);
                CallError::from(error)
            }),
            Err(error) => Err(self.failed(&call.plugin, error, cx)),
        }
    }

    /// Reports an event to a plugin and queues the effects it answers with. They are applied
    /// after the current update ends, each checked again first.
    pub fn handle(
        &mut self,
        call: &Call,
        input: Input,
        event: Event,
        providers: &PluginProviders,
        cx: &mut Context<Self>,
    ) -> Result<(), CallError> {
        let now = cx.background_executor().now();
        self.check_running(call)?;
        let token = self
            .broker
            .mint(&call.plugin, call.window, &call.local, &event, input, now);
        let outcomes = self.broker.take_outcomes(&call.plugin);
        let answer = self.guest(call, providers, outcomes, now, cx, |instance, snapshot| {
            instance.on_event(&call.local, event, snapshot)
        });
        let effects = match answer {
            Ok(effects) => effects,
            Err(error) => return Err(self.failed(&call.plugin, error, cx)),
        };
        if let Err(error) = effects.check() {
            self.refused_reply(&call.plugin, cx);
            return Err(error.into());
        }
        self.queue(call, effects, token, providers, cx);
        Ok(())
    }

    fn queue(
        &mut self,
        call: &Call,
        effects: Effects,
        token: Option<crate::gesture::TokenId>,
        providers: &PluginProviders,
        cx: &mut Context<Self>,
    ) {
        let Some(hash) = self
            .registry
            .running(&call.plugin, None)
            .map(|ready| *ready.package().hash())
        else {
            return;
        };
        for request in effects.0 {
            let origin = (call.window, &call.plugin, hash, &call.local);
            self.broker
                .enqueue(Queued::new(origin, request, token, providers));
        }
        let host = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = host.update(cx, |host, cx| host.apply_queued(cx));
        });
    }

    fn apply_queued(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.broker
            .drain(&mut self.registry, self.data.as_ref(), now, cx);
    }

    /// Runs one guest call against a fresh snapshot.
    fn guest<T>(
        &mut self,
        call: &Call,
        providers: &PluginProviders,
        outcomes: Vec<nocterm_plugin_api::EffectOutcome>,
        now: std::time::Instant,
        cx: &mut Context<Self>,
        run: impl FnOnce(
            &dyn nocterm_plugin_api::PluginInstance,
            &nocterm_plugin_api::Snapshot,
        ) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let Some(ready) = self.registry.running(&call.plugin, None) else {
            return Err(RuntimeError::Trap);
        };
        if !ready.package().manifest().has_contribution(&call.local) {
            return Err(RuntimeError::UnknownContribution(call.local.clone()));
        }
        let subject = Subject::new(
            call.window,
            &call.plugin,
            call.context,
            call.in_focus,
            providers,
        );
        let snapshot = assemble(ready, &subject, &mut self.cadence, outcomes, now, cx);
        run(ready.instance(self.runtime.as_ref())?, &snapshot)
    }

    /// Counts a runtime failure when it is the plugin's fault.
    fn failed(
        &mut self,
        plugin: &PluginId,
        error: RuntimeError,
        cx: &mut Context<Self>,
    ) -> CallError {
        if Faults::counts(&error) {
            self.fault(plugin, cx);
        }
        match error {
            RuntimeError::UnknownContribution(local) => {
                CallError::NoContribution(local.to_string())
            }
            other => CallError::Runtime(other),
        }
    }

    /// A reply the host refuses to draw or apply (too many nodes, too many effects, a key used
    /// twice) is the fault of a plugin that runs code. A declarative plugin's views come from the
    /// host's own interpreter, so a refusal there is not counted.
    fn refused_reply(&mut self, plugin: &PluginId, cx: &mut Context<Self>) {
        let runs_code = self
            .registry
            .running(plugin, None)
            .is_some_and(|ready| matches!(ready.package().manifest().runtime(), Runtime::Wasm));
        if runs_code {
            self.fault(plugin, cx);
        }
    }

    /// Counts one fault, and takes the plugin out of service at the third.
    fn fault(&mut self, plugin: &PluginId, cx: &mut Context<Self>) {
        if self.faults.record(plugin) {
            self.halt(plugin);
            if let Some(ready) = self.registry.get_mut(plugin).and_then(Installed::ready_mut) {
                ready.set_faulted(true);
            }
            self.registry.audit(plugin, AuditAction::Faulted);
            self.save(cx);
        }
    }

    fn check_running(&mut self, call: &Call) -> Result<(), CallError> {
        if self.registry.get(&call.plugin).is_none() {
            return Err(CallError::Unknown(call.plugin.clone()));
        }
        let Some(ready) = self.registry.running(&call.plugin, None) else {
            return Err(CallError::NotRunning(call.plugin.clone()));
        };
        if !ready.package().manifest().has_contribution(&call.local) {
            return Err(CallError::NoContribution(call.local.to_string()));
        }
        Ok(())
    }

    /// Asks the port to delete the plugin's data. Part of an update or an uninstall that has
    /// already happened, so a refusal is logged and the old file stays for the next attempt.
    fn drop_data(&self, plugin: &PluginId, cx: &mut Context<Self>) {
        if let Err(error) = self
            .data
            .change(plugin.clone(), PluginDataChange::Remove, cx)
        {
            tracing::warn!(plugin = %plugin.as_str(), %error, "plugin data was not removed");
        }
    }

    /// Everything the host holds for a plugin that must not outlive a state change.
    fn halt(&mut self, plugin: &PluginId) {
        self.broker.drop_plugin(plugin);
        self.cadence.forget(plugin);
        self.faults.clear(plugin);
        if let Some(ready) = self.registry.get_mut(plugin).and_then(Installed::ready_mut) {
            ready.stop();
        }
    }

    fn load_status(&self, record: &PluginRecord, cx: &gpui::App) -> Status {
        match self.load_package(record) {
            Ok((package, grant)) => {
                let data = self.read_data(&record.id, cx);
                Status::Ready(Box::new(Ready::new(package, grant, data)))
            }
            Err(error) => Status::Broken(error),
        }
    }

    fn load_package(&self, record: &PluginRecord) -> Result<(Package, Grant), BrokenPackage> {
        let hash = ContentHash::parse(&record.hash)?;
        let package = self.packages.load(&record.id, &hash)?;
        if package.manifest().plugin().id() != &record.id {
            return Err(BrokenPackage::OtherPlugin {
                found: package.manifest().plugin().id().clone(),
            });
        }
        let accepted: Vec<&str> = record.granted_optional.iter().map(String::as_str).collect();
        let grant = Grant::consent(&package, &accepted)?;
        Ok((package, grant))
    }

    fn read_data(&self, plugin: &PluginId, cx: &gpui::App) -> DataState {
        let broken = |error: PluginDataError| DataState::Broken {
            error,
            defaults: PluginDataDoc::default(),
        };
        match self.data.load(plugin, cx) {
            Ok(doc) => match doc.check() {
                Ok(()) => DataState::Loaded(doc),
                Err(error) => broken(PluginDataError::Bounds(error)),
            },
            Err(error) => broken(error),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.registry.touch();
        cx.notify();
        let task = self.records.replace(self.registry.to_doc(), cx);
        cx.spawn(async move |_, _| {
            if let Err(error) = task.await {
                tracing::warn!(%error, "plugin records were not saved");
            }
        })
        .detach();
    }
}

fn record_of(package: &Package, accepted: &[&str], enabled: bool) -> PluginRecord {
    PluginRecord {
        id: package.manifest().plugin().id().clone(),
        hash: package.hash().to_string(),
        enabled,
        placeable: false,
        granted_optional: accepted.iter().map(|id| (*id).to_owned()).collect(),
    }
}
