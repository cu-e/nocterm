//! The User section's port over the host: rows, the audit log and the install, update and
//! removal acts, with the consent facts worded by the host.
use futures::{channel::oneshot, lock::Mutex};
use std::{cell::Cell, future::Future, path::PathBuf, rc::Rc, sync::Arc};

use gpui_kit::{App, AsyncApp, Entity, Subscription, Task};
use nocterm_core::error::SharedCause;
use nocterm_plugin_api::{
    Additions, ContentHash, Manifest, Package, PluginPackages, PluginRuntime, Runtime, capability,
};
use nocterm_plugin_host::{
    AuditAction, ManageError, PluginData, PluginDataChange, PluginHost, PluginStatus,
    PluginSummary, PreparedPlugin,
};
use nocterm_plugin_model::PluginId;
use nocterm_plugins_ui::{
    AuditLine, Candidate, CapabilityLine, FieldLine, InstallRequest, Replaced, RowState,
    UserPluginError, UserPluginRow, UserPlugins,
};

/// One retained manager serializes every native package transaction across windows.
#[derive(Clone)]
pub(super) struct HostPlugins {
    host: Entity<PluginHost>,
    dev: Option<PathBuf>,
    packages: Arc<dyn PluginPackages>,
    runtime: Arc<dyn PluginRuntime>,
    data: Rc<dyn PluginData>,
    native_data: Option<super::PluginDataFiles>,
    ready: super::restore::Ready,
    serial: Rc<Mutex<()>>,
    pending: Rc<Cell<usize>>,
}
impl HostPlugins {
    pub(super) fn new(
        host: Entity<PluginHost>,
        dev: Option<PathBuf>,
        packages: Arc<dyn PluginPackages>,
        runtime: Arc<dyn PluginRuntime>,
        data: Rc<dyn PluginData>,
        native_data: Option<super::PluginDataFiles>,
        ready: super::restore::Ready,
    ) -> Self {
        Self {
            host,
            dev,
            packages,
            runtime,
            data,
            native_data,
            ready,
            serial: Rc::new(Mutex::new(())),
            pending: Rc::new(Cell::new(0)),
        }
    }
    fn operation<R: 'static, F, Fut>(
        &self,
        cx: &mut App,
        action: F,
    ) -> Task<Result<R, UserPluginError>>
    where
        F: FnOnce(Self, AsyncApp) -> Fut + 'static,
        Fut: Future<Output = Result<R, UserPluginError>> + 'static,
    {
        if self.pending.get() >= 8 {
            return Task::ready(Err(manage(nocterm_store::StoreError::Full)));
        }
        self.pending.set(self.pending.get() + 1);
        let permit = Permit(self.pending.clone());
        let this = self.clone();
        let (sender, receiver) = oneshot::channel();
        // Accepted work keeps its original owner and completes even when its UI closes.
        cx.spawn(async move |cx| {
            let _permit = permit;
            let result = async {
                this.ready.wait().await?;
                let serial = this.serial.clone();
                let _guard = serial.lock().await;
                action(this, cx.clone()).await
            }
            .await;
            let _ = sender.send(result);
        })
        .detach();
        cx.spawn(async move |_| {
            receiver
                .await
                .unwrap_or_else(|_| Err(manage(nocterm_store::StoreError::CompletionDropped)))
        })
    }
    fn change<F>(
        &self,
        plugin: PluginId,
        cx: &mut App,
        action: F,
    ) -> Task<Result<(), UserPluginError>>
    where
        F: FnOnce(&mut PluginHost, &mut gpui_kit::Context<PluginHost>) -> Result<(), ManageError>
            + 'static,
    {
        self.operation(cx, move |this, cx| async move {
            let publication = cx.update(|cx| {
                this.host.update(cx, |host, cx| {
                    action(host, cx).map_err(manage)?;
                    Ok::<_, UserPluginError>(host.take_publication())
                })
            })?;
            let records = publication.await;
            let values = cx.update(|cx| this.data.flush(&plugin, cx)).await;
            records.map_err(manage)?;
            values.map_err(manage)
        })
    }
}
struct Permit(Rc<Cell<usize>>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
fn manage(error: impl std::error::Error + Send + Sync + 'static) -> UserPluginError {
    UserPluginError::Manage(SharedCause::new(error))
}
fn package_error(error: nocterm_plugin_api::PackageError) -> UserPluginError {
    UserPluginError::Package(SharedCause::new(error))
}

/// The publisher part of `user-<publisher>-<name>`, for a plugin whose package cannot be read.
pub(super) fn publisher_of(id: &PluginId) -> String {
    id.as_str()
        .strip_prefix("user-")
        .and_then(|rest| rest.split_once('-'))
        .map_or_else(String::new, |(publisher, _)| publisher.to_owned())
}

pub(super) fn fingerprint(hash: &str) -> String {
    hash.chars().take(16).collect()
}

fn version_of(manifest: &Manifest) -> String {
    format!("API {}", manifest.api())
}

pub(super) fn host_words(capability: &str) -> String {
    capability::capability(capability)
        .map_or_else(|| capability.to_owned(), |entry| entry.consent.to_owned())
}

fn row(host: &PluginHost, summary: PluginSummary) -> UserPluginRow {
    let manifest = host.manifest(&summary.id);
    UserPluginRow {
        publisher: manifest.map_or_else(|| publisher_of(&summary.id), |m| m.publisher().to_owned()),
        version: manifest.map_or_else(String::new, version_of),
        fingerprint: fingerprint(&summary.hash),
        state: match summary.status {
            PluginStatus::Faulted => RowState::Faulted,
            PluginStatus::Broken(reason) => RowState::Broken(SharedCause::new(reason)),
            PluginStatus::Running | PluginStatus::Disabled => RowState::Ready,
        },
        granted: host
            .granted(&summary.id)
            .iter()
            .map(|id| host_words(id))
            .collect(),
        contributions: manifest.map_or_else(Vec::new, |manifest| {
            manifest
                .plugin()
                .contributions()
                .iter()
                .map(|contribution| contribution.title().to_owned())
                .collect()
        }),
        id: summary.id,
        name: summary.name,
        enabled: summary.enabled,
        placeable: summary.placeable,
    }
}

fn action_text(action: AuditAction) -> &'static str {
    match action {
        AuditAction::Installed => "Installed",
        AuditAction::Replaced => "Replaced by another package",
        AuditAction::Removed => "Removed",
        AuditAction::Enabled => "Turned on",
        AuditAction::Disabled => "Turned off",
        AuditAction::Placeable => "Allowed in windows",
        AuditAction::NotPlaceable => "Revoked from windows",
        AuditAction::DataRemoved => "Data removed",
        AuditAction::Faulted => "Stopped after repeated failures",
    }
}

