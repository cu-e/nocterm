//! The host: installed plugins, their calls and the effects they ask for.

use std::{rc::Rc, sync::Arc};

use gpui::{Context, Task, WindowId};
use nocterm_plugin_api::{
    ContentHash, Effects, Event, Grant, Manifest, Package, PackageError, PluginInstance,
    PluginPackages, PluginRuntime, Runtime, RuntimeError, View,
};
use nocterm_plugin_model::{PluginId, SettingValues, UserEntry};
use nocterm_plugin_ports::PluginProviders;

use crate::{
    AuditEntry, BrokenPackage, Call, CallError, HostPorts, ManageError, PluginData,
    PluginDataChange, PluginDataDoc, PluginRecord, PluginRecords, PluginSummary, PreparedPlugin,
    RecordsError,
    broker::{Broker, Checked, Queued},
    cadence::Cadence,
    caps::MAX_ENABLED,
    deferred::Deferred,
    faults::Faults,
    gesture::Input,
    records::AuditAction,
    registry::{DataState, Installed, Ready, Registry, Status},
    snapshot::{Subject, assemble},
};

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
    deferred: Deferred,
    publication: Option<Task<Result<(), RecordsError>>>,
    publication_error: Option<RecordsError>,
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
            deferred: Deferred::new(ports.net),
            publication: None,
            publication_error: None,
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

    /// Installs the captured startup results without native reads or runtime compilation.
    pub fn restore_prepared(
        &mut self,
        plugins: Vec<crate::RestoredPlugin>,
        audit: Vec<AuditEntry>,
        cx: &mut Context<Self>,
    ) {
        let mut registry = Registry::with_audit(audit);
        let mut trimmed = false;
        for mut plugin in plugins.into_iter().take(crate::MAX_RECORDS) {
            if registry.enabled() >= MAX_ENABLED
                && let Some(id) = plugin.restrict_enabled()
            {
                registry.audit(&id, AuditAction::Disabled);
                trimmed = true;
            }
            registry.insert(plugin.into_installed());
        }
        registry.touch();
        self.registry = registry;
        if trimmed {
            self.save(cx);
        }
        cx.notify();
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
        self.validate_change(&package, accepted, None)?;
        self.packages.publish(&package, false)?;
        self.install_published(PreparedPlugin::new(package, None), accepted, cx)
    }

    /// Commits already-published bytes after checking current admission again. Performs no package I/O.
    pub fn install_published(
        &mut self,
        prepared: PreparedPlugin,
        accepted: &[&str],
        cx: &mut Context<Self>,
    ) -> Result<PluginId, ManageError> {
        let (package, instance) = prepared.into_parts();
        self.validate_change(&package, accepted, None)?;
        let id = package.manifest().plugin().id().clone();
        let grant = Grant::consent(&package, accepted)?;
        let record = PluginRecord::of(&package, accepted, true);
        let data = self.read_data(&id, cx);
        let status = Status::Ready(Box::new(
            Ready::new(package, grant, data).with_instance(instance),
        ));
        self.registry.insert(Installed::new(record, status));
        self.registry.audit(&id, AuditAction::Installed);
        self.save(cx);
        Ok(id)
    }

    /// Validates the captured replacement identity, consent and capacity before native work.
    pub fn validate_change(
        &self,
        package: &Package,
        accepted: &[&str],
        expected: Option<&str>,
    ) -> Result<(), ManageError> {
        let id = package.manifest().plugin().id().clone();
        if let Some(expected) = expected {
            let installed = self
                .registry
                .get(&id)
                .ok_or_else(|| ManageError::Unknown(id))?;
            if installed.record().hash != expected {
                return Err(ManageError::Changed);
            }
            Grant::consent(package, accepted)?;
            return Ok(());
        }
        if self.registry.get(&id).is_some() {
            return Err(ManageError::Installed(id));
        }
        if self.registry.len() >= crate::records::MAX_RECORDS {
            return Err(ManageError::TooManyInstalled {
                max: crate::records::MAX_RECORDS,
            });
        }
        if self.registry.enabled() >= MAX_ENABLED {
            return Err(ManageError::TooManyEnabled { max: MAX_ENABLED });
        }
        Grant::consent(package, accepted)?;
        Ok(())
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
        if self.registry.get(&id).is_none() {
            return Err(ManageError::Unknown(id));
        }
        Grant::consent(&package, accepted)?;
        self.packages.publish(&package, true)?;
        self.replace_published(PreparedPlugin::new(package, None), accepted, keep_data, cx)
    }

    /// Commits a replacement whose immutable bytes and instance were prepared in the background.
    pub fn replace_published(
        &mut self,
        prepared: PreparedPlugin,
        accepted: &[&str],
        keep_data: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        let (package, instance) = prepared.into_parts();
        let id = package.manifest().plugin().id().clone();
        let Some(installed) = self.registry.get(&id) else {
            return Err(ManageError::Unknown(id));
        };
        let enabled = installed.record().enabled;
        let grant = Grant::consent(&package, accepted)?;
        if !keep_data {
            self.drop_data(&id, cx)?;
        }
        self.halt(&id);
        let data = if keep_data {
            let kept = self
                .registry
                .get_mut(&id)
                .and_then(Installed::ready_mut)
                .map(Ready::take_data);
            kept.unwrap_or_else(|| self.read_data(&id, cx))
        } else {
            DataState::Loaded(PluginDataDoc::default())
        };
        let mut record = PluginRecord::of(&package, accepted, enabled);
        record.placeable = false;
        let status = Status::Ready(Box::new(
            Ready::new(package, grant, data).with_instance(if enabled { instance } else { None }),
        ));
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
        if remove_data {
            self.drop_data(plugin, cx)?;
        }
        self.packages.remove(plugin)?;
        self.uninstall_record(plugin, false, cx)
    }

    /// Removes the record first; the caller removes native bytes only after its receipt succeeds.
    pub fn uninstall_record(
        &mut self,
        plugin: &PluginId,
        remove_data: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        if self.registry.get(plugin).is_none() {
            return Err(ManageError::Unknown(plugin.clone()));
        }
        if remove_data {
            self.drop_data(plugin, cx)?;
        }
        self.halt(plugin);
        self.broker.forget(plugin);
        self.registry.remove(plugin);
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
        self.set_enabled_prepared(plugin, enabled, None, cx)
    }

    /// Enables a prepared runtime without compilation or native package loading on this thread.
    pub fn set_enabled_prepared(
        &mut self,
        plugin: &PluginId,
        enabled: bool,
        instance: Option<Box<dyn PluginInstance>>,
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
                if let Some(instance) = instance {
                    ready.start_prepared(instance);
                }
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
        if self.registry.get(plugin).is_none() {
            return Err(ManageError::Unknown(plugin.clone()));
        }
        self.data
            .change(plugin.clone(), PluginDataChange::Remove, cx)?;
        self.remove_data_published(plugin, cx)
    }
    /// Pauses events/effects during an accepted native data mutation without dropping its instance.
    pub fn hold_data_change(
        &mut self,
        plugin: &PluginId,
        held: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        self.registry.hold_data(plugin, held)?;
        if held {
            self.broker.drop_plugin(plugin);
        }
        cx.notify();
        Ok(())
    }
    /// Commits a physically published reset; no second native deletion is queued.
    pub fn remove_data_published(
        &mut self,
        plugin: &PluginId,
        cx: &mut Context<Self>,
    ) -> Result<(), ManageError> {
        self.registry.clear_data(plugin)?;
        self.broker.drop_plugin(plugin);
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
        if self.registry.data_held(plugin) {
            return Err(ManageError::Changed);
        }
        let ready = self
            .registry
            .get_mut(plugin)
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?
            .ready_mut()
            .ok_or_else(|| ManageError::Unknown(plugin.clone()))?;
        let next = ready.data().with_values(&ready.bound(values))?;
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
            .admit_contribution(call, now)
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
            self.broker
                .enqueue(Queued::new(call, hash, request, token, providers));
        }
        let host = cx.weak_entity();
        cx.defer(move |cx| Self::apply_queued(&host, cx));
    }

    /// Checks the queue inside an update of the host, then has the windows' providers apply what
    /// it approved after that update: none of them runs while the host is held.
    fn apply_queued(host: &gpui::WeakEntity<Self>, cx: &mut gpui::App) {
        let Ok(checked) = host.update(cx, |host, cx| {
            let now = cx.background_executor().now();
            let data = host.data.as_ref();
            let checked =
                (host.broker).drain(&mut host.registry, &mut host.deferred, data, now, cx);
            host.deferred.start(cx);
            checked
        }) else {
            return;
        };
        let mut outcomes = Vec::with_capacity(checked.len());
        for item in checked {
            outcomes.push(match item {
                Checked::Done(plugin, key, result) => (plugin, key, result),
                Checked::Approved(item) => {
                    // An earlier effect of the batch may have taken the plugin out of service.
                    let runs = host.update(cx, |host, _| {
                        host.registry
                            .running(item.plugin(), Some(item.hash()))
                            .is_some()
                    });
                    if !matches!(runs, Ok(true)) {
                        continue;
                    }
                    let plugin = item.plugin().clone();
                    let (key, result) = item.apply(cx);
                    (plugin, key, result)
                }
            });
        }
        let _ = host.update(cx, |host, _| {
            for (plugin, key, result) in outcomes {
                host.broker.record(&plugin, key, result);
            }
        });
    }

    /// Reports an event the host raised (a response, a timer, a terminal line) while the plugin
    /// runs `hash` (any package when `None`), and draws it again.
    pub fn deliver(
        &mut self,
        call: &Call,
        hash: Option<&ContentHash>,
        event: Event,
        providers: &PluginProviders,
        cx: &mut Context<Self>,
    ) -> Result<(), CallError> {
        if self.registry.running(&call.plugin, hash).is_none() {
            return Err(CallError::NotRunning(call.plugin.clone()));
        }
        let handled = self.handle(call, Input::Host, event, providers, cx);
        self.registry.touch();
        cx.notify();
        handled
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
        )
        .for_terminal(call.terminal.as_deref());
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

    fn drop_data(&self, plugin: &PluginId, cx: &mut Context<Self>) -> Result<(), ManageError> {
        self.data
            .change(plugin.clone(), PluginDataChange::Remove, cx)?;
        Ok(())
    }

    /// Releases all observations and deferred actions of a window that has closed.
    pub fn release_window(&mut self, window: WindowId) {
        self.cadence.release_window(window);
        self.broker.release_window(window);
        self.deferred.release_window(window);
    }

    /// Everything the host holds for a plugin that must not outlive a state change.
    fn halt(&mut self, plugin: &PluginId) {
        self.broker.drop_plugin(plugin);
        self.deferred.forget(plugin);
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
        crate::data::load_state(self.data.as_ref(), plugin, cx)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.registry.touch();
        cx.notify();
        let task = self.records.replace(self.registry.to_doc(), cx);
        let revision = self.registry.revision();
        self.publication_error = None;
        self.publication = Some(cx.spawn(async move |host, cx| {
            let result = task.await;
            let _ = host.update(cx, |host, cx| {
                if host.registry.revision() == revision {
                    host.publication_error = result.as_ref().err().cloned();
                    cx.notify();
                }
            });
            result
        }));
    }

    /// The last accepted records write. Dropping a receipt never revokes accepted persistence work.
    pub fn take_publication(&mut self) -> Task<Result<(), RecordsError>> {
        self.publication
            .take()
            .unwrap_or_else(|| Task::ready(Ok(())))
    }

    pub fn publication_error(&self) -> Option<&RecordsError> {
        self.publication_error.as_ref()
    }
}
