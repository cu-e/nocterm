//! The User section's port over the host: rows, the audit log and the install, update and
//! removal acts, with the consent facts worded by the host.
use gpui_kit::{App, Entity, Subscription};
use nocterm_core::error::SharedCause;
use nocterm_plugin_api::{Additions, Manifest, Package, Runtime, capability};
use nocterm_plugin_host::{AuditAction, ManageError, PluginHost, PluginStatus, PluginSummary};
use nocterm_plugin_model::PluginId;
use nocterm_plugins_ui::{
    AuditLine, Candidate, CapabilityLine, FieldLine, InstallRequest, Replaced, RowState,
    UserPluginError, UserPluginRow, UserPlugins,
};

/// The user plugins of the installation, over the host entity.
pub(super) struct HostPlugins {
    host: Entity<PluginHost>,
}

impl HostPlugins {
    pub(super) fn new(host: Entity<PluginHost>) -> Self {
        Self { host }
    }

    fn act(
        &self,
        cx: &mut App,
        act: impl FnOnce(&mut PluginHost, &mut gpui_kit::Context<PluginHost>) -> Result<(), ManageError>,
    ) -> Result<(), UserPluginError> {
        self.host
            .update(cx, act)
            .map_err(|error| UserPluginError::Manage(SharedCause::new(error)))
    }
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

fn candidate(host: &PluginHost, location: &str, package: &Package) -> Candidate {
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
    }
}

fn runs_code(manifest: &Manifest) -> bool {
    manifest.runtime() == Runtime::Wasm
}

impl UserPlugins for HostPlugins {
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

    fn inspect(&self, location: &str, cx: &App) -> Result<Candidate, UserPluginError> {
        let host = self.host.read(cx);
        let package = host
            .read_package(location)
            .map_err(|error| UserPluginError::Package(SharedCause::new(error)))?;
        Ok(candidate(host, location, &package))
    }

    fn install(&self, request: InstallRequest, cx: &mut App) -> Result<(), UserPluginError> {
        let host = self.host.read(cx);
        let package = host
            .read_package(&request.location)
            .map_err(|error| UserPluginError::Package(SharedCause::new(error)))?;
        if package.hash().to_string() != request.hash {
            return Err(UserPluginError::Changed);
        }
        let id = package.manifest().plugin().id().clone();
        let installed = host
            .plugins()
            .into_iter()
            .find(|summary| summary.id == id)
            .map(|summary| summary.hash);
        if installed != request.replaces {
            return Err(UserPluginError::InstalledChanged);
        }
        let accepted: Vec<&str> = request.accepted.iter().map(String::as_str).collect();
        self.act(cx, |host, cx| {
            if installed.is_some() {
                host.replace(package, &accepted, request.keep_data, cx)
            } else {
                host.install(package, &accepted, cx).map(drop)
            }
        })
    }

    fn set_enabled(
        &self,
        plugin: &PluginId,
        enabled: bool,
        cx: &mut App,
    ) -> Result<(), UserPluginError> {
        self.act(cx, |host, cx| host.set_enabled(plugin, enabled, cx))
    }

    fn set_placeable(
        &self,
        plugin: &PluginId,
        placeable: bool,
        cx: &mut App,
    ) -> Result<(), UserPluginError> {
        self.act(cx, |host, cx| host.set_placeable(plugin, placeable, cx))
    }

    fn remove(
        &self,
        plugin: &PluginId,
        with_data: bool,
        cx: &mut App,
    ) -> Result<(), UserPluginError> {
        self.act(cx, |host, cx| host.uninstall(plugin, with_data, cx))
    }

    fn remove_data(&self, plugin: &PluginId, cx: &mut App) -> Result<(), UserPluginError> {
        self.act(cx, |host, cx| host.remove_data(plugin, cx))
    }

    fn observe(&self, cx: &mut App, mut on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        cx.observe(&self.host, move |_, cx| on_change(cx))
    }
}

#[cfg(test)]
mod tests;