/// The capabilities a package asks for, as the consent lists them. Optional capabilities this
/// version does not know are not offered: nothing could ever be granted for them.
fn capability_lines(next: &Manifest, added: &[String]) -> Vec<CapabilityLine> {
    let line = |id: &str, required: bool| CapabilityLine {
        id: id.to_owned(),
        description: host_words(id),
        reason: next.reason(id).map(str::to_owned),
        required,
        added: added.iter().any(|added| added == id),
    };
    let required = next.required().iter().map(|entry| line(entry.id, true));
    let optional = next
        .optional()
        .iter()
        .filter_map(|optional| match optional {
            nocterm_plugin_api::OptionalCapability::Known(entry) => Some(line(entry.id, false)),
            nocterm_plugin_api::OptionalCapability::Unknown(_) => None,
        });
    required.chain(optional).collect()
}

fn field_lines(next: &Manifest, added: &[String]) -> Vec<FieldLine> {
    next.plugin()
        .settings()
        .fields()
        .map(|field| FieldLine {
            label: field.label.to_string(),
            added: added.iter().any(|key| key.as_str() == field.key),
        })
        .collect()
}

fn candidate(host: &PluginHost, location: &str, package: &Package, dev: bool) -> Candidate {
    let next = package.manifest();
    let id = next.plugin().id().clone();
    let installed = host.plugins().into_iter().find(|summary| summary.id == id);
    let previous = host.manifest(&id);
    let additions = previous.map(|previous| Additions::between(previous, next));
    let (capabilities, settings) = additions
        .map(|additions| (additions.capabilities, additions.settings))
        .unwrap_or_default();
    let hash = package.hash().to_string();
    Candidate {
        location: location.to_owned(),
        name: next.plugin().name().to_owned(),
        publisher: next.publisher().to_owned(),
        version: version_of(next),
        fingerprint: package.hash().fingerprint(),
        capabilities: capability_lines(next, &capabilities),
        fields: field_lines(next, &settings),
        replaces: installed.map(|summary| Replaced {
            version: previous.map_or_else(String::new, version_of),
            fingerprint: fingerprint(&summary.hash),
            hash: summary.hash,
            downgrade: previous.is_some_and(|previous| next.api() < previous.api()),
            runs_code: previous.is_some_and(runs_code),
        }),
        hash,
        id,
        runs_code: runs_code(next),
        dev,
    }
}

fn runs_code(manifest: &Manifest) -> bool {
    manifest.runtime() == Runtime::Wasm
}

impl HostPlugins {
    async fn clear_data(&self, plugin: &PluginId, cx: &AsyncApp) -> Result<(), UserPluginError> {
        cx.update(|cx| {
            self.host
                .update(cx, |host, cx| host.hold_data_change(plugin, true, cx))
        })
        .map_err(manage)?;
        let result = async {
            cx.update(|cx| {
                self.data
                    .change(plugin.clone(), PluginDataChange::Remove, cx)
            })
            .map_err(manage)?;
            cx.update(|cx| self.data.flush(plugin, cx))
                .await
                .map_err(manage)?;
            let publication = cx.update(|cx| {
                self.host.update(cx, |host, cx| {
                    host.remove_data_published(plugin, cx).map_err(manage)?;
                    Ok::<_, UserPluginError>(host.take_publication())
                })
            })?;
            publication.await.map_err(manage)
        }
        .await;
        let _ = cx.update(|cx| {
            self.host
                .update(cx, |host, cx| host.hold_data_change(plugin, false, cx))
        });
        result
    }
    async fn install_request(
        self,
        request: InstallRequest,
        cx: AsyncApp,
    ) -> Result<(), UserPluginError> {
        let packages = self.packages.clone();
        let location = request.location.clone();
        let package = cx
            .background_executor()
            .spawn(async move { packages.read(&location) })
            .await
            .map_err(package_error)?;
        if package.hash().to_string() != request.hash {
            return Err(UserPluginError::Changed);
        }
        let plugin = package.manifest().plugin().id().clone();
        let active = cx.update(|cx| {
            let host = self.host.read(cx);
            if host.hash_of(&plugin) != request.replaces {
                return Err(UserPluginError::InstalledChanged);
            }
            let accepted: Vec<_> = request.accepted.iter().map(String::as_str).collect();
            host.validate_change(&package, &accepted, request.replaces.as_deref())
                .map_err(manage)?;
            Ok(request.replaces.is_none()
                || host
                    .plugins()
                    .iter()
                    .any(|row| row.id == plugin && row.enabled))
        })?;
        let packages = self.packages.clone();
        let runtime = self.runtime.clone();
        let expected = *package.hash();
        let prepared = cx
            .background_executor()
            .spawn(async move {
                let instance = active
                    .then(|| runtime.instantiate(&package))
                    .transpose()
                    .map_err(manage)?;
                packages.publish(&package, true).map_err(package_error)?;
                Ok::<_, UserPluginError>(PreparedPlugin::new(package, instance))
            })
            .await?;
        if request.replaces.is_none()
            && let Some(files) = &self.native_data
        {
            cx.update(|cx| files.acquire(plugin.clone(), cx)).await;
        }
        if request.replaces.is_some() && !request.keep_data {
            self.clear_data(&plugin, &cx).await?;
        }
        let publication = cx.update(|cx| {
            self.host.update(cx, |host, cx| {
                if host.hash_of(&plugin) != request.replaces {
                    return Err(UserPluginError::InstalledChanged);
                }
                let accepted: Vec<_> = request.accepted.iter().map(String::as_str).collect();
                host.validate_change(prepared.package(), &accepted, request.replaces.as_deref())
                    .map_err(manage)?;
                if request.replaces.is_some() {
                    host.replace_published(prepared, &accepted, true, cx)
                        .map_err(manage)?;
                } else {
                    host.install_published(prepared, &accepted, cx)
                        .map_err(manage)?;
                }
                Ok(host.take_publication())
            })
        })?;
        publication.await.map_err(manage)?;
        cx.update(|cx| self.data.flush(&plugin, cx))
            .await
            .map_err(manage)?;
        let packages = self.packages.clone();
        cx.background_executor()
            .spawn(async move { packages.retain(&plugin, &expected) })
            .await
            .map_err(package_error)
    }
}

impl UserPlugins for HostPlugins {
    fn error(&self, cx: &App) -> Option<UserPluginError> {
        self.ready
            .error()
            .or_else(|| self.host.read(cx).publication_error().cloned().map(manage))
    }
    fn rows(&self, cx: &App) -> Vec<UserPluginRow> {
        let host = self.host.read(cx);
        host.plugins()
            .into_iter()
            .map(|summary| row(host, summary))
            .collect()
    }

    fn audit(&self, cx: &App) -> Vec<AuditLine> {
        self.host
            .read(cx)
            .audit()
            .iter()
            .map(|entry| AuditLine {
                plugin: entry.plugin.clone(),
                action: action_text(entry.action).to_owned(),
            })
            .collect()
    }

    fn dev_location(&self, _cx: &App) -> Option<String> {
        self.dev
            .as_deref()
            .and_then(std::path::Path::to_str)
            .map(str::to_owned)
    }

    fn inspect(&self, location: &str, cx: &mut App) -> Task<Result<Candidate, UserPluginError>> {
        let location = location.to_owned();
        self.operation(cx, move |this, cx| async move {
            let packages = this.packages.clone();
            let requested = location.clone();
            let package = cx
                .background_executor()
                .spawn(async move { packages.read(&requested) })
                .await
                .map_err(package_error)?;
            Ok(cx.update(|cx| {
                candidate(this.host.read(cx), &location, &package, this.dev.is_some())
            }))
        })
    }

    fn install(&self, request: InstallRequest, cx: &mut App) -> Task<Result<(), UserPluginError>> {
        self.operation(cx, move |this, cx| async move {
            this.install_request(request, cx).await
        })
    }
    fn set_enabled(
        &self,
        plugin: &PluginId,
        enabled: bool,
        cx: &mut App,
    ) -> Task<Result<(), UserPluginError>> {
        let plugin = plugin.clone();
        self.operation(cx, move |this, cx| async move {
            let hash = cx
                .update(|cx| this.host.read(cx).hash_of(&plugin))
                .ok_or_else(|| manage(ManageError::Unknown(plugin.clone())))?;
            let instance = if enabled {
                let packages = this.packages.clone();
                let runtime = this.runtime.clone();
                let id = plugin.clone();
                let expected = hash.clone();
                Some(
                    cx.background_executor()
                        .spawn(async move {
                            let expected = ContentHash::parse(&expected).map_err(manage)?;
                            let package = packages.load(&id, &expected).map_err(package_error)?;
                            runtime.instantiate(&package).map_err(manage)
                        })
                        .await?,
                )
            } else {
                None
            };
            let publication = cx.update(|cx| {
                if this.host.read(cx).hash_of(&plugin).as_deref() != Some(hash.as_str()) {
                    return Err(UserPluginError::InstalledChanged);
                }
                this.host.update(cx, |host, cx| {
                    host.set_enabled_prepared(&plugin, enabled, instance, cx)
                        .map_err(manage)?;
                    Ok(host.take_publication())
                })
            })?;
            publication.await.map_err(manage)
        })
    }
    fn set_placeable(
        &self,
        plugin: &PluginId,
        placeable: bool,
        cx: &mut App,
    ) -> Task<Result<(), UserPluginError>> {
        let id = plugin.clone();
        self.change(plugin.clone(), cx, move |host, cx| {
            host.set_placeable(&id, placeable, cx)
        })
    }
    fn remove(
        &self,
        plugin: &PluginId,
        with_data: bool,
        cx: &mut App,
    ) -> Task<Result<(), UserPluginError>> {
        let plugin = plugin.clone();
        self.operation(cx, move |this, cx| async move {
            if with_data {
                this.clear_data(&plugin, &cx).await?;
            } else {
                if let Some(files) = &this.native_data {
                    cx.update(|cx| files.retry(&plugin, cx)).map_err(manage)?;
                }
                cx.update(|cx| this.data.flush(&plugin, cx))
                    .await
                    .map_err(manage)?;
            }
            let publication = cx.update(|cx| {
                this.host.update(cx, |host, cx| {
                    host.uninstall_record(&plugin, false, cx).map_err(manage)?;
                    Ok::<_, UserPluginError>(host.take_publication())
                })
            })?;
            // Data is already settled and this host no longer accepts its effects.
            // A records failure keeps native bytes for recovery, without retaining its cache.
            if let Some(files) = &this.native_data {
                cx.update(|cx| files.forget(&plugin, cx));
            }
            publication.await.map_err(manage)?;
            let packages = this.packages.clone();
            cx.background_executor()
                .spawn(async move { packages.remove(&plugin) })
                .await
                .map_err(package_error)
        })
    }
    fn remove_data(&self, plugin: &PluginId, cx: &mut App) -> Task<Result<(), UserPluginError>> {
        let plugin = plugin.clone();
        self.operation(cx, move |this, cx| async move {
            this.clear_data(&plugin, &cx).await
        })
    }

    fn observe(&self, cx: &mut App, mut on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        cx.observe(&self.host, move |_, cx| on_change(cx))
    }
}

#[cfg(test)]
mod tests;
